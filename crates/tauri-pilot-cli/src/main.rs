mod cli;
mod client;
mod mcp;
mod output;
mod protocol;
mod scenario;
mod style;

use std::fmt::Write as _;

use anyhow::{Context, Result};
use base64::Engine;
use clap::Parser;
use serde_json::{Value, json};
use std::io::IsTerminal;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use cli::{
    AssertKind, Cli, Command, FormsArgs, RecordAction, StorageAction, StorageArgs, Target,
    parse_target,
};
use client::Client;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    let is_mcp = matches!(args.command, Command::Mcp);
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));

    // CLI tools must keep stdout reserved for data so callers can pipe into
    // `jq`, `python -c 'json.load(...)'`, etc. without log noise corrupting the
    // payload (see #80).
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .init();

    if is_mcp {
        return mcp::run_mcp_server(args.socket, args.window).await;
    }

    if let Command::Run {
        ref scenario,
        ref junit,
        no_fail_fast,
    } = args.command
    {
        return run_scenario_command(
            scenario,
            junit.as_deref(),
            no_fail_fast,
            args.socket,
            args.window.as_deref(),
        )
        .await;
    }

    let socket = resolve_socket(args.socket)?;
    let mut client = Client::connect(&socket).await?;

    // Handle --follow mode: loop forever polling for new entries
    if let Command::Logs {
        follow: true,
        ref level,
        ref last,
        ..
    } = args.command
    {
        follow_logs(
            &mut client,
            args.json,
            level.as_deref(),
            *last,
            args.window.as_deref(),
        )
        .await?;
    } else if let Command::Network {
        follow: true,
        ref filter,
        ref last,
        failed,
        ..
    } = args.command
    {
        follow_network(
            &mut client,
            args.json,
            filter.as_deref(),
            *last,
            failed,
            args.window.as_deref(),
        )
        .await?;
    }

    let screenshot_path = if let Command::Screenshot { ref path, .. } = args.command {
        path.clone()
    } else {
        None
    };
    let is_screenshot = matches!(args.command, Command::Screenshot { .. });
    // Capture output kind before command is consumed by run_command
    let output_kind = OutputKind::from(&args.command);
    let result = if is_screenshot && !args.json && std::io::stdout().is_terminal() {
        let spinner = indicatif::ProgressBar::new_spinner();
        spinner.enable_steady_tick(Duration::from_millis(80));
        spinner.set_message("Taking screenshot...");
        let res = run_command(&mut client, args.command, args.window.as_deref()).await;
        spinner.finish_and_clear();
        res?
    } else {
        run_command(&mut client, args.command, args.window.as_deref()).await?
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

    format_result(output_kind, &result, args.json)?;
    Ok(())
}

#[derive(Copy, Clone)]
enum OutputKind {
    Snapshot,
    Diff,
    Logs,
    Network,
    Watch,
    Storage,
    Forms,
    Windows,
    Record,
    Replay,
    Text,
}

impl From<&Command> for OutputKind {
    fn from(cmd: &Command) -> Self {
        match cmd {
            Command::Snapshot { .. } => OutputKind::Snapshot,
            Command::Diff { .. } => OutputKind::Diff,
            Command::Logs { .. } => OutputKind::Logs,
            Command::Network { .. } => OutputKind::Network,
            Command::Watch { .. } => OutputKind::Watch,
            Command::Storage(..) => OutputKind::Storage,
            Command::Forms(..) => OutputKind::Forms,
            Command::Windows => OutputKind::Windows,
            Command::Record { .. } => OutputKind::Record,
            Command::Replay { .. } => OutputKind::Replay,
            _ => OutputKind::Text,
        }
    }
}

fn format_result(kind: OutputKind, result: &serde_json::Value, emit_json: bool) -> Result<()> {
    if emit_json {
        return output::format_json(result);
    }
    match kind {
        OutputKind::Snapshot => output::format_snapshot(result),
        OutputKind::Diff => output::format_diff(result),
        OutputKind::Logs => {
            if result.get("cleared").is_some() {
                output::format_text(result);
            } else {
                print!("{}", output::format_logs(result));
            }
        }
        OutputKind::Network => {
            if result.get("cleared").is_some() {
                output::format_text(result);
            } else {
                print!("{}", output::format_network(result));
            }
        }
        OutputKind::Watch => output::format_watch(result),
        OutputKind::Storage => {
            if result.get("cleared").is_some() {
                output::format_text(result);
            } else if result.get("entries").is_some() {
                output::format_storage(result);
            } else if result.get("found").is_some() {
                output::format_storage_value(result);
            } else {
                output::format_text(result);
            }
        }
        OutputKind::Forms => output::format_forms(result),
        OutputKind::Windows => output::format_windows(result),
        OutputKind::Record => {
            let formatted = output::format_record(result);
            if !formatted.is_empty() {
                println!("{formatted}");
            }
        }
        OutputKind::Replay | OutputKind::Text => output::format_text(result),
    }
    Ok(())
}

