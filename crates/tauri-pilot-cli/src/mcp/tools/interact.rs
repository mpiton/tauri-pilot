//! Tool specs for the `interact` domain.
//!
//! Tools: `click`, `fill`, `type`, `press`, `select`, `check`, `scroll`, `drag`, `drop`.

use super::super::schemas::{fill_schema, target_schema};
use super::ToolSpec;
use super::schemas::{drag_schema, drop_schema, press_schema, scroll_schema, type_schema};

pub(super) fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "click",
            description: "Click an element target by ref, selector, or coordinates.",
            schema: target_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "fill",
            description: "Clear and fill an input target with a value.",
            schema: fill_schema,
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "type",
            description: "Type text into an element target without clearing it first.",
            schema: type_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "press",
            description: "Press a keyboard key.",
            schema: press_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "select",
            description: "Select an option in a select element.",
            schema: fill_schema,
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "check",
            description: "Toggle a checkbox or radio element.",
            schema: target_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "scroll",
            description: "Scroll the page or an element ref.",
            schema: scroll_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "drag",
            description: "Drag an element to another target or by an offset.",
            schema: drag_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "drop",
            description: "Drop one or more local files on an element target.",
            schema: drop_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
    ]
}
