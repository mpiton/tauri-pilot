//! Handlers for the `record` tool group: `record_start`, `record_stop`,
//! `record_status`, `replay`.

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};

use super::super::server::PilotMcpServer;

pub(super) async fn dispatch(
    server: &PilotMcpServer,
    name: &str,
    args: &JsonObject,
    window: Option<String>,
) -> Result<CallToolResult, McpError> {
    match name {
        "record_start" => server.call_app_tool("record.start", None, window).await,
        "record_stop" => server.call_app_tool("record.stop", None, window).await,
        "record_status" => server.call_app_tool("record.status", None, window).await,
        "replay" => server.call_replay_tool(args.clone(), window).await,
        _ => unreachable!("handlers/mod.rs guarantees prefix match: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value, json};

    use super::super::super::server::PilotMcpServer;
    use super::super::call_tool_by_name;

    #[tokio::test]
    async fn replay_export_does_not_connect_to_socket() {
        let recording = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-replay-test-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &recording,
            r#"[{"action":"click","timestamp":0,"ref":"e1"}]"#,
        )
        .expect("write recording");

        let missing_socket = std::env::temp_dir().join(format!(
            "tauri-pilot-mcp-missing-{}.sock",
            std::process::id()
        ));
        let pilot = PilotMcpServer::new(Some(missing_socket), None);
        let mut args = Map::new();
        args.insert("path".to_owned(), json!(recording.display().to_string()));
        args.insert("export".to_owned(), json!("sh"));

        let result = call_tool_by_name(&pilot, "replay", args)
            .await
            .expect("tool call succeeds");

        assert_eq!(result.is_error, Some(false));
        let script = result
            .structured_content
            .as_ref()
            .and_then(|content| content.get("result"))
            .and_then(Value::as_str)
            .expect("script result");
        assert!(script.starts_with("#!/bin/bash"));
        assert!(script.contains("tauri-pilot click @e1"));

        let _ = std::fs::remove_file(&recording);
    }
}
