use std::sync::Arc;

use rmcp::model::JsonObject;
use serde_json::{Map, Value, json};

pub(super) fn object_schema(
    mut properties: Map<String, Value>,
    required: &[&str],
) -> Arc<JsonObject> {
    properties.insert(
        "window".to_owned(),
        string_prop("Optional Tauri window label overriding the MCP server default."),
    );
    let mut schema = Map::new();
    schema.insert("type".to_owned(), json!("object"));
    schema.insert("properties".to_owned(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".to_owned(), json!(required));
    }
    schema.insert("additionalProperties".to_owned(), json!(false));
    Arc::new(schema)
}

pub(super) fn props<const N: usize>(properties: [(&str, Value); N]) -> Map<String, Value> {
    properties
        .into_iter()
        .map(|(name, schema)| (name.to_owned(), schema))
        .collect()
}

pub(super) fn string_prop(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

pub(super) fn bool_prop(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}

pub(super) fn integer_prop(description: &str) -> Value {
    json!({"type": "integer", "description": description})
}

pub(super) fn array_string_prop(description: &str) -> Value {
    json!({"type": "array", "items": {"type": "string"}, "description": description})
}

pub(super) fn any_prop(description: &str) -> Value {
    json!({"description": description})
}

pub(super) fn enum_prop(description: &str, values: &[&str]) -> Value {
    json!({"type": "string", "enum": values, "description": description})
}
