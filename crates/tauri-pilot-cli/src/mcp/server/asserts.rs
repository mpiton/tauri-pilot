//! `PilotMcpServer` assert helper methods: `assert_text`, `assert_value`,
//! `assert_bool`, `assert_count`. (`assert_url` lives in `calls.rs`.)

use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde_json::{Value, json};

use super::super::args::required_string;
use super::super::responses::{tool_error, tool_error_msg, tool_success};
use super::PilotMcpServer;
use crate::target_params;

impl PilotMcpServer {
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
}
