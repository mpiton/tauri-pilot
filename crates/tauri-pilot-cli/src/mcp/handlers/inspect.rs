//! Handlers for the `inspect` tool group: text, html, value, attrs.

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};

use super::super::args::optional_string;
use super::super::server::PilotMcpServer;
use crate::target_params;

pub(super) async fn dispatch(
    server: &PilotMcpServer,
    name: &str,
    args: &JsonObject,
    window: Option<String>,
) -> Result<CallToolResult, McpError> {
    match name {
        "text" => server.target_call("text", args, window).await,
        "html" => {
            let params = optional_string(args, "target")?.map(|target| target_params(&target));
            server.call_app_tool("html", params, window).await
        }
        "value" => server.target_call("value", args, window).await,
        "attrs" => server.target_call("attrs", args, window).await,
        _ => unreachable!("handlers/mod.rs guarantees prefix match: {name}"),
    }
}
