use std::{io::IsTerminal, path::Path};

pub(super) fn print_startup_banner(socket: Option<&Path>, window: Option<&str>) {
    if std::io::stdin().is_terminal() && std::io::stderr().is_terminal() {
        eprintln!("{}", startup_banner(socket, window));
    }
}

pub(super) fn startup_banner(socket: Option<&Path>, window: Option<&str>) -> String {
    let socket = socket.map_or_else(
        || "auto-detect on first tool call".to_owned(),
        |path| path.display().to_string(),
    );
    let window = window.unwrap_or("default app window");

    format!(
        r"
tauri-pilot MCP server

Status : listening on stdio
Socket : {socket}
Window : {window}

stdout is reserved for MCP JSON-RPC.
Configure your MCP client to launch this command instead of typing requests here.
"
    )
}
