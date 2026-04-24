//! Handlers for the `core` tool group: ping, windows, state, snapshot, diff,
//! screenshot, navigate, url, title, wait.

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::json;

use super::super::args::{
    optional_bool, optional_string, optional_u8, optional_u64, required_string,
};
use super::super::server::PilotMcpServer;

pub(super) async fn dispatch(
    server: &PilotMcpServer,
    name: &str,
    args: &JsonObject,
    window: Option<String>,
) -> Result<CallToolResult, McpError> {
    match name {
        "ping" => server.call_app_tool("ping", None, window).await,
        "windows" => server.call_app_tool("windows.list", None, None).await,
        "state" => server.call_app_tool("state", None, window).await,
        "snapshot" => {
            server
                .call_app_tool(
                    "snapshot",
                    Some(json!({
                        "interactive": optional_bool(args, "interactive")?.unwrap_or(false),
                        "selector": optional_string(args, "selector")?,
                        "depth": optional_u8(args, "depth")?,
                    })),
                    window,
                )
                .await
        }
        "diff" => {
            let mut params = json!({
                "interactive": optional_bool(args, "interactive")?.unwrap_or(false),
                "selector": optional_string(args, "selector")?,
                "depth": optional_u8(args, "depth")?,
            });
            if let Some(reference) = args.get("reference") {
                params["reference"] = reference.clone();
            }
            server.call_app_tool("diff", Some(params), window).await
        }
        "screenshot" => {
            server
                .call_app_tool(
                    "screenshot",
                    Some(json!({"selector": optional_string(args, "selector")?})),
                    window,
                )
                .await
        }
        "navigate" => {
            server
                .call_app_tool(
                    "navigate",
                    Some(json!({"url": required_string(args, "url")?})),
                    window,
                )
                .await
        }
        "url" => server.call_app_tool("url", None, window).await,
        "title" => server.call_app_tool("title", None, window).await,
        "wait" => {
            server
                .call_app_tool(
                    "wait",
                    Some(json!({
                        "target": optional_string(args, "target")?,
                        "selector": optional_string(args, "selector")?,
                        "gone": optional_bool(args, "gone")?.unwrap_or(false),
                        "timeout": optional_u64(args, "timeout")?.unwrap_or(10_000),
                    })),
                    window,
                )
                .await
        }
        _ => unreachable!("handlers/mod.rs guarantees prefix match: {name}"),
    }
}
