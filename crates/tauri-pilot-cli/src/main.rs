mod cli;
mod client;
mod output;
mod protocol;
mod style;

use anyhow::Result;
use base64::Engine;
use clap::Parser;
use serde_json::json;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

use cli::{Cli, Command, Target, parse_target};
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
    } else {
        output::format_text(&result);
    }

    Ok(())
}

async fn run_command(client: &mut Client, command: Command) -> Result<serde_json::Value> {
    match command {
        Command::Ping => client.call("ping", None).await,
        Command::State => client.call("state", None).await,
        Command::Snapshot {
            interactive,
            selector,
            depth,
        } => {
            client
                .call(
                    "snapshot",
                    Some(json!({
                        "interactive": interactive,
                        "selector": selector,
                        "depth": depth,
                    })),
                )
                .await
        }
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
        Command::Ipc { command, args } => {
            let parsed_args: Option<serde_json::Value> =
                args.as_deref().map(serde_json::from_str).transpose()?;
            client
                .call(
                    "ipc",
                    Some(json!({"command": command, "args": parsed_args})),
                )
                .await
        }
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
    }
}

fn target_params(raw: &str) -> serde_json::Value {
    match parse_target(raw) {
        Target::Ref(r) => json!({"ref": r}),
        Target::Selector(s) => json!({"selector": s}),
        Target::Coords(x, y) => json!({"x": x, "y": y}),
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
