//! Tool specs for the `eval` domain.
//!
//! Tools: `eval`, `ipc`.

use super::ToolSpec;
use super::schemas::{eval_schema, ipc_schema};

pub(super) fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "eval",
            description: "Evaluate JavaScript in the WebView context.",
            schema: eval_schema,
            read_only: false,
            destructive: true,
            idempotent: false,
        },
        ToolSpec {
            name: "ipc",
            description: "Invoke a Tauri IPC command with optional JSON arguments.",
            schema: ipc_schema,
            read_only: false,
            destructive: true,
            idempotent: false,
        },
    ]
}
