//! Tool specs for the `assert` domain.
//!
//! Tools: `assert_text`, `assert_contains`, `assert_visible`, `assert_hidden`,
//!        `assert_value`, `assert_count`, `assert_checked`, `assert_url`.

use super::super::schemas::{expected_schema, expected_target_schema, target_schema};
use super::ToolSpec;
use super::schemas::assert_count_schema;

pub(super) fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "assert_text",
            description: "Assert exact text content for an element target.",
            schema: expected_target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_contains",
            description: "Assert that target text contains a substring.",
            schema: expected_target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_visible",
            description: "Assert that an element target is visible.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_hidden",
            description: "Assert that an element target is hidden.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_value",
            description: "Assert an input, textarea, or select value.",
            schema: expected_target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_count",
            description: "Assert the number of elements matching a selector.",
            schema: assert_count_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_checked",
            description: "Assert that a checkbox or radio target is checked.",
            schema: target_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "assert_url",
            description: "Assert that the current URL contains a substring.",
            schema: expected_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
    ]
}
