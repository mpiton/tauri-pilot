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

#[cfg(test)]
mod tests {
    use super::startup_banner;

    #[test]
    fn startup_banner_explains_stdio_server() {
        let banner = startup_banner(None, Some("main"));

        assert!(banner.contains("tauri-pilot MCP server"));
        assert!(banner.contains("listening on stdio"));
        assert!(banner.contains("auto-detect on first tool call"));
        assert!(banner.contains("main"));
        assert!(banner.contains("stdout is reserved for MCP JSON-RPC"));
    }
}
