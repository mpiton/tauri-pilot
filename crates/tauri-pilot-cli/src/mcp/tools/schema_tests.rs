//! Tests for tool sort order and schema structure correctness.

use super::*;

#[test]
fn tool_list_is_sorted_alphabetically() {
    let tools = tools();
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    let expected = vec![
        "assert_checked",
        "assert_contains",
        "assert_count",
        "assert_hidden",
        "assert_text",
        "assert_url",
        "assert_value",
        "assert_visible",
        "attrs",
        "check",
        "click",
        "diff",
        "drag",
        "drop",
        "eval",
        "fill",
        "forms",
        "html",
        "ipc",
        "logs",
        "navigate",
        "network",
        "ping",
        "press",
        "record_start",
        "record_status",
        "record_stop",
        "replay",
        "screenshot",
        "scroll",
        "select",
        "snapshot",
        "state",
        "storage_clear",
        "storage_get",
        "storage_list",
        "storage_set",
        "text",
        "title",
        "type",
        "url",
        "value",
        "wait",
        "watch",
        "windows",
    ];
    assert_eq!(names, expected);
}

#[test]
fn schemas_include_window_override() {
    use super::super::schemas::target_schema;
    use serde_json::Value;

    let schema = target_schema();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("schema has properties");
    assert!(properties.contains_key("target"));
    assert!(properties.contains_key("window"));
}

#[test]
fn windows_schema_omits_window_override() {
    use serde_json::Value;

    let schema = core::global_empty_schema();
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("schema has properties");
    assert!(!properties.contains_key("window"));
}
