//! Schema builders for the `assert` tool group: `assert_count`.

use std::sync::Arc;

use rmcp::model::JsonObject;

use super::super::super::schemas::{integer_prop, object_schema, props, string_prop};

pub(in super::super) fn assert_count_schema() -> Arc<JsonObject> {
    object_schema(
        props([
            ("selector", string_prop("CSS selector to count.")),
            ("expected", integer_prop("Expected element count.")),
        ]),
        &["selector", "expected"],
    )
}
