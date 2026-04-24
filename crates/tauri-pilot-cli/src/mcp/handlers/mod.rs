//! MCP tool dispatcher. Routes a tool name to the appropriate handler.
//!
//! In PR1 Task 5, this is a single free function that takes `&PilotMcpServer`
//! and matches on tool name. PR1 Task 7 will split it into per-domain
//! sibling modules (core, interact, inspect, eval, observe, storage, assert,
//! record).

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, ErrorCode, JsonObject},
};
use serde_json::json;

use super::args::{
    optional_bool, optional_i32, optional_ref, optional_string, optional_u8, optional_u64,
    required_string,
};
use super::responses::invalid_params;
use super::server::PilotMcpServer;
use crate::target_params;

#[allow(clippy::too_many_lines)]
pub(super) async fn call_tool_by_name(
    server: &PilotMcpServer,
    name: &str,
    args: JsonObject,
) -> Result<CallToolResult, McpError> {
    let window = server.window_arg(&args)?;
    match name {
        "ping" => server.call_app_tool("ping", None, window).await,
        "windows" => server.call_app_tool("windows.list", None, None).await,
        "state" => server.call_app_tool("state", None, window).await,
        "snapshot" => {
            server
                .call_app_tool(
                    "snapshot",
                    Some(json!({
                        "interactive": optional_bool(&args, "interactive")?.unwrap_or(false),
                        "selector": optional_string(&args, "selector")?,
                        "depth": optional_u8(&args, "depth")?,
                    })),
                    window,
                )
                .await
        }
        "diff" => {
            let mut params = json!({
                "interactive": optional_bool(&args, "interactive")?.unwrap_or(false),
                "selector": optional_string(&args, "selector")?,
                "depth": optional_u8(&args, "depth")?,
            });
            if let Some(reference) = args.get("reference") {
                params["reference"] = reference.clone();
            }
            server.call_app_tool("diff", Some(params), window).await
        }
        "click" => server.target_call("click", &args, window).await,
        "fill" => {
            let mut params = target_params(&required_string(&args, "target")?);
            params["value"] = json!(required_string(&args, "value")?);
            server.call_app_tool("fill", Some(params), window).await
        }
        "type" => {
            let mut params = target_params(&required_string(&args, "target")?);
            params["text"] = json!(required_string(&args, "text")?);
            server.call_app_tool("type", Some(params), window).await
        }
        "press" => {
            server
                .call_app_tool(
                    "press",
                    Some(json!({"key": required_string(&args, "key")?})),
                    window,
                )
                .await
        }
        "select" => {
            let mut params = target_params(&required_string(&args, "target")?);
            params["value"] = json!(required_string(&args, "value")?);
            server.call_app_tool("select", Some(params), window).await
        }
        "check" => server.target_call("check", &args, window).await,
        "scroll" => {
            server
                .call_app_tool(
                    "scroll",
                    Some(json!({
                        "direction": optional_string(&args, "direction")?.unwrap_or_else(|| "down".to_owned()),
                        "amount": optional_i32(&args, "amount")?,
                        "ref": optional_ref(&args)?,
                    })),
                    window,
                )
                .await
        }
        "drag" => {
            let source = required_string(&args, "source")?;
            let mut params = json!({"source": target_params(&source)});
            let target = optional_string(&args, "target")?;
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
        "drop" => server.call_drop_tool(args, window).await,
        "text" => server.target_call("text", &args, window).await,
        "html" => {
            let params =
                optional_string(&args, "target")?.map(|target| target_params(&target));
            server.call_app_tool("html", params, window).await
        }
        "value" => server.target_call("value", &args, window).await,
        "attrs" => server.target_call("attrs", &args, window).await,
        "eval" => {
            server
                .call_app_tool(
                    "eval",
                    Some(json!({"script": required_string(&args, "script")?})),
                    window,
                )
                .await
        }
        "ipc" => {
            server
                .call_app_tool(
                    "ipc",
                    Some(json!({
                        "command": required_string(&args, "command")?,
                        "args": args.get("args").cloned(),
                    })),
                    window,
                )
                .await
        }
        "screenshot" => {
            server
                .call_app_tool(
                    "screenshot",
                    Some(json!({"selector": optional_string(&args, "selector")?})),
                    window,
                )
                .await
        }
        "navigate" => {
            server
                .call_app_tool(
                    "navigate",
                    Some(json!({"url": required_string(&args, "url")?})),
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
                        "target": optional_string(&args, "target")?,
                        "selector": optional_string(&args, "selector")?,
                        "gone": optional_bool(&args, "gone")?.unwrap_or(false),
                        "timeout": optional_u64(&args, "timeout")?.unwrap_or(10_000),
                    })),
                    window,
                )
                .await
        }
        "watch" => {
            let mut watch_params = json!({
                "selector": optional_string(&args, "selector")?,
                "timeout": optional_u64(&args, "timeout")?.unwrap_or(10_000),
                "stable": optional_u64(&args, "stable")?.unwrap_or(300),
            });
            if optional_bool(&args, "require_mutation")?.unwrap_or(false) {
                watch_params["requireMutation"] = json!(true);
            }
            server.call_app_tool("watch", Some(watch_params), window).await
        }
        "logs" => server.call_logs_tool(&args, window).await,
        "network" => server.call_network_tool(&args, window).await,
        "storage_get" => {
            server
                .call_app_tool(
                    "storage.get",
                    Some(json!({
                        "key": required_string(&args, "key")?,
                        "session": optional_bool(&args, "session")?.unwrap_or(false),
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
                        "key": required_string(&args, "key")?,
                        "value": required_string(&args, "value")?,
                        "session": optional_bool(&args, "session")?.unwrap_or(false),
                    })),
                    window,
                )
                .await
        }
        "storage_list" => {
            server
                .call_app_tool(
                    "storage.list",
                    Some(json!({"session": optional_bool(&args, "session")?.unwrap_or(false)})),
                    window,
                )
                .await
        }
        "storage_clear" => {
            server
                .call_app_tool(
                    "storage.clear",
                    Some(json!({"session": optional_bool(&args, "session")?.unwrap_or(false)})),
                    window,
                )
                .await
        }
        "forms" => {
            let params =
                optional_string(&args, "selector")?.map(|selector| json!({ "selector": selector }));
            server.call_app_tool("forms.dump", params, window).await
        }
        "assert_text" => server.assert_text(args, window, false).await,
        "assert_contains" => server.assert_text(args, window, true).await,
        "assert_visible" => server.assert_bool("visible", args, window, true).await,
        "assert_hidden" => server.assert_bool("visible", args, window, false).await,
        "assert_value" => server.assert_value(args, window).await,
        "assert_count" => server.assert_count(args, window).await,
        "assert_checked" => server.assert_bool("checked", args, window, true).await,
        "assert_url" => server.assert_url(args, window).await,
        "record_start" => server.call_app_tool("record.start", None, window).await,
        "record_stop" => server.call_app_tool("record.stop", None, window).await,
        "record_status" => server.call_app_tool("record.status", None, window).await,
        "replay" => server.call_replay_tool(args, window).await,
        _ => Err(McpError::new(
            ErrorCode::METHOD_NOT_FOUND,
            format!("unknown tool: {name}"),
            None,
        )),
    }
}
