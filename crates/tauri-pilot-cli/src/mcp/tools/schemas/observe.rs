//! Schema builders for the `observe` tool group: watch, logs, network.

use std::sync::Arc;

use rmcp::model::JsonObject;

use super::super::super::schemas::{
    bool_prop, enum_prop, integer_prop, object_schema, props, string_prop,
};

pub(in super::super) fn watch_schema() -> Arc<JsonObject> {
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

pub(in super::super) fn logs_schema() -> Arc<JsonObject> {
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

pub(in super::super) fn network_schema() -> Arc<JsonObject> {
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
