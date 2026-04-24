//! Handlers for the `observe` tool group: watch, logs, network.

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::json;

use super::super::args::{optional_bool, optional_string, optional_u64};
use super::super::server::PilotMcpServer;

pub(super) async fn dispatch(
    server: &PilotMcpServer,
    name: &str,
    args: &JsonObject,
    window: Option<String>,
) -> Result<CallToolResult, McpError> {
    match name {
        "watch" => {
            let mut watch_params = json!({
                "selector": optional_string(args, "selector")?,
                "timeout": optional_u64(args, "timeout")?.unwrap_or(10_000),
                "stable": optional_u64(args, "stable")?.unwrap_or(300),
            });
            if optional_bool(args, "require_mutation")?.unwrap_or(false) {
                watch_params["requireMutation"] = json!(true);
            }
            server
                .call_app_tool("watch", Some(watch_params), window)
                .await
        }
        "logs" => server.call_logs_tool(args, window).await,
        "network" => server.call_network_tool(args, window).await,
        _ => unreachable!("handlers/mod.rs guarantees prefix match: {name}"),
    }
}
