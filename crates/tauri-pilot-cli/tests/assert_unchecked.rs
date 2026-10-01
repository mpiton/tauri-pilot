//! Regression test for issue #286: `assert unchecked` passes when the bridge
//! reports `checked: false`, fails with exit 1 when it reports `true`, and
//! errors when the bridge rejects a target that is not a checkbox or radio.
//!
//! A one-shot mock JSON-RPC unix socket server from `common` answers the
//! single request, then the binary runs against that socket. Both sides are
//! bounded by `SERVER_DONE_TIMEOUT`, so a hang fails the test, not the suite.

#![cfg(unix)]

mod common;

// Rust guideline compliant 2026-02-21
use std::process::{Command, Output, Stdio};

use common::{SERVER_DONE_TIMEOUT, spawn_mock_responder, unique_socket_path, wait_bounded};

/// Runs `assert unchecked <target>` against a mock bridge whose `checked`
/// method answers with `body` (`{"result": ...}` or `{"error": ...}`).
fn run_assert_unchecked(target: &'static str, body: serde_json::Value) -> Output {
    let socket = unique_socket_path("assert-unchecked");
    let done = spawn_mock_responder(&socket, move |req| {
        assert_eq!(req["method"], "checked");
        assert_eq!(req["params"]["selector"], target);
        body
    });
    let child = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .arg("--socket")
        .arg(&socket)
        .args(["assert", "unchecked", target])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tauri-pilot");
    let output = wait_bounded(child);

    // Disconnected means the server panicked (e.g. wrong method); timeout means
    // the binary never connected.
    let served = done.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    if let Err(err) = served {
        panic!(
            "mock server did not answer checked: {err}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output
}

#[test]
fn assert_unchecked_passes_when_not_checked() {
    let output = run_assert_unchecked(
        "#remember",
        serde_json::json!({"result": {"checked": false}}),
    );

    assert!(
        output.status.success(),
        "unchecked box must exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("ok"));
}

#[test]
fn assert_unchecked_fails_with_exit_1_when_checked() {
    let output = run_assert_unchecked(
        "#remember",
        serde_json::json!({"result": {"checked": true}}),
    );

    assert_eq!(output.status.code(), Some(1), "checked box must exit 1");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("FAIL: element is checked"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The bridge rejects a target that is not a checkbox or radio; the CLI must
/// surface that error instead of reporting the box as unchecked.
#[test]
fn assert_unchecked_errors_when_the_target_is_not_checkable() {
    let message =
        "checked requires an <input type=\"checkbox\"> or <input type=\"radio\">, got: div";
    let output = run_assert_unchecked(
        "#remember",
        serde_json::json!({"error": {"code": -32000, "message": message}}),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "a rejected target must not pass, stdout: {stdout}"
    );
    assert!(!stdout.contains("ok"), "stdout: {stdout}");
    assert!(stderr.contains(message), "stderr: {stderr}");
}
