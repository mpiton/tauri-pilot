//! Tool specs for the `storage` domain.
//!
//! Tools: `storage_get`, `storage_set`, `storage_list`, `storage_clear`, `forms`.

use super::super::schemas::{selector_schema, session_schema};
use super::ToolSpec;
use super::schemas::{storage_get_schema, storage_set_schema};

pub(super) fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "storage_get",
            description: "Read a localStorage or sessionStorage key.",
            schema: storage_get_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "storage_set",
            description: "Set a localStorage or sessionStorage key.",
            schema: storage_set_schema,
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "storage_list",
            description: "List localStorage or sessionStorage entries.",
            schema: session_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "storage_clear",
            description: "Clear localStorage or sessionStorage.",
            schema: session_schema,
            read_only: false,
            destructive: true,
            idempotent: true,
        },
        ToolSpec {
            name: "forms",
            description: "Dump all form fields on the page or inside a selector.",
            schema: selector_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
    ]
}
