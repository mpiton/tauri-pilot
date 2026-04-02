mod cli;
mod client;
#[allow(dead_code)]
mod protocol;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

use cli::{Cli, Command};
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

    match args.command {
        Command::Ping => {
            let result = client.call("ping", None).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("ok");
            }
        }
    }

    Ok(())
}

/// Resolve the socket path from explicit arg, env var, or auto-detection.
fn resolve_socket(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }

    // Auto-detect: find the most recent tauri-pilot-*.sock in /tmp
    let mut candidates: Vec<PathBuf> = std::fs::read_dir("/tmp")
        .into_iter()
        .flatten()
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
