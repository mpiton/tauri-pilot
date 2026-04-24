//! Schema builders for the `record` tool group: replay.

use std::sync::Arc;

use rmcp::model::JsonObject;

use super::super::super::schemas::{enum_prop, object_schema, props, string_prop};

pub(in super::super) fn replay_schema() -> Arc<JsonObject> {
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
