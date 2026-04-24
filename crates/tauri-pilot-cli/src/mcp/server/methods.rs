//! Additional `impl PilotMcpServer` methods: the assert helpers, call_*_tool
//! helpers, and `target_call`. Kept in a separate file so that `server/mod.rs`
//! stays under 150 lines.

use std::path::PathBuf;

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::{Map, Value, json};

use super::super::args::{
    insert_optional_string, insert_optional_usize, optional_bool, optional_string, required_string,
    required_string_array, required_u64,
};
use super::super::responses::{invalid_params, tool_error, tool_error_msg, tool_success};
use super::PilotMcpServer;
use crate::{export_replay_file, run_drop_command, run_replay_command, target_params};

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

    pub(crate) async fn call_drop_tool(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let target = required_string(&args, "target")?;
        let files: Vec<PathBuf> = required_string_array(&args, "files")?
            .into_iter()
            .map(PathBuf::from)
            .collect();
        if files.is_empty() {
            return Err(invalid_params("'files' must contain at least one path"));
        }
        let mut client = match self.connect_client().await {
            Ok(client) => client,
            Err(err) => return Ok(tool_error(err)),
        };
        Ok(
            match run_drop_command(&mut client, &target, files, window.as_deref()).await {
                Ok(result) => tool_success(result),
                Err(err) => tool_error(err),
            },
        )
    }

    pub(crate) async fn call_replay_tool(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let path = PathBuf::from(required_string(&args, "path")?);
        let export = optional_string(&args, "export")?;
        if let Some(export) = export.as_deref() {
            return Ok(match export_replay_file(&path, export) {
                Ok(result) => tool_success(result),
                Err(err) => tool_error(err),
            });
        }
        let mut client = match self.connect_client().await {
            Ok(client) => client,
            Err(err) => return Ok(tool_error(err)),
        };
        Ok(
            match run_replay_command(&mut client, &path, None, window.as_deref()).await {
                Ok(result) => tool_success(result),
                Err(err) => tool_error(err),
            },
        )
    }

    pub(crate) async fn assert_text(
        &self,
        args: JsonObject,
        window: Option<String>,
        contains: bool,
    ) -> Result<CallToolResult, McpError> {
        let expected = required_string(&args, "expected")?;
        let target = required_string(&args, "target")?;
        let actual = match self
            .call_app("text", Some(target_params(&target)), window)
            .await
        {
            Ok(Value::String(actual)) => actual,
            Ok(other) => {
                return Ok(tool_error_msg(format!(
                    "expected string response, got {other}"
                )));
            }
            Err(err) => return Ok(tool_error(err)),
        };
        let passed = if contains {
            actual.contains(&expected)
        } else {
            actual == expected
        };
        if passed {
            Ok(tool_success(json!({"ok": true})))
        } else {
            let message = if contains {
                format!("text does not contain \"{expected}\", got \"{actual}\"")
            } else {
                format!("expected text \"{expected}\", got \"{actual}\"")
            };
            Ok(tool_error_msg(message))
        }
    }

    pub(crate) async fn assert_value(
        &self,
        args: JsonObject,
        window: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let expected = required_string(&args, "expected")?;
        let target = required_string(&args, "target")?;
        let actual = match self
            .call_app("value", Some(target_params(&target)), window)
            .await
        {
            Ok(Value::String(actual)) => actual,
            Ok(other) => {
                return Ok(tool_error_msg(format!(
                    "expected string response, got {other}"
                )));
            }
            Err(err) => return Ok(tool_error(err)),
        };
        if actual == expected {
            Ok(tool_success(json!({"ok": true})))
        } else {
            Ok(tool_error_msg(format!(
                "expected value \"{expected}\", got \"{actual}\""
            )))
        }
    }

    pub(crate) async fn assert_bool(
        &self,
        method: &'static str,
        args: JsonObject,
        window: Option<String>,
        expected: bool,
    ) -> Result<CallToolResult, McpError> {
        let target = required_string(&args, "target")?;
        let field = method;
        let actual = match self
            .call_app(method, Some(target_params(&target)), window)
            .await
        {
            Ok(result) => match result.get(field).and_then(Value::as_bool) {
                Some(value) => value,
                None => return Ok(tool_error_msg(format!("missing boolean field '{field}'"))),
            },
            Err(err) => return Ok(tool_error(err)),
        };
        if actual == expected {
            Ok(tool_success(json!({"ok": true})))
        } else if method == "visible" && expected {
            Ok(tool_error_msg("element is not visible"))
        } else if method == "visible" {
            Ok(tool_error_msg("element is visible"))
        } else if method == "checked" && expected {
            Ok(tool_error_msg("element is not checked"))
        } else if method == "checked" {
            Ok(tool_error_msg("element is checked"))
        } else {
            Ok(tool_error_msg(format!(
                "element '{method}' state mismatch: expected {expected}"
            )))
        }
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
