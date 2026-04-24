//! Schema builders for the `storage` tool group: `storage_get`, `storage_set`.

use std::sync::Arc;

use rmcp::model::JsonObject;

use super::super::super::schemas::{bool_prop, object_schema, props, string_prop};

pub(in super::super) fn storage_get_schema() -> Arc<JsonObject> {
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

pub(in super::super) fn storage_set_schema() -> Arc<JsonObject> {
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
