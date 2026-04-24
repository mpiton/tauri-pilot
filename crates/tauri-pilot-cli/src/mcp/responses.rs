use rmcp::{ErrorData as McpError, model::CallToolResult};
use serde_json::{Map, Value, json};

pub(super) fn tool_success(result: Value) -> CallToolResult {
    let mut payload = Map::new();
    payload.insert("result".to_owned(), result);
    CallToolResult::structured(Value::Object(payload))
}

pub(super) fn tool_error(err: impl std::fmt::Display) -> CallToolResult {
    tool_error_msg(err.to_string())
}

pub(super) fn tool_error_msg(message: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({ "error": message.into() }))
}

pub(super) fn invalid_params(message: impl Into<String>) -> McpError {
    McpError::invalid_params(message.into(), None)
}
