//! `PilotMcpServer` call helper methods: `target_call`, `call_logs_tool`,
//! `call_network_tool`, `assert_count`, `assert_url`.
//! (`call_drop_tool`, `call_replay_tool` live in `file_ops.rs`.)

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::{Map, Value, json};

use super::super::args::{
    insert_optional_string, insert_optional_usize, optional_bool, required_string, required_u64,
};
use super::super::responses::{tool_error, tool_error_msg, tool_success};
use super::PilotMcpServer;
use crate::target_params;

impl PilotMcpServer {
    pub(crate) async fn target_call(
        &self,
        method: &'static str,
        args: &JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let target = required_string(args, "target")?;
        self.call_app_tool(method, Some(target_params(&target)), window)
            .await
    }

    pub(crate) async fn call_logs_tool(
        &self,
        args: &JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        if optional_bool(args, "clear")?.unwrap_or(false) {
            return self.call_app_tool("console.clear", None, window).await;
        }
        let mut params = Map::new();
        insert_optional_string(&mut params, args, "level")?;
        insert_optional_usize(&mut params, args, "last")?;
        self.call_app_tool("console.getLogs", Some(Value::Object(params)), window)
            .await
    }

    pub(crate) async fn call_network_tool(
        &self,
        args: &JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        if optional_bool(args, "clear")?.unwrap_or(false) {
            return self.call_app_tool("network.clear", None, window).await;
        }
        let mut params = Map::new();
        insert_optional_string(&mut params, args, "filter")?;
        insert_optional_usize(&mut params, args, "last")?;
        if optional_bool(args, "failed")?.unwrap_or(false) {
            params.insert("failedOnly".into(), json!(true));
        }
        self.call_app_tool("network.getRequests", Some(Value::Object(params)), window)
            .await
    }

    pub(crate) async fn assert_count(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let selector = required_string(&args, "selector")?;
        let expected = required_u64(&args, "expected")?;
        let actual = match self
            .call_app("count", Some(json!({"selector": selector})), window)
            .await
        {
            Ok(result) => match result.get("count").and_then(Value::as_u64) {
                Some(value) => value,
                None => return Ok(tool_error_msg("missing 'count' field")),
            },
            Err(err) => return Ok(tool_error(err)),
        };
        if actual == expected {
            Ok(tool_success(json!({"ok": true})))
        } else {
            Ok(tool_error_msg(format!(
                "expected {expected} elements, found {actual}"
            )))
        }
    }

    pub(crate) async fn assert_url(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let expected = required_string(&args, "expected")?;
        let actual = match self.call_app("url", None, window).await {
            Ok(Value::String(actual)) => actual,
            Ok(other) => {
                return Ok(tool_error_msg(format!(
                    "expected string response, got {other}"
                )));
            }
            Err(err) => return Ok(tool_error(err)),
        };
        if actual.contains(&expected) {
            Ok(tool_success(json!({"ok": true})))
        } else {
            Ok(tool_error_msg(format!(
                "URL does not contain \"{expected}\", got \"{actual}\""
            )))
        }
    }
}
