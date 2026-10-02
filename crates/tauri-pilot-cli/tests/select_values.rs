//! Regression tests for issue #306: `select` takes one or more values, from
//! the CLI and from a scenario step, and sends a list when given several.
//!
//! A looping mock JSON-RPC unix socket records every request and answers
//! `{"ok": true}`. The binary is then spawned against it.

#![cfg(unix)]

mod common;

use std::process::Output;

use common::run_against_looping_mock;
use serde_json::{Value, json};

/// Runs the binary with `args` against a mock that answers `{"ok": true}`.
///
/// Returns the binary's output and the requests the mock received.
fn run_against_mock(args: &[&str]) -> (Output, Vec<Value>) {
    run_against_looping_mock("select-values", args, json!({"ok": true}))
}

#[test]
fn select_with_several_values_sends_the_list() {
    let (output, requests) = run_against_mock(&["select", "select[name=skills]", "rust", "go"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "select");
    assert_eq!(
        requests[0]["params"],
        json!({"selector": "select[name=skills]", "value": ["rust", "go"]})
    );
}

/// One value still goes out as a string, so an older plugin keeps working.
#[test]
fn select_with_one_value_sends_a_string() {
    let (output, requests) = run_against_mock(&["select", "@e5", "admin"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");
    assert_eq!(requests.len(), 1, "requests: {requests:?}");
    assert_eq!(requests[0]["method"], "select");
    assert_eq!(
        requests[0]["params"],
        json!({"ref": "e5", "value": "admin"})
    );
}

#[test]
fn scenario_select_step_sends_a_list_value() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("scenario.toml");
    std::fs::write(
        &path,
        r#"
[[step]]
action = "select"
target = "select[name=skills]"
value = ["rust", "go"]
"#,
    )
    .expect("write scenario");
    let (output, requests) = run_against_mock(&["run", path.to_str().expect("UTF-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");

    let select: Vec<&Value> = requests
        .iter()
        .filter(|r| r["method"] == "select")
        .collect();
    assert_eq!(select.len(), 1, "requests: {requests:?}");
    assert_eq!(
        select[0]["params"],
        json!({"selector": "select[name=skills]", "value": ["rust", "go"]})
    );
}

/// A recorded multi-select step replays with its whole list.
#[test]
fn replay_sends_the_recorded_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("rec.json");
    let recording = json!([{
        "action": "select", "timestamp": 0,
        "selector": "select[name=skills]", "value": ["rust", "go"],
    }]);
    std::fs::write(&path, recording.to_string()).expect("write recording");
    let (output, requests) =
        run_against_mock(&["--json", "replay", path.to_str().expect("UTF-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "select");
    assert_eq!(
        requests[0]["params"],
        json!({"selector": "select[name=skills]", "value": ["rust", "go"]})
    );
}
