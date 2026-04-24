//! Schema builders for the `interact` tool group: type, press, scroll, drag, drop.

use std::sync::Arc;

use rmcp::model::JsonObject;

use super::super::super::schemas::{
    any_prop, array_string_prop, enum_prop, integer_prop, object_schema, props, string_prop,
};

pub(in super::super) fn type_schema() -> Arc<JsonObject> {
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

pub(in super::super) fn press_schema() -> Arc<JsonObject> {
    object_schema(
        props([("key", string_prop("Keyboard key to press."))]),
        &["key"],
    )
}

pub(in super::super) fn scroll_schema() -> Arc<JsonObject> {
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

pub(in super::super) fn drag_schema() -> Arc<JsonObject> {
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

pub(in super::super) fn drop_schema() -> Arc<JsonObject> {
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
