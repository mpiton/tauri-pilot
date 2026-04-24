//! Characterization tests for `tauri_pilot_cli::output` — pin current behavior
//! before splitting `output.rs` into per-domain modules (issue #70 PR1).
//!
//! Only String-returning formatters are tested. Void formatters
//! (`format_text`, `format_snapshot`, `format_diff`, etc.) print directly
//! to stdout — capturing real stdout from a test requires unsafe FD
//! manipulation or external crates, which is out of scope for PR1. They
//! are exercised indirectly by integration smoke tests against a real
//! sample app.
//!
//! NOTE: `format_logs` and `format_network` expect a JSON array directly
//! (not a wrapper object). `format_record` expects an object with known keys
//! such as `active`/`count` or `status`.

use serde_json::json;
use tauri_pilot_cli::output;

#[test]
fn test_format_logs_with_one_entry_contains_message() {
    // format_logs expects a JSON array; entries use `args` (not `message`).
    let payload = json!([
        { "timestamp": 1_700_000_000_000_u64, "level": "info", "args": ["hello"] }
    ]);
    let rendered = output::format_logs(&payload);
    assert!(
        rendered.contains("hello"),
        "expected 'hello' in rendered logs, got: {rendered}"
    );
}

#[test]
fn test_format_logs_with_empty_array_returns_string() {
    // format_logs expects a JSON array; empty array returns a "No logs" string.
    let payload = json!([]);
    let rendered = output::format_logs(&payload);
    assert!(
        !rendered.is_empty(),
        "expected non-empty string for empty log array"
    );
}

#[test]
fn test_format_record_with_active_recording_returns_non_empty() {
    // format_record shape: active status with count and elapsed_ms.
    let payload = json!({
        "active": true,
        "count": 5,
        "elapsed_ms": 2000_u64
    });
    let rendered = output::format_record(&payload);
    assert!(
        rendered.contains("5"),
        "expected count '5' in rendered output, got: {rendered}"
    );
    assert!(
        rendered.to_lowercase().contains("recording")
            || rendered.to_lowercase().contains("record"),
        "expected 'recording' marker in output, got: {rendered}"
    );
}

#[test]
fn test_format_replay_step_returns_non_empty() {
    let rendered = output::format_replay_step(1, 3, "click", "ok");
    assert!(
        rendered.contains("click"),
        "expected 'click' in: {rendered}"
    );
    assert!(rendered.contains("ok"), "expected 'ok' in: {rendered}");
}

#[test]
fn test_format_network_with_empty_array_does_not_panic() {
    // format_network expects a JSON array directly.
    let payload = json!([]);
    let _rendered = output::format_network(&payload);
}
