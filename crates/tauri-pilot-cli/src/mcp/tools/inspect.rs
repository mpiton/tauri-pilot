//! Tool specs for the `inspect` domain.
//!
//! Tools: `text`, `html`, `value`, `attrs`.

use super::super::schemas::{optional_target_schema, target_schema};
use super::ToolSpec;

pub(super) fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "text",
            description: "Get text content for an element target.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "html",
            description: "Get inner HTML for an element target, or the full page if target is omitted.",
            schema: optional_target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "value",
            description: "Get an input, textarea, or select value.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "attrs",
            description: "Get all HTML attributes for an element target.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
    ]
}
