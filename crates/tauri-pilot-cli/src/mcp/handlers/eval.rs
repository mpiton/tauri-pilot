//! Handlers for the `eval` tool group: eval, ipc.

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::json;

use super::super::args::required_string;
use super::super::server::PilotMcpServer;

pub(super) async fn dispatch(
    server: &PilotMcpServer,
    name: &str,
    args: &JsonObject,
    window: Option<String>,
) -> Result<CallToolResult, McpError> {
    match name {
        "eval" => {
            server
                .call_app_tool(
                    "eval",
                    Some(json!({"script": required_string(args, "script")?})),
                    window,
                )
                .await
        }
        "ipc" => {
            server
                .call_app_tool(
                    "ipc",
                    Some(json!({
                        "command": required_string(args, "command")?,
                        "args": args.get("args").cloned(),
                    })),
                    window,
                )
                .await
        }
        _ => unreachable!("handlers/mod.rs guarantees prefix match: {name}"),
    }
}
