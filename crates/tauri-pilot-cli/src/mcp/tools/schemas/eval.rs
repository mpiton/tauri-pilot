//! Schema builders for the `eval` tool group: eval, ipc.

use std::sync::Arc;

use rmcp::model::JsonObject;

use super::super::super::schemas::{any_prop, object_schema, props, string_prop};

pub(in super::super) fn eval_schema() -> Arc<JsonObject> {
    object_schema(
        props([("script", string_prop("JavaScript to evaluate."))]),
        &["script"],
    )
}

pub(in super::super) fn ipc_schema() -> Arc<JsonObject> {
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