async fn follow_logs(
    client: &mut Client,
    emit_json: bool,
    level: Option<&str>,
    last: Option<usize>,
    window: Option<&str>,
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
        if first_poll && let Some(n) = last {
            params.insert("last".into(), json!(n));
            first_poll = false;
        }
        let result = client
            .call(
                "console.getLogs",
                with_window(Some(Value::Object(params)), window),
            )
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
    window: Option<&str>,
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
        if first_poll && let Some(n) = last {
            params.insert("last".into(), json!(n));
            first_poll = false;
        }
        let result = client
            .call(
                "network.getRequests",
                with_window(Some(Value::Object(params)), window),
            )
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

pub(crate) fn with_window(params: Option<Value>, window: Option<&str>) -> Option<Value> {
    match (params, window) {
        (Some(Value::Object(mut map)), Some(w)) => {
            map.insert("window".to_string(), json!(w));
            Some(Value::Object(map))
        }
        (None, Some(w)) => Some(json!({"window": w})),
        (params, _) => params,
    }
}

async fn run_command(
    client: &mut Client,
    command: Command,
    window: Option<&str>,
) -> Result<serde_json::Value> {
    match command {
        Command::Mcp => anyhow::bail!("mcp must be handled before run_command"),
        Command::Run { .. } => anyhow::bail!("run must be handled before run_command"),
        Command::Windows => client.call("windows.list", None).await,
        Command::Ping => client.call("ping", with_window(None, window)).await,
        Command::State => client.call("state", with_window(None, window)).await,
        Command::Snapshot {
            interactive,
            selector,
            depth,
            save,
        } => run_snapshot_command(client, interactive, selector, depth, save, window).await,
        Command::Diff {
            r#ref: ref_path,
            interactive,
            selector,
            depth,
        } => run_diff_command(client, ref_path, interactive, selector, depth, window).await,
        Command::Ipc { command, args } => {
            run_ipc_command(client, &command, args.as_deref(), window).await
        }
        Command::Screenshot { path, selector } => {
            client
                .call(
                    "screenshot",
                    with_window(Some(json!({"path": path, "selector": selector})), window),
                )
                .await
        }
        Command::Navigate { url } => {
            client
                .call("navigate", with_window(Some(json!({"url": url})), window))
                .await
        }
        Command::Url => client.call("url", with_window(None, window)).await,
        Command::Title => client.call("title", with_window(None, window)).await,
        Command::Wait {
            target,
            selector,
            gone,
            timeout,
        } => {
            let params = build_wait_params(target.as_deref(), selector.as_deref(), gone, timeout);
            client.call("wait", with_window(Some(params), window)).await
        }
        Command::Watch {
            selector,
            timeout,
            stable,
            require_mutation,
        } => run_watch_command(client, selector, timeout, stable, require_mutation, window).await,
        Command::Logs {
            level,
            last,
            clear,
            follow,
        } => run_logs_command(client, level, last, clear, follow, window).await,
        Command::Network {
            filter,
            failed,
            last,
            clear,
            follow,
        } => run_network_command(client, filter, failed, last, clear, follow, window).await,
        Command::Assert(kind) => run_assert_command(client, kind, window).await,
        Command::Storage(storage_args) => run_storage_command(client, storage_args, window).await,
        Command::Forms(args) => run_forms_command(client, args, window).await,
        Command::Drop { target, file } => run_drop_command(client, &target, file, window).await,
        Command::Record { action } => run_record_command(client, action, window).await,
        Command::Replay { path, export } => {
            run_replay_command(client, &path, export.as_deref(), window).await
        }
        cmd => run_dom_command(client, cmd, window).await,
    }
}

async fn run_snapshot_command(
    client: &mut Client,
    interactive: bool,
    selector: Option<String>,
    depth: Option<u8>,
    save: Option<std::path::PathBuf>,
    window: Option<&str>,
) -> Result<serde_json::Value> {
    let params = with_window(
        Some(json!({
            "interactive": interactive,
            "selector": selector,
            "depth": depth,
        })),
        window,
    );
    let mut result = client.call("snapshot", params).await?;
    if let Some(ref path) = save {
        let json = serde_json::to_string_pretty(&result)?;
        std::fs::write(path, &json)
            .with_context(|| format!("Failed to save snapshot to {}", path.display()))?;
        // Embed the saved path in the JSON result so `--json` consumers can
        // recover it without parsing stderr (matches `record stop --output`
        // and `screenshot` conventions; see #80).
        if let serde_json::Value::Object(ref mut obj) = result {
            obj.insert("path".into(), json!(path.display().to_string()));
        }
        eprintln!("Snapshot saved to {}", path.display());
    }
    Ok(result)
}

async fn run_watch_command(
    client: &mut Client,
    selector: Option<String>,
    timeout: u64,
    stable: u64,
    require_mutation: bool,
    window: Option<&str>,
) -> Result<serde_json::Value> {
    let mut params = serde_json::Map::new();
    params.insert("timeout".into(), json!(timeout));
    params.insert("stable".into(), json!(stable));
    if require_mutation {
        params.insert("requireMutation".into(), json!(true));
    }
    if let Some(sel) = selector {
        params.insert("selector".into(), json!(sel));
    }
    client
        .call(
            "watch",
            with_window(Some(serde_json::Value::Object(params)), window),
        )
        .await
}

async fn run_diff_command(
    client: &mut Client,
    ref_path: Option<std::path::PathBuf>,
    interactive: bool,
    selector: Option<String>,
    depth: Option<u8>,
    window: Option<&str>,
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
    client.call("diff", with_window(Some(params), window)).await
}

fn read_script(reader: &mut impl std::io::Read) -> Result<String> {
    let mut s = String::new();
    reader
        .read_to_string(&mut s)
        .context("reading script from stdin")?;
    anyhow::ensure!(
        !s.trim().is_empty(),
        "script read from stdin is empty or blank"
    );
    Ok(s)
}

async fn handle_eval(
    client: &mut Client,
    script: Option<String>,
    window: Option<&str>,
) -> Result<Value> {
    let script = match script.as_deref() {
        None | Some("-") => {
            anyhow::ensure!(
                !std::io::stdin().is_terminal(),
                "stdin is a terminal: pass a script as argument or pipe it in"
            );
            read_script(&mut std::io::stdin())?
        }
        Some(s) => s.to_owned(),
    };
    client
        .call("eval", with_window(Some(json!({"script": script})), window))
        .await
}

async fn run_dom_command(
    client: &mut Client,
    command: Command,
    window: Option<&str>,
) -> Result<serde_json::Value> {
    match command {
        Command::Click { target } => {
            client
                .call("click", with_window(Some(target_params(&target)), window))
                .await
        }
        Command::Fill { target, value } => {
            let mut p = target_params(&target);
            p["value"] = json!(value);
            client.call("fill", with_window(Some(p), window)).await
        }
        Command::Type { target, text } => {
            let mut p = target_params(&target);
            p["text"] = json!(text);
            client.call("type", with_window(Some(p), window)).await
        }
        Command::Press { key } => {
            client
                .call("press", with_window(Some(json!({"key": key})), window))
                .await
        }
        Command::Select { target, value } => {
            let mut p = target_params(&target);
            p["value"] = json!(value);
            client.call("select", with_window(Some(p), window)).await
        }
        Command::Check { target } => {
            client
                .call("check", with_window(Some(target_params(&target)), window))
                .await
        }
        Command::Scroll {
            direction,
            amount,
            r#ref,
        } => {
            client
                .call(
                    "scroll",
                    with_window(
                        Some(json!({"direction": direction, "amount": amount, "ref": r#ref})),
                        window,
                    ),
                )
                .await
        }
        Command::Text { target } => {
            client
                .call("text", with_window(Some(target_params(&target)), window))
                .await
        }
        Command::Html { target } => {
            let params = target.map(|t| target_params(&t));
            client.call("html", with_window(params, window)).await
        }
        Command::Value { target } => {
            client
                .call("value", with_window(Some(target_params(&target)), window))
                .await
        }
        Command::Attrs { target } => {
            client
                .call("attrs", with_window(Some(target_params(&target)), window))
                .await
        }
        Command::Eval { script } => handle_eval(client, script, window).await,
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
            client.call("drag", with_window(Some(p), window)).await
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

async fn run_assert_command(
    client: &mut Client,
    kind: AssertKind,
    window: Option<&str>,
) -> Result<serde_json::Value> {
    match kind {
        AssertKind::Text { target, expected } => {
            let result = client
                .call("text", with_window(Some(target_params(&target)), window))
                .await?;
            let actual = require_str(&result)?;
            if actual != expected {
                assert_fail(&format!("expected text \"{expected}\", got \"{actual}\""));
            }
        }
        AssertKind::Visible { target } => {
            let visible = require_bool_field(
                &client
                    .call("visible", with_window(Some(target_params(&target)), window))
                    .await?,
                "visible",
            )?;
            if !visible {
                assert_fail("element is not visible");
            }
        }
        AssertKind::Hidden { target } => {
            let visible = require_bool_field(
                &client
                    .call("visible", with_window(Some(target_params(&target)), window))
                    .await?,
                "visible",
            )?;
            if visible {
                assert_fail("element is visible");
            }
        }
        AssertKind::Value { target, expected } => {
            let result = client
                .call("value", with_window(Some(target_params(&target)), window))
                .await?;
            let actual = require_str(&result)?;
            if actual != expected {
                assert_fail(&format!("expected value \"{expected}\", got \"{actual}\""));
            }
        }
        AssertKind::Count { selector, expected } => {
            let result = client
                .call(
                    "count",
                    with_window(Some(json!({"selector": selector})), window),
                )
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
                &client
                    .call("checked", with_window(Some(target_params(&target)), window))
                    .await?,
                "checked",
            )?;
            if !checked {
                assert_fail("element is not checked");
            }
        }
        AssertKind::Contains { target, expected } => {
            let result = client
                .call("text", with_window(Some(target_params(&target)), window))
                .await?;
            let actual = require_str(&result)?;
            if !actual.contains(&expected) {
                assert_fail(&format!(
                    "text does not contain \"{expected}\", got \"{actual}\""
                ));
            }
        }
        AssertKind::Url { expected } => {
            let result = client.call("url", with_window(None, window)).await?;
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
    window: Option<&str>,
) -> Result<serde_json::Value> {
    let parsed_args: Option<serde_json::Value> = args.map(serde_json::from_str).transpose()?;
    client
        .call(
            "ipc",
            with_window(
                Some(json!({"command": command, "args": parsed_args})),
                window,
            ),
        )
        .await
}

async fn run_logs_command(
    client: &mut Client,
    level: Option<String>,
    last: Option<usize>,
    clear: bool,
    follow: bool,
    window: Option<&str>,
) -> Result<serde_json::Value> {
    if follow {
        anyhow::bail!("follow mode must be handled before run_command");
    }
    if clear {
        return client
            .call("console.clear", with_window(None, window))
            .await;
    }
    let mut params = serde_json::Map::new();
    if let Some(l) = level {
        params.insert("level".into(), json!(l));
    }
    if let Some(n) = last {
        params.insert("last".into(), json!(n));
    }
    client
        .call(
            "console.getLogs",
            with_window(Some(serde_json::Value::Object(params)), window),
        )
        .await
}

async fn run_network_command(
    client: &mut Client,
    filter: Option<String>,
    failed: bool,
    last: Option<usize>,
    clear: bool,
    follow: bool,
    window: Option<&str>,
) -> Result<serde_json::Value> {
    if follow {
        anyhow::bail!("follow mode must be handled before run_command");
    }
    if clear {
        return client
            .call("network.clear", with_window(None, window))
            .await;
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
            with_window(Some(serde_json::Value::Object(params)), window),
        )
        .await
}

async fn run_forms_command(
    client: &mut Client,
    args: FormsArgs,
    window: Option<&str>,
) -> Result<serde_json::Value> {
    let params = args.selector.map(|s| json!({"selector": s}));
    client.call("forms.dump", with_window(params, window)).await
}

async fn run_storage_command(
    client: &mut Client,
    args: StorageArgs,
    window: Option<&str>,
) -> Result<serde_json::Value> {
    let session = args.session;
    match args.action {
        StorageAction::Get { key } => {
            client
                .call(
                    "storage.get",
                    with_window(Some(json!({"key": key, "session": session})), window),
                )
                .await
        }
        StorageAction::Set { key, value } => {
            client
                .call(
                    "storage.set",
                    with_window(
                        Some(json!({"key": key, "value": value, "session": session})),
                        window,
                    ),
                )
                .await
        }
        StorageAction::List => {
            client
                .call(
                    "storage.list",
                    with_window(Some(json!({"session": session})), window),
                )
                .await
        }
        StorageAction::Clear => {
            client
                .call(
                    "storage.clear",
                    with_window(Some(json!({"session": session})), window),
                )
                .await
        }
    }
}

const MAX_DROP_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50 MB per file
const MAX_TOTAL_DROP_SIZE: usize = 100 * 1024 * 1024; // 100 MB total base64 payload

pub(crate) async fn run_drop_command(
    client: &mut Client,
    target: &str,
    file: Vec<std::path::PathBuf>,
    window: Option<&str>,
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
    client.call("drop", with_window(Some(p), window)).await
}

pub(crate) fn target_params(raw: &str) -> serde_json::Value {
    match parse_target(raw) {
        Target::Ref(r) => json!({"ref": r}),
        Target::Selector(s) => json!({"selector": s}),
        Target::Coords(x, y) => json!({"x": x, "y": y}),
    }
}

/// Build params for the `wait` RPC call.
///
/// Routing precedence:
/// - `--selector` flag wins over positional target.
/// - Positional target is parsed via [`parse_target`] (`@x` ref, else selector).
///   Coords are not meaningful for `wait`, so they pass through as a selector
///   so the bridge surfaces a `SyntaxError` from `document.querySelector`
///   instead of silently timing out.
/// - With neither target nor selector, only `gone`/`timeout` are sent and the
///   bridge `waitFor` rejects up-front (issue #74).
pub(crate) fn build_wait_params(
    target: Option<&str>,
    selector: Option<&str>,
    gone: bool,
    timeout: u64,
) -> serde_json::Value {
    match (selector, target) {
        (Some(s), _) => json!({ "selector": s, "gone": gone, "timeout": timeout }),
        (None, Some(t)) => match parse_target(t) {
            // `@` alone strips to an empty ref. Drop it so the bridge's
            // `waitFor` rejects up-front instead of hanging on a
            // `MutationObserver` until the timeout fires.
            Target::Ref(r) if r.is_empty() => json!({ "gone": gone, "timeout": timeout }),
            Target::Ref(r) => json!({ "ref": r, "gone": gone, "timeout": timeout }),
            Target::Selector(s) => json!({ "selector": s, "gone": gone, "timeout": timeout }),
            Target::Coords(..) => json!({ "selector": t, "gone": gone, "timeout": timeout }),
        },
        (None, None) => json!({ "gone": gone, "timeout": timeout }),
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

async fn run_record_command(
    client: &mut Client,
    action: RecordAction,
    window: Option<&str>,
) -> Result<Value> {
    match action {
        RecordAction::Start => client.call("record.start", with_window(None, window)).await,
        RecordAction::Stop { output } => {
            let result = client
                .call("record.stop", with_window(None, window))
                .await?;
            let entries = result
                .get("entries")
                .and_then(|e| e.as_array())
                .ok_or_else(|| anyhow::anyhow!("no entries in response"))?;
            let json = serde_json::to_string_pretty(entries)?;
            std::fs::write(&output, &json)
                .with_context(|| format!("Failed to write recording to {}", output.display()))?;
            Ok(serde_json::json!({
                "status": "saved",
                "path": output.display().to_string(),
                "count": entries.len()
            }))
        }
        RecordAction::Status => {
            client
                .call("record.status", with_window(None, window))
                .await
        }
    }
}

pub(crate) async fn run_replay_command(
    client: &mut Client,
    path: &std::path::Path,
    export: Option<&str>,
    window: Option<&str>,
) -> Result<Value> {
    let entries = read_replay_entries(path)?;

    if let Some(fmt) = export {
        return export_replay_command(&entries, fmt);
    }

    let total = entries.len();
    let mut prev_ts: u64 = 0;
    let mut passed = 0;
    let mut skipped = 0;

    for (i, entry) in entries.iter().enumerate() {
        let action = entry
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("unknown");
        let timestamp = entry
            .get("timestamp")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        let delta = timestamp.saturating_sub(prev_ts);
        if delta > 0 && i > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delta)).await;
        }
        prev_ts = timestamp;

        if !is_replayable(action) {
            skipped += 1;
            eprintln!(
                "{}",
                crate::output::format_replay_step(i + 1, total, action, "SKIP")
            );
            continue;
        }

        let mut params = serde_json::Map::new();
        if let Some(obj) = entry.as_object() {
            for (k, v) in obj {
                if k != "action" && k != "timestamp" {
                    params.insert(k.clone(), v.clone());
                }
            }
        }

        if let Some(w) = window {
            params.insert("window".to_string(), Value::String(w.to_string()));
        }

        let result = client.call(action, Some(Value::Object(params))).await;

        let status = if result.is_ok() {
            passed += 1;
            "ok"
        } else {
            "FAIL"
        };
        eprintln!(
            "{}",
            crate::output::format_replay_step(i + 1, total, action, status)
        );
    }

    let executed = total - skipped;
    let status = if passed == executed { "ok" } else { "failed" };
    Ok(serde_json::json!({
        "status": status,
        "total": total,
        "passed": passed,
        "skipped": skipped,
        "failed": executed - passed
    }))
}

fn read_replay_entries(path: &std::path::Path) -> Result<Vec<serde_json::Value>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read recording file: {}", path.display()))?;
    serde_json::from_str(&content).context("Invalid recording file format")
}

pub(crate) fn export_replay_file(path: &std::path::Path, export: &str) -> Result<Value> {
    let entries = read_replay_entries(path)?;
    export_replay_command(&entries, export)
}

fn export_replay_command(entries: &[Value], export: &str) -> Result<Value> {
    if export == "sh" {
        return Ok(Value::String(export_shell_script(entries)));
    }
    anyhow::bail!("unsupported export format: {export}; supported: sh")
}

fn export_shell_script(entries: &[Value]) -> String {
    let mut script = String::from("#!/bin/bash\n# Generated by tauri-pilot record/replay\n\n");
    let mut prev_ts: u64 = 0;

    for (i, entry) in entries.iter().enumerate() {
        let action = entry
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("unknown");
        let timestamp = entry
            .get("timestamp")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        let delta = timestamp.saturating_sub(prev_ts);
        if delta > 0 && i > 0 {
            let secs = std::time::Duration::from_millis(delta).as_secs_f64();
            let _ = writeln!(script, "sleep {secs:.1}");
        }
        prev_ts = timestamp;

        script.push_str(&entry_to_cli_command(action, entry));
        script.push('\n');
    }

    script
}

/// Wrap a string in single quotes with proper escaping for shell safety.
fn shell_escape(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 2);
    escaped.push('\'');
    for c in s.chars() {
        if c == '\'' {
            escaped.push_str("'\\''");
        } else {
            escaped.push(c);
        }
    }
    escaped.push('\'');
    escaped
}

/// Returns `true` for actions that are safe to replay.
fn is_replayable(action: &str) -> bool {
    matches!(
        action,
        "click"
            | "fill"
            | "type"
            | "press"
            | "select"
            | "check"
            | "scroll"
            | "drag"
            | "drop"
            | "navigate"
            | "snapshot"
            | "wait"
            | "eval"
    )
}

/// Extract a CLI target string from a nested source/target object.
/// Handles `{"ref": "e3"}`, `{"selector": ".foo"}`, and `{"x": N, "y": N}`.
fn resolve_export_target(val: Option<&Value>) -> Option<String> {
    let obj = val?;
    if let Some(r) = obj.get("ref").and_then(|r| r.as_str()) {
        return Some(format!("@{r}"));
    }
    if let Some(s) = obj.get("selector").and_then(|s| s.as_str()) {
        return Some(shell_escape(s));
    }
    if let (Some(x), Some(y)) = (
        obj.get("x").and_then(serde_json::Value::as_i64),
        obj.get("y").and_then(serde_json::Value::as_i64),
    ) {
        return Some(format!("{x},{y}"));
    }
    None
}

fn entry_to_cli_command(action: &str, entry: &Value) -> String {
    let ref_id = entry.get("ref").and_then(|r| r.as_str());
    let selector = entry.get("selector").and_then(|s| s.as_str());
    let target = if let Some(r) = ref_id {
        format!("@{r}")
    } else if let Some(s) = selector {
        shell_escape(s)
    } else {
        String::new()
    };

    match action {
        "click" => format!("tauri-pilot click {target}"),
        "fill" => {
            let value = entry.get("value").and_then(|v| v.as_str()).unwrap_or("");
            format!("tauri-pilot fill {target} {}", shell_escape(value))
        }
        "type" => {
            let text = entry.get("text").and_then(|v| v.as_str()).unwrap_or("");
            format!("tauri-pilot type {target} {}", shell_escape(text))
        }
        "press" => {
            let key = entry.get("key").and_then(|k| k.as_str()).unwrap_or("");
            format!("tauri-pilot press {}", shell_escape(key))
        }
        "select" => {
            let value = entry.get("value").and_then(|v| v.as_str()).unwrap_or("");
            format!("tauri-pilot select {target} {}", shell_escape(value))
        }
        "check" => format!("tauri-pilot check {target}"),
        "scroll" => {
            let dir = entry
                .get("direction")
                .and_then(|d| d.as_str())
                .unwrap_or("down");
            let mut cmd = format!("tauri-pilot scroll {}", shell_escape(dir));
            if let Some(amt) = entry.get("amount").and_then(serde_json::Value::as_i64) {
                let _ = write!(cmd, " {amt}");
            }
            if let Some(r) = entry.get("ref").and_then(|r| r.as_str()) {
                let _ = write!(cmd, " --ref @{r}");
            }
            cmd
        }
        "drag" => {
            let src = resolve_export_target(entry.get("source"));
            let dst = resolve_export_target(entry.get("target"));
            // Offsets may be stored as floats (browser getBoundingClientRect
            // returns fractional pixels). The CLI --offset parser accepts
            // f64 values, so pass them through without truncation.
            let offset = entry.get("offset").and_then(|o| {
                let x = o.get("x").and_then(serde_json::Value::as_f64)?;
                let y = o.get("y").and_then(serde_json::Value::as_f64)?;
                Some(format!("{x},{y}"))
            });
            match (src, dst, offset) {
                (Some(s), Some(d), _) => format!("tauri-pilot drag {s} {d}"),
                (Some(s), None, Some(off)) => format!("tauri-pilot drag {s} --offset {off}"),
                (Some(s), None, None) => format!("tauri-pilot drag {s}"),
                _ => "# drag: missing source ref/selector".to_string(),
            }
        }
        "drop" => {
            // drop requires --file with file data; cannot be fully exported
            // as a shell command since recordings store base64 file contents.
            format!("# drop {target} (requires --file; not exportable)")
        }
        "navigate" => {
            let url = entry.get("url").and_then(|u| u.as_str()).unwrap_or("");
            format!("tauri-pilot navigate {}", shell_escape(url))
        }
        _ => {
            let safe = action.replace(['\n', '\r'], " ");
            format!("# unknown action: {safe}")
        }
    }
}

async fn run_scenario_command(
    scenario_path: &std::path::Path,
    junit: Option<&std::path::Path>,
    no_fail_fast: bool,
    explicit_socket: Option<PathBuf>,
    window: Option<&str>,
) -> Result<()> {
    let loaded = scenario::load_scenario(scenario_path)?;

    // Socket resolution: CLI flag > TOML [connect].socket > auto-detect
    let socket = explicit_socket
        .or_else(|| loaded.connect.as_ref().and_then(|c| c.socket.clone()))
        .map_or_else(|| resolve_socket(None), Ok)?;

    let connect_timeout = loaded
        .connect
        .as_ref()
        .and_then(|c| c.timeout_ms)
        .map(std::time::Duration::from_millis);
    let mut client = match connect_timeout {
        Some(t) => tokio::time::timeout(t, Client::connect(&socket))
            .await
            .map_err(|_| anyhow::anyhow!("connection timed out after {}ms", t.as_millis()))??,
        None => Client::connect(&socket).await?,
    };

    let fail_fast_override = if no_fail_fast { Some(false) } else { None };
    let global_ms = loaded.scenario.global_timeout_ms;
    let report = match global_ms {
        Some(ms) => {
            let t = std::time::Duration::from_millis(ms);
            tokio::time::timeout(
                t,
                scenario::run_scenario(&mut client, &loaded, window, fail_fast_override),
            )
            .await
            .map_err(|_| anyhow::anyhow!("scenario exceeded global timeout of {ms}ms"))??
        }
        None => scenario::run_scenario(&mut client, &loaded, window, fail_fast_override).await?,
    };

    scenario::print_report(&report);

    if let Some(xml_path) = junit {
        scenario::write_junit_xml(&report, xml_path)?;
        eprintln!("JUnit XML written to {}", xml_path.display());
    }

    if !report.all_passed() {
        std::process::exit(1);
    }
    Ok(())
}

/// Resolve the socket path from explicit arg, env var, or auto-detection.
pub(crate) fn resolve_socket(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    #[cfg(unix)]
    {
        if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            let xdg = PathBuf::from(xdg);
            if let Some(socket) = newest_socket_in_dir(&xdg) {
                return Ok(socket);
            }
        }

        newest_socket_in_dir(Path::new("/tmp"))
            .ok_or_else(|| anyhow::anyhow!("No tauri-pilot socket found. Is a Tauri app running?"))
    }

    #[cfg(windows)]
    {
        resolve_socket_windows()
    }
}

#[cfg(windows)]
fn resolve_socket_windows() -> Result<PathBuf> {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    fn is_pid_alive(pid: u32) -> bool {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
        match handle {
            Ok(h) => {
                let alive = unsafe {
                    let mut exit_code: u32 = 0;
                    GetExitCodeProcess(h, &raw mut exit_code).is_ok()
                        && exit_code == STILL_ACTIVE.0 as u32
                };
                unsafe {
                    let _ = CloseHandle(h);
                }
                alive
            }
            Err(_) => false,
        }
    }

    #[derive(serde::Deserialize)]
    struct InstanceEntry {
        pid: u32,
        pipe: String,
        created_at: u64,
    }

    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "LOCALAPPDATA environment variable is not set or empty. \
                 Is a Tauri app running?"
            )
        })?;

    let instances_dir = PathBuf::from(local_app_data)
        .join("tauri-pilot")
        .join("instances");

    if !instances_dir.exists() {
        anyhow::bail!(
            "No tauri-pilot instances directory found at {}. \
             Is a Tauri app running?",
            instances_dir.display()
        );
    }

    let mut newest: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(&instances_dir)?.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping corrupted instance file");
                continue;
            }
        };
        let info: InstanceEntry = match serde_json::from_str(&content) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skipping malformed instance file");
                continue;
            }
        };
        if !is_pid_alive(info.pid) {
            tracing::debug!(pid = info.pid, path = %path.display(), "skipping stale instance");
            continue;
        }
        let should_update = match newest {
            None => true,
            Some((current_max, _)) => info.created_at > current_max,
        };
        if should_update {
            newest = Some((info.created_at, PathBuf::from(info.pipe)));
        }
    }

    newest.map(|(_, pipe)| pipe).ok_or_else(|| {
        anyhow::anyhow!("No active tauri-pilot instance found. Is a Tauri app running?")
    })
}

