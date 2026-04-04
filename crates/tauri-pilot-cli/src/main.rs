mod cli;
mod client;
mod output;
mod protocol;
mod style;

use anyhow::{Context, Result};
use base64::Engine;
use clap::Parser;
use serde_json::{Value, json};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

use cli::{AssertKind, Cli, Command, Target, parse_target};
use client::Client;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = Cli::parse();
    let socket = resolve_socket(args.socket)?;
    let mut client = Client::connect(&socket).await?;

    let is_snapshot = matches!(args.command, Command::Snapshot { .. });
    let is_diff = matches!(args.command, Command::Diff { .. });
    let is_logs = matches!(args.command, Command::Logs { .. });
    let is_network = matches!(args.command, Command::Network { .. });
    let is_watch = matches!(args.command, Command::Watch { .. });

    // Handle --follow mode: loop forever polling for new entries
    if let Command::Logs {
        follow: true,
        ref level,
        ref last,
        ..
    } = args.command
    {
        follow_logs(&mut client, args.json, level.as_deref(), *last).await?;
    } else if let Command::Network {
        follow: true,
        ref filter,
        ref last,
        failed,
        ..
    } = args.command
    {
        follow_network(&mut client, args.json, filter.as_deref(), *last, failed).await?;
    }

    let screenshot_path = if let Command::Screenshot { ref path, .. } = args.command {
        path.clone()
    } else {
        None
    };
    let is_screenshot = matches!(args.command, Command::Screenshot { .. });
    let result = if is_screenshot && !args.json && std::io::stdout().is_terminal() {
        let spinner = indicatif::ProgressBar::new_spinner();
        spinner.enable_steady_tick(Duration::from_millis(80));
        spinner.set_message("Taking screenshot...");
        let res = run_command(&mut client, args.command).await;
        spinner.finish_and_clear();
        res?
    } else {
        run_command(&mut client, args.command).await?
    };

    // Screenshot save-to-file: decode base64 data URL and write PNG
    if let Some(path) = screenshot_path {
        save_screenshot(&result, &path)?;
        if args.json {
            output::format_json(&serde_json::json!({"path": path.display().to_string()}))?;
        } else {
            println!(
                "{}",
                crate::style::success(&format!("Saved to {}", path.display()))
            );
        }
        return Ok(());
    }

    if args.json {
        output::format_json(&result)?;
    } else if is_snapshot {
        output::format_snapshot(&result);
    } else if is_diff {
        output::format_diff(&result);
    } else if is_logs {
        // console.clear returns {"cleared": true}, not an array
        if result.get("cleared").is_some() {
            output::format_text(&result);
        } else {
            print!("{}", output::format_logs(&result));
        }
    } else if is_network {
        if result.get("cleared").is_some() {
            output::format_text(&result);
        } else {
            print!("{}", output::format_network(&result));
        }
    } else if is_watch {
        output::format_watch(&result);
    } else {
        output::format_text(&result);
    }

    Ok(())
}

