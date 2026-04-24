//! Schema builders for the `core` tool group: snapshot, diff, navigate, wait.

use std::sync::Arc;

use rmcp::model::JsonObject;

use super::super::super::schemas::{
    any_prop, bool_prop, integer_prop, object_schema, props, string_prop,
};

pub(in super::super) fn snapshot_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "interactive",
                bool_prop("Only include interactive elements."),
            ),
            (
                "selector",
                string_prop("CSS selector to scope the snapshot."),
            ),
            ("depth", integer_prop("Maximum traversal depth.")),
        ]),
        &[],
    )
}

pub(in super::super) fn diff_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "interactive",
                bool_prop("Only include interactive elements."),
            ),
            (
                "selector",
                string_prop("CSS selector to scope the new snapshot."),
            ),
            ("depth", integer_prop("Maximum traversal depth.")),
            (
                "reference",
                any_prop("Optional prior snapshot object to compare against."),
            ),
        ]),
        &[],
    )
}

pub(in super::super) fn navigate_schema() -> Arc<JsonObject> {
    object_schema(
        props([("url", string_prop("URL to navigate to."))]),
        &["url"],
    )
}

pub(in super::super) fn wait_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "target",
                string_prop("Element ref, CSS selector, or x,y coordinates."),
            ),
            ("selector", string_prop("CSS selector to wait for.")),
            ("gone", bool_prop("Wait for the element to disappear.")),
            ("timeout", integer_prop("Timeout in milliseconds.")),
        ]),
        &[],
    )
}
