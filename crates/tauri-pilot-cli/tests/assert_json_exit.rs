//! Regression test for #282: a failed `assert` exits 1 and, with `--json`,
//! prints `{"ok": false, "message": ...}` on stdout instead of nothing. The
//! plain-text mode keeps the `FAIL:` line on stderr.

#![cfg(unix)]

mod common;

use std::process::{Command, Output, Stdio};

use common::{SERVER_DONE_TIMEOUT, wait_bounded};

/// Runs `[--json] assert <args>` against a mock server answering `result`.
fn run_assert(json: bool, args: &[&str], result: serde_json::Value) -> Output {
    let socket = common::unique_socket_path("assert");
    let done = common::spawn_mock_server(&socket, result);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"));
    cmd.arg("--socket").arg(&socket);
    if json {
        cmd.arg("--json");
    }
    let child = cmd
        .arg("assert")
        .args(args)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tauri-pilot");
    let output = wait_bounded(child);
    let served = done.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    assert!(
        served.is_ok(),
        "mock server did not answer: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn stdout_json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {:?}, stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn json_text_mismatch_prints_failure_object_and_exits_1() {
    let output = run_assert(true, &["text", "#btn", "nope"], serde_json::json!("Log in"));

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout_json(&output),
        serde_json::json!({
            "ok": false,
            "message": "expected text \"nope\", got \"Log in\"",
            "expected": "nope",
            "actual": "Log in",
        })
    );
    assert!(
        output.stderr.is_empty(),
        "stderr={:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn json_count_mismatch_reports_numbers() {
    let output = run_assert(
        true,
        &["count", ".item", "5"],
        serde_json::json!({"count": 3}),
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout_json(&output),
        serde_json::json!({
            "ok": false,
            "message": "expected 5 elements, found 3",
            "expected": 5,
            "actual": 3,
        })
    );
}

#[test]
fn json_visible_failure_has_message_only() {
    let output = run_assert(
        true,
        &["visible", "#form"],
        serde_json::json!({"visible": false}),
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout_json(&output),
        serde_json::json!({"ok": false, "message": "element is not visible"})
    );
}

#[test]
fn json_pass_prints_ok_and_exits_0() {
    let output = run_assert(
        true,
        &["text", "#btn", "Log in"],
        serde_json::json!("Log in"),
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout_json(&output), serde_json::json!({"ok": true}));
    assert!(output.stderr.is_empty());
}

#[test]
fn plain_text_mismatch_prints_fail_line_on_stderr_and_exits_1() {
    let output = run_assert(
        false,
        &["text", "#btn", "nope"],
        serde_json::json!("Log in"),
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "stdout={:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "FAIL: expected text \"nope\", got \"Log in\"\n"
    );
}

#[test]
fn plain_visible_pass_prints_ok_and_exits_0() {
    let output = run_assert(
        false,
        &["visible", "#form"],
        serde_json::json!({"visible": true}),
    );

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "✓ ok\n");
    assert!(output.stderr.is_empty());
}