async fn follow_logs(
    client: &mut Client,
    emit_json: bool,
    level: Option<&str>,
    last: Option<usize>,
) -> Result<()> {
    let mut last_seen_id: u64 = 0;
    let mut first_poll = true;
    loop {
        let mut params = serde_json::Map::new();
        if last_seen_id > 0 {
            params.insert("sinceId".into(), json!(last_seen_id));
        }
        if let Some(l) = level {
            params.insert("level".into(), json!(l));
        }
        if first_poll {
            if let Some(n) = last {
                params.insert("last".into(), json!(n));
            }
            first_poll = false;
        }
        let result = client
            .call("console.getLogs", Some(Value::Object(params)))
            .await?;
        if let Some(entries) = result.as_array()
            && !entries.is_empty()
        {
            if emit_json {
                // Emit NDJSON: one JSON object per entry for jq compatibility
                for entry in entries {
                    println!("{entry}");
                }
            } else {
                print!("{}", output::format_logs(&result));
            }
            last_seen_id = entries
                .last()
                .and_then(|e| e.get("id"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(last_seen_id);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn follow_network(
    client: &mut Client,
    emit_json: bool,
    filter: Option<&str>,
    last: Option<usize>,
    failed: bool,
) -> Result<()> {
    let mut last_seen_id: u64 = 0;
    let mut first_poll = true;
    loop {
        let mut params = serde_json::Map::new();
        if last_seen_id > 0 {
            params.insert("sinceId".into(), json!(last_seen_id));
        }
        if let Some(f) = filter {
            params.insert("filter".into(), json!(f));
        }
        if failed {
            params.insert("failedOnly".into(), json!(true));
        }
        if first_poll {
            if let Some(n) = last {
                params.insert("last".into(), json!(n));
            }
            first_poll = false;
        }
        let result = client
            .call("network.getRequests", Some(Value::Object(params)))
            .await?;
        if let Some(entries) = result.as_array()
            && !entries.is_empty()
        {
            if emit_json {
                for entry in entries {
                    println!("{entry}");
                }
            } else {
                print!("{}", output::format_network(&result));
            }
            last_seen_id = entries
                .last()
                .and_then(|e| e.get("id"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(last_seen_id);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn run_command(client: &mut Client, command: Command) -> Result<serde_json::Value> {
    match command {
        Command::Ping => client.call("ping", None).await,
        Command::State => client.call("state", None).await,
        Command::Snapshot {
            interactive,
            selector,
            depth,
            save,
        } => {
            let result = client
                .call(
                    "snapshot",
                    Some(json!({
                        "interactive": interactive,
                        "selector": selector,
                        "depth": depth,
                    })),
                )
                .await?;
            if let Some(ref path) = save {
                let json = serde_json::to_string_pretty(&result)?;
                std::fs::write(path, &json)
                    .with_context(|| format!("Failed to save snapshot to {}", path.display()))?;
                eprintln!("Snapshot saved to {}", path.display());
            }
            Ok(result)
        }
        Command::Diff {
            r#ref: ref_path,
            interactive,
            selector,
            depth,
        } => run_diff_command(client, ref_path, interactive, selector, depth).await,
        Command::Ipc { command, args } => run_ipc_command(client, &command, args.as_deref()).await,
        Command::Screenshot { path, selector } => {
            client
                .call(
                    "screenshot",
                    Some(json!({"path": path, "selector": selector})),
                )
                .await
        }
        Command::Navigate { url } => client.call("navigate", Some(json!({"url": url}))).await,
        Command::Url => client.call("url", None).await,
        Command::Title => client.call("title", None).await,
        Command::Wait {
            target,
            selector,
            gone,
            timeout,
        } => {
            client
                .call(
                    "wait",
                    Some(json!({
                        "target": target,
                        "selector": selector,
                        "gone": gone,
                        "timeout": timeout,
                    })),
                )
                .await
        }
        Command::Watch {
            selector,
            timeout,
            stable,
        } => {
            let mut params = serde_json::Map::new();
            params.insert("timeout".into(), json!(timeout));
            params.insert("stable".into(), json!(stable));
            if let Some(sel) = selector {
                params.insert("selector".into(), json!(sel));
            }
            client
                .call("watch", Some(serde_json::Value::Object(params)))
                .await
        }
        Command::Logs {
            level,
            last,
            clear,
            follow,
        } => run_logs_command(client, level, last, clear, follow).await,
        Command::Network {
            filter,
            failed,
            last,
            clear,
            follow,
        } => run_network_command(client, filter, failed, last, clear, follow).await,
        Command::Assert(kind) => run_assert_command(client, kind).await,
        Command::Drop { target, file } => run_drop_command(client, &target, file).await,
        cmd => run_dom_command(client, cmd).await,
    }
}

async fn run_diff_command(
    client: &mut Client,
    ref_path: Option<std::path::PathBuf>,
    interactive: bool,
    selector: Option<String>,
    depth: Option<u8>,
) -> Result<serde_json::Value> {
    let mut params = json!({
        "interactive": interactive,
        "selector": selector,
        "depth": depth,
    });
    if let Some(path) = ref_path {
        let meta = std::fs::metadata(&path)
            .with_context(|| format!("Failed to stat snapshot file: {}", path.display()))?;
        anyhow::ensure!(
            meta.len() < 50 * 1024 * 1024,
            "Snapshot file too large (>50 MB): {}",
            path.display()
        );
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read snapshot file: {}", path.display()))?;
        let reference: serde_json::Value =
            serde_json::from_str(&content).context("Invalid snapshot file format")?;
        anyhow::ensure!(
            reference.get("elements").is_some(),
            "Snapshot file missing \"elements\" key — not a valid snapshot"
        );
        params["reference"] = reference;
    }
    client.call("diff", Some(params)).await
}

async fn run_dom_command(client: &mut Client, command: Command) -> Result<serde_json::Value> {
    match command {
        Command::Click { target } => client.call("click", Some(target_params(&target))).await,
        Command::Fill { target, value } => {
            let mut p = target_params(&target);
            p["value"] = json!(value);
            client.call("fill", Some(p)).await
        }
        Command::Type { target, text } => {
            let mut p = target_params(&target);
            p["text"] = json!(text);
            client.call("type", Some(p)).await
        }
        Command::Press { key } => client.call("press", Some(json!({"key": key}))).await,
        Command::Select { target, value } => {
            let mut p = target_params(&target);
            p["value"] = json!(value);
            client.call("select", Some(p)).await
        }
        Command::Check { target } => client.call("check", Some(target_params(&target))).await,
        Command::Scroll {
            direction,
            amount,
            r#ref,
        } => {
            client
                .call(
                    "scroll",
                    Some(json!({"direction": direction, "amount": amount, "ref": r#ref})),
                )
                .await
        }
        Command::Text { target } => client.call("text", Some(target_params(&target))).await,
        Command::Html { target } => {
            let params = target.map(|t| target_params(&t));
            client.call("html", params).await
        }
        Command::Value { target } => client.call("value", Some(target_params(&target))).await,
        Command::Attrs { target } => client.call("attrs", Some(target_params(&target))).await,
        Command::Eval { script } => client.call("eval", Some(json!({"script": script}))).await,
        Command::Drag {
            source,
            target,
            offset,
        } => {
            let mut p = json!({"source": target_params(&source)});
            if let Some(t) = target {
                p["target"] = target_params(&t);
            } else if let Some(off) = offset {
                let parts: Vec<&str> = off.split(',').collect();
                anyhow::ensure!(parts.len() == 2, "offset must be X,Y (e.g., '0,100')");
                let x: f64 = parts[0].trim().parse().context("invalid offset X value")?;
                let y: f64 = parts[1].trim().parse().context("invalid offset Y value")?;
                p["offset"] = json!({"x": x, "y": y});
            } else {
                anyhow::bail!("drag requires either a target element or --offset");
            }
            client.call("drag", Some(p)).await
        }
        _ => anyhow::bail!("unexpected command in run_dom_command"),
    }
}

/// Extract a string from a JSON value, bailing if not a string.
fn require_str(val: &serde_json::Value) -> Result<&str> {
    val.as_str()
        .ok_or_else(|| anyhow::anyhow!("expected string response from server"))
}

/// Extract a bool field from a JSON object, bailing if missing.
fn require_bool_field(val: &serde_json::Value, field: &str) -> Result<bool> {
    val.get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("missing '{field}' field in server response"))
}

/// Print assertion failure and exit with code 1.
fn assert_fail(msg: &str) -> ! {
    output::format_assert_fail(msg);
    std::process::exit(1)
}

async fn run_assert_command(client: &mut Client, kind: AssertKind) -> Result<serde_json::Value> {
    match kind {
        AssertKind::Text { target, expected } => {
            let result = client.call("text", Some(target_params(&target))).await?;
            let actual = require_str(&result)?;
            if actual != expected {
                assert_fail(&format!("expected text \"{expected}\", got \"{actual}\""));
            }
        }
        AssertKind::Visible { target } => {
            let visible = require_bool_field(
                &client.call("visible", Some(target_params(&target))).await?,
                "visible",
            )?;
            if !visible {
                assert_fail("element is not visible");
            }
        }
        AssertKind::Hidden { target } => {
            let visible = require_bool_field(
                &client.call("visible", Some(target_params(&target))).await?,
                "visible",
            )?;
            if visible {
                assert_fail("element is visible");
            }
        }
        AssertKind::Value { target, expected } => {
            let result = client.call("value", Some(target_params(&target))).await?;
            let actual = require_str(&result)?;
            if actual != expected {
                assert_fail(&format!("expected value \"{expected}\", got \"{actual}\""));
            }
        }
        AssertKind::Count { selector, expected } => {
            let result = client
                .call("count", Some(json!({"selector": selector})))
                .await?;
            let actual = result
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("missing 'count' field in server response"))?;
            if actual != expected {
                assert_fail(&format!("expected {expected} elements, found {actual}"));
            }
        }
        AssertKind::Checked { target } => {
            let checked = require_bool_field(
                &client.call("checked", Some(target_params(&target))).await?,
                "checked",
            )?;
            if !checked {
                assert_fail("element is not checked");
            }
        }
        AssertKind::Contains { target, expected } => {
            let result = client.call("text", Some(target_params(&target))).await?;
            let actual = require_str(&result)?;
            if !actual.contains(&expected) {
                assert_fail(&format!(
                    "text does not contain \"{expected}\", got \"{actual}\""
                ));
            }
        }
        AssertKind::Url { expected } => {
            let result = client.call("url", None).await?;
            let actual = require_str(&result)?;
            if !actual.contains(&expected) {
                assert_fail(&format!(
                    "URL does not contain \"{expected}\", got \"{actual}\""
                ));
            }
        }
    }
    Ok(json!({"ok": true}))
}

async fn run_ipc_command(
    client: &mut Client,
    command: &str,
    args: Option<&str>,
) -> Result<serde_json::Value> {
    let parsed_args: Option<serde_json::Value> = args.map(serde_json::from_str).transpose()?;
    client
        .call(
            "ipc",
            Some(json!({"command": command, "args": parsed_args})),
        )
        .await
}

async fn run_logs_command(
    client: &mut Client,
    level: Option<String>,
    last: Option<usize>,
    clear: bool,
    follow: bool,
) -> Result<serde_json::Value> {
    if follow {
        anyhow::bail!("follow mode must be handled before run_command");
    }
    if clear {
        return client.call("console.clear", None).await;
    }
    let mut params = serde_json::Map::new();
    if let Some(l) = level {
        params.insert("level".into(), json!(l));
    }
    if let Some(n) = last {
        params.insert("last".into(), json!(n));
    }
    client
        .call("console.getLogs", Some(serde_json::Value::Object(params)))
        .await
}

async fn run_network_command(
    client: &mut Client,
    filter: Option<String>,
    failed: bool,
    last: Option<usize>,
    clear: bool,
    follow: bool,
) -> Result<serde_json::Value> {
    if follow {
        anyhow::bail!("follow mode must be handled before run_command");
    }
    if clear {
        return client.call("network.clear", None).await;
    }
    let mut params = serde_json::Map::new();
    if let Some(f) = filter {
        params.insert("filter".into(), json!(f));
    }
    if failed {
        params.insert("failedOnly".into(), json!(true));
    }
    if let Some(n) = last {
        params.insert("last".into(), json!(n));
    }
    client
        .call(
            "network.getRequests",
            Some(serde_json::Value::Object(params)),
        )
        .await
}

const MAX_DROP_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50 MB per file
const MAX_TOTAL_DROP_SIZE: usize = 100 * 1024 * 1024; // 100 MB total base64 payload

async fn run_drop_command(
    client: &mut Client,
    target: &str,
    file: Vec<std::path::PathBuf>,
) -> Result<serde_json::Value> {
    let mut p = target_params(target);
    let mut files = Vec::new();
    let mut total_encoded = 0usize;
    for path in &file {
        let meta = std::fs::metadata(path)
            .with_context(|| format!("Failed to stat file: {}", path.display()))?;
        anyhow::ensure!(meta.is_file(), "Not a regular file: {}", path.display());
        anyhow::ensure!(
            meta.len() <= MAX_DROP_FILE_SIZE,
            "File too large (>50 MB): {}",
            path.display()
        );
        let data = std::fs::read(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        total_encoded += encoded.len();
        anyhow::ensure!(
            total_encoded <= MAX_TOTAL_DROP_SIZE,
            "Total drop payload exceeds 100 MB limit"
        );
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mime = mime_from_ext(path);
        files.push(json!({"name": name, "type": mime, "data": encoded}));
    }
    p["files"] = json!(files);
    client.call("drop", Some(p)).await
}

fn target_params(raw: &str) -> serde_json::Value {
    match parse_target(raw) {
        Target::Ref(r) => json!({"ref": r}),
        Target::Selector(s) => json!({"selector": s}),
        Target::Coords(x, y) => json!({"x": x, "y": y}),
    }
}

fn mime_from_ext(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("json") => "application/json",
        Some("txt") => "text/plain",
        Some("html" | "htm") => "text/html",
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("csv") => "text/csv",
        Some("xml") => "application/xml",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Decode a base64 data URL and write the PNG file.
fn save_screenshot(result: &serde_json::Value, path: &std::path::Path) -> Result<()> {
    let data_url = result
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Screenshot result is not a string"))?;

    let base64_data = data_url
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(data_url);

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| anyhow::anyhow!("Failed to decode base64: {e}"))?;

    std::fs::write(path, bytes)?;
    Ok(())
}

/// Resolve the socket path from explicit arg, env var, or auto-detection.
fn resolve_socket(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    // Auto-detect: find the most recent tauri-pilot-*.sock in /tmp
    let mut candidates: Vec<PathBuf> = std::fs::read_dir("/tmp")
        .map_err(|e| anyhow::anyhow!("Failed to read /tmp: {e}"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with("tauri-pilot-")
                    && std::path::Path::new(n)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("sock"))
            })
        })
        .collect();

    candidates.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    candidates
        .pop()
        .ok_or_else(|| anyhow::anyhow!("No tauri-pilot socket found. Is a Tauri app running?"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mime_from_ext_png() {
        assert_eq!(
            mime_from_ext(std::path::Path::new("photo.png")),
            "image/png"
        );
    }

    #[test]
    fn test_mime_from_ext_jpeg_variants() {
        assert_eq!(mime_from_ext(std::path::Path::new("a.jpg")), "image/jpeg");
        assert_eq!(mime_from_ext(std::path::Path::new("b.jpeg")), "image/jpeg");
    }

    #[test]
    fn test_mime_from_ext_unknown_defaults_to_octet_stream() {
        assert_eq!(
            mime_from_ext(std::path::Path::new("data.bin")),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_mime_from_ext_no_extension() {
        assert_eq!(
            mime_from_ext(std::path::Path::new("Makefile")),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_mime_from_ext_case_insensitive() {
        assert_eq!(
            mime_from_ext(std::path::Path::new("PHOTO.PNG")),
            "image/png"
        );
        assert_eq!(mime_from_ext(std::path::Path::new("file.PnG")), "image/png");
        assert_eq!(
            mime_from_ext(std::path::Path::new("doc.PDF")),
            "application/pdf"
        );
    }
}
