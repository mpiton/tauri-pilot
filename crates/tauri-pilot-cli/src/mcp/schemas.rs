use std::sync::Arc;

use rmcp::model::JsonObject;
use serde_json::{Map, Value, json};

/// Empty schema — no parameters. Used by `ping`, `state`, `title`, `url`,
/// `record_start`, `record_stop`, `record_status`.
pub(super) fn empty_schema() -> Arc<JsonObject> {
    object_schema(Map::new(), &[])
}

/// Schema with a required `target` field.
/// Used by `attrs`, `check`, `click`, `text`, `value`,
/// `assert_checked`, `assert_hidden`, `assert_visible`.
pub(super) fn target_schema() -> Arc<JsonObject> {
    object_schema(
        props([(
            "target",
            string_prop("Element ref, CSS selector, or x,y coordinates."),
        )]),
        &["target"],
    )
}

/// Schema with an optional `target` field. Used by `html`.
pub(super) fn optional_target_schema() -> Arc<JsonObject> {
    object_schema(
        props([(
            "target",
            string_prop("Optional element ref, CSS selector, or x,y coordinates."),
        )]),
        &[],
    )
}

/// Schema with required `target` and `expected`. Used by
/// `assert_contains`, `assert_text`, `assert_value`.
pub(super) fn expected_target_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "target",
                string_prop("Element ref, CSS selector, or x,y coordinates."),
            ),
            ("expected", string_prop("Expected value or substring.")),
        ]),
        &["target", "expected"],
    )
}

/// Schema with a required `expected` field only. Used by `assert_url`.
pub(super) fn expected_schema() -> Arc<JsonObject> {
    object_schema(
        props([("expected", string_prop("Expected value or substring."))]),
        &["expected"],
    )
}

/// Schema with an optional `selector` field. Used by `forms`, `screenshot`.
pub(super) fn selector_schema() -> Arc<JsonObject> {
    object_schema(
        props([("selector", string_prop("Optional CSS selector."))]),
        &[],
    )
}

/// Schema with required `target` + `value`. Used by `fill`, `select`.
pub(super) fn fill_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "target",
                string_prop("Element ref, CSS selector, or x,y coordinates."),
            ),
            ("value", string_prop("Value to set.")),
        ]),
        &["target", "value"],
    )
}

/// Schema with an optional `session` bool. Used by `storage_clear`, `storage_list`.
pub(super) fn session_schema() -> Arc<JsonObject> {
    object_schema(
        props([(
            "session",
            bool_prop("Use sessionStorage instead of localStorage."),
        )]),
        &[],
    )
}

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
