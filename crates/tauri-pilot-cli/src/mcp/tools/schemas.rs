//! Domain-specific schema builders used by per-domain tool specs.
//!
//! Generic schemas shared across ≥2 top-level modules live in `mcp/schemas.rs`.
//! These are specific to the tool domains and would clutter the global schemas.

use std::sync::Arc;

use rmcp::model::JsonObject;

use super::super::schemas::{
    any_prop, array_string_prop, bool_prop, enum_prop, integer_prop, object_schema, props,
    string_prop,
};

// ── core ──────────────────────────────────────────────────────────────────────

pub(super) fn snapshot_schema() -> Arc<JsonObject> {
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

pub(super) fn diff_schema() -> Arc<JsonObject> {
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

pub(super) fn navigate_schema() -> Arc<JsonObject> {
    object_schema(
        props([("url", string_prop("URL to navigate to."))]),
        &["url"],
    )
}

pub(super) fn wait_schema() -> Arc<JsonObject> {
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

// ── interact ──────────────────────────────────────────────────────────────────

pub(super) fn type_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "target",
                string_prop("Element ref, CSS selector, or x,y coordinates."),
            ),
            ("text", string_prop("Text to type.")),
        ]),
        &["target", "text"],
    )
}

pub(super) fn press_schema() -> Arc<JsonObject> {
    object_schema(
        props([("key", string_prop("Keyboard key to press."))]),
        &["key"],
    )
}

pub(super) fn scroll_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "direction",
                enum_prop("Direction to scroll.", &["up", "down", "left", "right"]),
            ),
            ("amount", integer_prop("Pixel amount to scroll.")),
            (
                "ref",
                string_prop("Optional element ref, with or without @."),
            ),
        ]),
        &[],
    )
}

pub(super) fn drag_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "source",
                string_prop("Source element ref, selector, or coordinates."),
            ),
            (
                "target",
                string_prop("Destination element ref, selector, or coordinates."),
            ),
            (
                "offset",
                any_prop("Optional offset object such as {\"x\": 0, \"y\": 100}."),
            ),
        ]),
        &["source"],
    )
}

pub(super) fn drop_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "target",
                string_prop("Drop target ref, selector, or coordinates."),
            ),
            ("files", array_string_prop("Local file paths to drop.")),
        ]),
        &["target", "files"],
    )
}

// ── eval ──────────────────────────────────────────────────────────────────────

pub(super) fn eval_schema() -> Arc<JsonObject> {
    object_schema(
        props([("script", string_prop("JavaScript to evaluate."))]),
        &["script"],
    )
}

pub(super) fn ipc_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("command", string_prop("Tauri IPC command name.")),
            (
                "args",
                any_prop("Optional JSON object of command arguments."),
            ),
        ]),
        &["command"],
    )
}

// ── observe ───────────────────────────────────────────────────────────────────

pub(super) fn watch_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "selector",
                string_prop("CSS selector to scope observation."),
            ),
            ("timeout", integer_prop("Timeout in milliseconds.")),
            ("stable", integer_prop("Stability window in milliseconds.")),
            (
                "require_mutation",
                bool_prop(
                    "Defer the stability timer until at least one DOM mutation occurs. \
                     Rejects on timeout when nothing changed.",
                ),
            ),
        ]),
        &[],
    )
}

pub(super) fn logs_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            (
                "level",
                enum_prop(
                    "Optional log level filter.",
                    &["log", "info", "warn", "error"],
                ),
            ),
            ("last", integer_prop("Return only the last N log entries.")),
            (
                "clear",
                bool_prop("Clear the log buffer instead of reading it."),
            ),
        ]),
        &[],
    )
}

pub(super) fn network_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("filter", string_prop("Optional URL substring filter.")),
            ("failed", bool_prop("Only return failed requests.")),
            ("last", integer_prop("Return only the last N requests.")),
            (
                "clear",
                bool_prop("Clear the request buffer instead of reading it."),
            ),
        ]),
        &[],
    )
}

// ── storage ───────────────────────────────────────────────────────────────────

pub(super) fn storage_get_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("key", string_prop("Storage key.")),
            (
                "session",
                bool_prop("Use sessionStorage instead of localStorage."),
            ),
        ]),
        &["key"],
    )
}

pub(super) fn storage_set_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("key", string_prop("Storage key.")),
            ("value", string_prop("Storage value.")),
            (
                "session",
                bool_prop("Use sessionStorage instead of localStorage."),
            ),
        ]),
        &["key", "value"],
    )
}

// ── assert ────────────────────────────────────────────────────────────────────

pub(super) fn assert_count_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("selector", string_prop("CSS selector to count.")),
            ("expected", integer_prop("Expected element count.")),
        ]),
        &["selector", "expected"],
    )
}

// ── record ────────────────────────────────────────────────────────────────────

pub(super) fn replay_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("path", string_prop("Path to a recording JSON file.")),
            (
                "export",
                enum_prop("Export format instead of replaying.", &["sh"]),
            ),
        ]),
        &["path"],
    )
}
