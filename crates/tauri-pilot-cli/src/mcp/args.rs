use rmcp::{ErrorData as McpError, model::JsonObject};
use serde_json::{Map, Value, json};

use super::responses::invalid_params;

pub(super) fn required_string(args: &JsonObject, name: &str) -> Result<String, McpError> {
    args.get(name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid_params(format!("'{name}' is required and must be a string")))
}

pub(super) fn optional_string(args: &JsonObject, name: &str) -> Result<Option<String>, McpError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(invalid_params(format!("'{name}' must be a string"))),
    }
}

pub(super) fn required_u64(args: &JsonObject, name: &str) -> Result<u64, McpError> {
    args.get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_params(format!("'{name}' is required and must be an integer")))
}

pub(super) fn optional_u64(args: &JsonObject, name: &str) -> Result<Option<u64>, McpError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| invalid_params(format!("'{name}' must be an integer"))),
    }
}

pub(super) fn optional_usize(args: &JsonObject, name: &str) -> Result<Option<usize>, McpError> {
    optional_u64(args, name)?
        .map(|value| {
            usize::try_from(value)
                .map_err(|_| invalid_params(format!("'{name}' is out of range for usize")))
        })
        .transpose()
}

pub(super) fn optional_i32(args: &JsonObject, name: &str) -> Result<Option<i32>, McpError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let parsed = value
                .as_i64()
                .ok_or_else(|| invalid_params(format!("'{name}' must be an integer")))?;
            i32::try_from(parsed)
                .map(Some)
                .map_err(|_| invalid_params(format!("'{name}' is out of range for i32")))
        }
    }
}

pub(super) fn optional_u8(args: &JsonObject, name: &str) -> Result<Option<u8>, McpError> {
    match optional_u64(args, name)? {
        Some(value) => u8::try_from(value)
            .map(Some)
            .map_err(|_| invalid_params(format!("'{name}' is out of range for u8"))),
        None => Ok(None),
    }
}

pub(super) fn optional_bool(args: &JsonObject, name: &str) -> Result<Option<bool>, McpError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| invalid_params(format!("'{name}' must be a boolean"))),
    }
}

pub(super) fn optional_ref(args: &JsonObject) -> Result<Option<String>, McpError> {
    match optional_string(args, "ref")? {
        Some(value) => Ok(Some(value.trim_start_matches('@').to_owned())),
        None => Ok(None),
    }
}

pub(super) fn required_string_array(
    args: &JsonObject,
    name: &str,
) -> Result<Vec<String>, McpError> {
    let values = args
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_params(format!("'{name}' is required and must be an array")))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| invalid_params(format!("'{name}' must contain only strings")))
        })
        .collect()
}

pub(super) fn insert_optional_string(
    params: &mut Map<String, Value>,
    args: &JsonObject,
    name: &str,
) -> Result<(), McpError> {
    if let Some(value) = optional_string(args, name)? {
        params.insert(name.to_owned(), json!(value));
    }
    Ok(())
}

pub(super) fn insert_optional_usize(
    params: &mut Map<String, Value>,
    args: &JsonObject,
    name: &str,
) -> Result<(), McpError> {
    if let Some(value) = optional_usize(args, name)? {
        params.insert(name.to_owned(), json!(value));
    }
    Ok(())
}