#[cfg(not(windows))]
fn newest_socket_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
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
        .filter(|p| {
            use std::os::unix::fs::MetadataExt;
            // SAFETY: getuid() has no preconditions.
            let my_uid = unsafe { libc::getuid() };
            std::fs::metadata(p).is_ok_and(|m| m.uid() == my_uid)
        })
        .collect();

    candidates.sort_by_key(|p| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    candidates.pop()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

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

    #[test]
    fn test_with_window_no_window_returns_params_unchanged() {
        let params = Some(json!({"selector": "#btn"}));
        let result = with_window(params.clone(), None);
        assert_eq!(result, params);
    }

    #[test]
    fn test_with_window_none_params_none_window_returns_none() {
        let result = with_window(None, None);
        assert_eq!(result, None);
    }

    #[test]
    fn test_with_window_injects_into_existing_object() {
        let params = Some(json!({"selector": "#btn"}));
        let result = with_window(params, Some("settings"));
        assert_eq!(
            result,
            Some(json!({"selector": "#btn", "window": "settings"}))
        );
    }

    #[test]
    fn test_with_window_none_params_creates_object() {
        let result = with_window(None, Some("main"));
        assert_eq!(result, Some(json!({"window": "main"})));
    }

    #[test]
    #[cfg(unix)]
    #[serial]
    fn test_resolve_socket_finds_socket_in_xdg_runtime_dir() {
        let dir =
            std::env::temp_dir().join(format!("tauri-pilot-xdg-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create xdg test dir");
        let sock = dir.join("tauri-pilot-myapp.sock");
        // Create a dummy file that looks like a socket name.
        std::fs::write(&sock, b"").expect("create dummy socket file");

        // SAFETY: serial attribute serializes tests that touch XDG_RUNTIME_DIR.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &dir) };
        let result = resolve_socket(None);
        unsafe { std::env::remove_var("XDG_RUNTIME_DIR") };

        let _ = std::fs::remove_file(&sock);
        let _ = std::fs::remove_dir(&dir);

        assert_eq!(result.expect("socket found"), sock);
    }

    #[test]
    #[cfg(unix)]
    #[serial]
    fn test_resolve_socket_prefers_xdg_runtime_dir_over_tmp() {
        let dir = std::env::temp_dir().join(format!(
            "tauri-pilot-xdg-precedence-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create xdg test dir");
        let xdg_sock = dir.join("tauri-pilot-xdg.sock");
        let tmp_sock = std::path::PathBuf::from(format!(
            "/tmp/tauri-pilot-newer-tmp-test-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&tmp_sock);
        std::fs::write(&xdg_sock, b"").expect("create xdg socket file");
        std::fs::write(&tmp_sock, b"").expect("create newer tmp socket file");

        // SAFETY: serial attribute serializes tests that touch XDG_RUNTIME_DIR.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &dir) };
        let result = resolve_socket(None);
        unsafe { std::env::remove_var("XDG_RUNTIME_DIR") };

        let _ = std::fs::remove_file(&xdg_sock);
        let _ = std::fs::remove_file(&tmp_sock);
        let _ = std::fs::remove_dir(&dir);

        assert_eq!(result.expect("socket found"), xdg_sock);
    }

    #[test]
    #[cfg(unix)]
    #[serial]
    fn test_resolve_socket_falls_back_to_tmp_when_xdg_unset() {
        let tmp_sock = std::path::PathBuf::from(format!(
            "/tmp/tauri-pilot-fallback-test-{}.sock",
            std::process::id()
        ));
        // Remove then recreate to ensure this file has the newest mtime.
        let _ = std::fs::remove_file(&tmp_sock);
        std::fs::write(&tmp_sock, b"").expect("create dummy socket in /tmp");

        unsafe { std::env::remove_var("XDG_RUNTIME_DIR") };
        let result = resolve_socket(None);

        let _ = std::fs::remove_file(&tmp_sock);

        // Assert the result is a valid tauri-pilot socket path, not an exact path,
        // to avoid flakiness if other sockets exist in /tmp with newer mtime.
        let found = result.expect("socket found in /tmp");
        let name = found
            .file_name()
            .and_then(|n| n.to_str())
            .expect("socket has a filename");
        assert!(
            name.starts_with("tauri-pilot-")
                && std::path::Path::new(name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("sock")),
            "expected a tauri-pilot-*.sock path, got: {found:?}"
        );
    }

    #[test]
    fn test_resolve_socket_returns_explicit_path() {
        let explicit = std::path::PathBuf::from("/tmp/my-explicit.sock");
        let result = resolve_socket(Some(explicit.clone()));
        assert_eq!(result.expect("explicit path returned"), explicit);
    }

    #[test]
    #[cfg(windows)]
    #[serial]
    fn test_resolve_socket_windows_reads_registry() {
        let dir = std::env::temp_dir().join(format!("tauri-pilot-reg-test-{}", std::process::id()));
        let instances_dir = dir.join("tauri-pilot").join("instances");
        std::fs::create_dir_all(&instances_dir).expect("create reg test dir");

        let pipe = r"\\.\pipe\tauri-pilot-testapp";
        // Use the current process id so the liveness check in
        // `resolve_socket_windows` doesn't skip this mock entry as stale.
        std::fs::write(
            instances_dir.join("testapp.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "pid": std::process::id(),
                "pipe": pipe,
                "created_at": 1_745_200_000_u64
            }))
            .expect("serialize mock entry"),
        )
        .expect("write mock registry file");

        unsafe { std::env::set_var("LOCALAPPDATA", &dir) };
        let result = resolve_socket(None);
        unsafe { std::env::remove_var("LOCALAPPDATA") };

        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(result.expect("pipe found"), std::path::PathBuf::from(pipe));
    }

    #[test]
    #[cfg(windows)]
    fn test_resolve_socket_windows_returns_explicit() {
        let explicit = std::path::PathBuf::from(r"\\.\pipe\my-explicit");
        let result = resolve_socket(Some(explicit.clone()));
        assert_eq!(result.expect("explicit path returned"), explicit);
    }

    #[test]
    #[cfg(windows)]
    #[serial]
    fn test_resolve_socket_windows_picks_newest_instance() {
        let dir = std::env::temp_dir().join(format!(
            "tauri-pilot-reg-newest-test-{}",
            std::process::id()
        ));
        let instances_dir = dir.join("tauri-pilot").join("instances");
        std::fs::create_dir_all(&instances_dir).expect("create reg test dir");

        // Both entries must use live PIDs — otherwise the liveness filter in
        // `resolve_socket_windows` would skip them as stale and the test
        // would flake. Using the current process id keeps both "alive".
        let live_pid = std::process::id();
        let old_entry = serde_json::json!({
            "pid": live_pid,
            "pipe": r"\\.\pipe\tauri-pilot-old",
            "created_at": 1000
        });
        let new_entry = serde_json::json!({
            "pid": live_pid,
            "pipe": r"\\.\pipe\tauri-pilot-new",
            "created_at": 2000
        });
        std::fs::write(
            instances_dir.join("old_app.json"),
            serde_json::to_string(&old_entry).expect("serialize old entry"),
        )
        .expect("write mock old instance");
        std::fs::write(
            instances_dir.join("new_app.json"),
            serde_json::to_string(&new_entry).expect("serialize new entry"),
        )
        .expect("write mock new instance");

        unsafe { std::env::set_var("LOCALAPPDATA", &dir) };
        let result = resolve_socket(None);
        unsafe { std::env::remove_var("LOCALAPPDATA") };

        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            result.expect("pipe found"),
            std::path::PathBuf::from(r"\\.\pipe\tauri-pilot-new")
        );
    }

    #[test]
    fn test_read_script_valid() {
        let mut reader = std::io::Cursor::new(b"document.title");
        assert_eq!(
            read_script(&mut reader).expect("read_script succeeds"),
            "document.title"
        );
    }

    #[test]
    fn test_read_script_empty_errors() {
        let mut reader = std::io::Cursor::new(b"");
        let err = read_script(&mut reader).expect_err("empty input rejected");
        assert!(err.to_string().contains("empty or blank"), "got: {err}");
    }

    #[test]
    fn test_read_script_blank_errors() {
        let mut reader = std::io::Cursor::new(b"   \n  ");
        let err = read_script(&mut reader).expect_err("blank input rejected");
        assert!(err.to_string().contains("empty or blank"), "got: {err}");
    }

    // ─── build_wait_params (issue #74) ────────────────────────────────────────

    #[test]
    fn test_build_wait_params_positional_css_selector_routes_to_selector() {
        let p = build_wait_params(Some("#trigger"), None, false, 1000);
        assert_eq!(
            p,
            json!({"selector": "#trigger", "gone": false, "timeout": 1000})
        );
    }

    #[test]
    fn test_build_wait_params_positional_class_selector_routes_to_selector() {
        let p = build_wait_params(Some(".btn-primary"), None, false, 5000);
        assert_eq!(
            p,
            json!({"selector": ".btn-primary", "gone": false, "timeout": 5000})
        );
    }

    #[test]
    fn test_build_wait_params_positional_attr_selector_routes_to_selector() {
        let p = build_wait_params(Some("[data-test=foo]"), None, false, 1000);
        assert_eq!(
            p,
            json!({"selector": "[data-test=foo]", "gone": false, "timeout": 1000})
        );
    }

    #[test]
    fn test_build_wait_params_at_prefix_routes_to_ref() {
        let p = build_wait_params(Some("@e1"), None, false, 1000);
        assert_eq!(p, json!({"ref": "e1", "gone": false, "timeout": 1000}));
    }

    #[test]
    fn test_build_wait_params_explicit_selector_flag_wins_over_positional() {
        let p = build_wait_params(Some("@e1"), Some("#real"), false, 1000);
        assert_eq!(
            p,
            json!({"selector": "#real", "gone": false, "timeout": 1000})
        );
    }

    #[test]
    fn test_build_wait_params_no_target_no_selector_yields_empty_payload() {
        let p = build_wait_params(None, None, true, 2000);
        assert_eq!(p, json!({"gone": true, "timeout": 2000}));
    }

    #[test]
    fn test_build_wait_params_coords_fall_back_to_selector_for_syntax_error() {
        // wait does not act on coordinates; pass through verbatim so
        // `document.querySelector("100,200")` raises a `SyntaxError`
        // instead of the bridge silently waiting on a `MutationObserver`.
        let p = build_wait_params(Some("100,200"), None, false, 1000);
        assert_eq!(
            p,
            json!({"selector": "100,200", "gone": false, "timeout": 1000})
        );
    }

    #[test]
    fn test_build_wait_params_propagates_gone_flag() {
        let p = build_wait_params(Some("#x"), None, true, 500);
        assert_eq!(p, json!({"selector": "#x", "gone": true, "timeout": 500}));
    }

    #[test]
    fn test_build_wait_params_empty_ref_at_alone_drops_to_no_target() {
        // `@` strips to an empty ref. We omit it so the bridge `waitFor`
        // rejects immediately instead of hanging on `MutationObserver`.
        let p = build_wait_params(Some("@"), None, false, 1000);
        assert_eq!(p, json!({"gone": false, "timeout": 1000}));
    }
}
