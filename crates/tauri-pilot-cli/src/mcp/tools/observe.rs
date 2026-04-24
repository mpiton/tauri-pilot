//! Tool specs for the `observe` domain.
//!
//! Tools: `watch`, `logs`, `network`.

use super::ToolSpec;
use super::schemas::{logs_schema, network_schema, watch_schema};

pub(super) fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "watch",
            description: "Watch for DOM mutations until the page is stable.",
            schema: watch_schema,
            read_only: true,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "logs",
            description: "Read or clear captured console logs.",
            schema: logs_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "network",
            description: "Read or clear captured network requests.",
            schema: network_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
    ]
}
