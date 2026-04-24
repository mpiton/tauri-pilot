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
