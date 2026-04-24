//! Handlers for the `interact` tool group: click, fill, type, press, select,
//! check, scroll, drag, drop.

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::json;

use super::super::args::{optional_i32, optional_ref, optional_string, required_string};
use super::super::responses::invalid_params;
use super::super::server::PilotMcpServer;
use crate::target_params;

pub(super) async fn dispatch(
    server: &PilotMcpServer,
    name: &str,
    args: &JsonObject,
    window: Option<String>,
) -> Result<CallToolResult, McpError> {
    match name {
        "click" => server.target_call("click", args, window).await,
        "fill" => {
            let mut params = target_params(&required_string(args, "target")?);
            params["value"] = json!(required_string(args, "value")?);
            server.call_app_tool("fill", Some(params), window).await
        }
        "type" => {
            let mut params = target_params(&required_string(args, "target")?);
            params["text"] = json!(required_string(args, "text")?);
            server.call_app_tool("type", Some(params), window).await
        }
        "press" => {
            server
                .call_app_tool(
                    "press",
                    Some(json!({"key": required_string(args, "key")?})),
                    window,
                )
                .await
        }
        "select" => {
            let mut params = target_params(&required_string(args, "target")?);
            params["value"] = json!(required_string(args, "value")?);
            server.call_app_tool("select", Some(params), window).await
        }
        "check" => server.target_call("check", args, window).await,
        "scroll" => {
            server
                .call_app_tool(
                    "scroll",
                    Some(json!({
                        "direction": optional_string(args, "direction")?.unwrap_or_else(|| "down".to_owned()),
                        "amount": optional_i32(args, "amount")?,
                        "ref": optional_ref(args)?,
                    })),
                    window,
                )
                .await
        }
        "drag" => {
            let source = required_string(args, "source")?;
            let mut params = json!({"source": target_params(&source)});
            let target = optional_string(args, "target")?;
            let offset = args.get("offset").cloned();
            match (target, offset) {
                (Some(_), Some(_)) => {
                    return Err(invalid_params(
                        "drag accepts either 'target' or 'offset', not both",
                    ));
                }
                (None, None) => {
                    return Err(invalid_params("drag requires either 'target' or 'offset'"));
                }
                (Some(target), None) => {
                    params["target"] = target_params(&target);
                }
                (None, Some(offset)) => {
                    params["offset"] = offset;
                }
            }
            server.call_app_tool("drag", Some(params), window).await
        }
        "drop" => server.call_drop_tool(args.clone(), window).await,
        _ => unreachable!("handlers/mod.rs guarantees prefix match: {name}"),
    }
}
