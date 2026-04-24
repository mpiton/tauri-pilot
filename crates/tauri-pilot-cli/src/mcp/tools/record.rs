//! Tool specs for the `record` domain.
//!
//! Tools: `record_start`, `record_stop`, `record_status`, `replay`.

use super::super::schemas::empty_schema;
use super::ToolSpec;
use super::schemas::replay_schema;

pub(super) fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "record_start",
            description: "Start recording app interactions.",
            schema: empty_schema,
            read_only: false,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "record_stop",
            description: "Stop recording and return recorded entries.",
            schema: empty_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
        ToolSpec {
            name: "record_status",
            description: "Get recorder status.",
            schema: empty_schema,
            read_only: true,
            destructive: false,
            idempotent: true,
        },
        ToolSpec {
            name: "replay",
            description: "Replay or export a recorded tauri-pilot session file.",
            schema: replay_schema,
            read_only: false,
            destructive: false,
            idempotent: false,
        },
    ]
}
