//! Tool specs for the `core` domain.
//!
//! Tools: `ping`, `windows`, `state`, `snapshot`, `diff`, `screenshot`,
//!        `navigate`, `url`, `title`, `wait`.

use std::sync::Arc;

use rmcp::model::JsonObject;
use serde_json::{Map, Value, json};

use super::super::schemas::{empty_schema, selector_schema};
use super::ToolSpec;
use super::schemas::{diff_schema, navigate_schema, snapshot_schema, wait_schema};

pub(super) fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "ping",
            description: "Check connectivity with the running Tauri app.",
            schema: empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "windows",
            description: "List all open Tauri windows.",
            schema: global_empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "state",
            description: "Get page URL, title, viewport, and scroll state.",
            schema: empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "snapshot",
            description: "Capture an accessibility snapshot of the WebView.",
            schema: snapshot_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "diff",
            description: "Compare the current page to the previous or supplied snapshot.",
            schema: diff_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "screenshot",
            description: "Capture the WebView or an element selector as a PNG data URL.",
            schema: selector_schema,
            read_only: true,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "navigate",
            description: "Navigate the WebView to a URL.",
            schema: navigate_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "url",
            description: "Get the current page URL.",
            schema: empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "title",
            description: "Get the current page title.",
            schema: empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "wait",
            description: "Wait for an element or condition.",
            schema: wait_schema,
            read_only: true,
            destructive: false,
            idempotent: false,
        },
    ]
}

/// Empty schema without the `window` override — used by `windows` tool.
pub(super) fn global_empty_schema() -> Arc<JsonObject> {
    let mut schema = Map::new();
    schema.insert("type".to_owned(), json!("object"));
    schema.insert("properties".to_owned(), Value::Object(Map::new()));
    schema.insert("additionalProperties".to_owned(), json!(false));
    Arc::new(schema)
}
