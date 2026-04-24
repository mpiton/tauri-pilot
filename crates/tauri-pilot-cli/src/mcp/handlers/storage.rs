//! Handlers for the `storage` tool group: `storage_get`, `storage_set`,
//! `storage_list`, `storage_clear`, `forms`.

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::json;

use super::super::args::{optional_bool, optional_string, required_string};
use super::super::server::PilotMcpServer;

pub(super) async fn dispatch(
    server: &PilotMcpServer,
    name: &str,
    args: &JsonObject,
    window: Option<String>,
) -> Result<CallToolResult, McpError> {
    match name {
        "storage_get" => {
            server
                .call_app_tool(
                    "storage.get",
                    Some(json!({
                        "key": required_string(args, "key")?,
                        "session": optional_bool(args, "session")?.unwrap_or(false),
                    })),
                    window,
                )
                .await
        }
        "storage_set" => {
            server
                .call_app_tool(
                    "storage.set",
                    Some(json!({
                        "key": required_string(args, "key")?,
                        "value": required_string(args, "value")?,
                        "session": optional_bool(args, "session")?.unwrap_or(false),
                    })),
                    window,
                )
                .await
        }
        "storage_list" => {
            server
                .call_app_tool(
                    "storage.list",
                    Some(json!({"session": optional_bool(args, "session")?.unwrap_or(false)})),
                    window,
                )
                .await
        }
        "storage_clear" => {
            server
                .call_app_tool(
                    "storage.clear",
                    Some(json!({"session": optional_bool(args, "session")?.unwrap_or(false)})),
                    window,
                )
                .await
        }
        "forms" => {
            let params =
                optional_string(args, "selector")?.map(|selector| json!({ "selector": selector }));
            server.call_app_tool("forms.dump", params, window).await
        }
        _ => unreachable!("handlers/mod.rs guarantees prefix match: {name}"),
    }
}
