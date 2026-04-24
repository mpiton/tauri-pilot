//! MCP tool dispatcher. Routes a tool name to the per-domain handler module
//! that owns it. Each domain handler returns `Result<CallToolResult, McpError>`.

mod assert;
mod core;
mod eval;
mod inspect;
mod interact;
mod observe;
mod record;
mod storage;

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};

use super::responses::invalid_params;
use super::server::PilotMcpServer;

pub(super) async fn call_tool_by_name(
    server: &PilotMcpServer,
    name: &str,
    args: JsonObject,
) -> Result<CallToolResult, McpError> {
    let window = server.window_arg(&args)?;
    match name {
        // core
        "ping" | "windows" | "state" | "snapshot" | "diff" | "screenshot" | "navigate" | "url"
        | "title" | "wait" => core::dispatch(server, name, &args, window).await,
        // interact
        "click" | "fill" | "type" | "press" | "select" | "check" | "scroll" | "drag" | "drop" => {
            interact::dispatch(server, name, &args, window).await
        }
        // inspect
        "text" | "html" | "value" | "attrs" => inspect::dispatch(server, name, &args, window).await,
        // eval
        "eval" | "ipc" => eval::dispatch(server, name, &args, window).await,
        // observe
        "watch" | "logs" | "network" => observe::dispatch(server, name, &args, window).await,
        // storage
        "storage_get" | "storage_set" | "storage_list" | "storage_clear" | "forms" => {
            storage::dispatch(server, name, &args, window).await
        }
        // assert
        n if n.starts_with("assert_") => assert::dispatch(server, name, &args, window).await,
        // record
        "record_start" | "record_stop" | "record_status" | "replay" => {
            record::dispatch(server, name, &args, window).await
        }
        other => Err(invalid_params(format!("unknown tool: {other}"))),
    }
}
