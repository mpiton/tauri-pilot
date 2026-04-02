use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tauri-pilot", about = "Interactive testing CLI for Tauri apps")]
pub(crate) struct Cli {
    /// Socket path (auto-detected if omitted).
    #[arg(long, env = "TAURI_PILOT_SOCKET")]
    pub socket: Option<PathBuf>,

    /// Output JSON instead of text.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Check connectivity with a running Tauri app.
    Ping,
}
