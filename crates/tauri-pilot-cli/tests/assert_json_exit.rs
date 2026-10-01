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

/// Every assertion kind prints its own failure object on stdout under
/// `--json`, exits 1, and writes nothing on stderr.
#[test]
fn json_failure_object_for_every_assert_kind() {
    let cases: [(&[&str], serde_json::Value, serde_json::Value); 9] = [
        (
            &["text", "#btn", "nope"],
            serde_json::json!("Log in"),
            serde_json::json!({
                "ok": false,
                "message": "expected text \"nope\", got \"Log in\"",
                "expected": "nope",
                "actual": "Log in",
            }),
        ),
        (
            &["value", "#email", "a@b.c"],
            serde_json::json!("x@y.z"),
            serde_json::json!({
                "ok": false,
                "message": "expected value \"a@b.c\", got \"x@y.z\"",
                "expected": "a@b.c",
                "actual": "x@y.z",
            }),
        ),
        (
            &["count", ".item", "5"],
            serde_json::json!({"count": 3}),
            serde_json::json!({
                "ok": false,
                "message": "expected 5 elements, found 3",
                "expected": 5,
                "actual": 3,
            }),
        ),
        (
            &["contains", "#msg", "saved"],
            serde_json::json!("Error: retry"),
            serde_json::json!({
                "ok": false,
                "message": "text does not contain \"saved\", got \"Error: retry\"",
                "expected": "saved",
                "actual": "Error: retry",
            }),
        ),
        (
            &["url", "/dashboard"],
            serde_json::json!("http://localhost/login"),
            serde_json::json!({
                "ok": false,
                "message": "URL does not contain \"/dashboard\", got \"http://localhost/login\"",
                "expected": "/dashboard",
                "actual": "http://localhost/login",
            }),
        ),
        (
            &["visible", "#form"],
            serde_json::json!({"visible": false}),
            serde_json::json!({"ok": false, "message": "element is not visible"}),
        ),
        (
            &["hidden", "#spinner"],
            serde_json::json!({"visible": true}),
            serde_json::json!({"ok": false, "message": "element is visible"}),
        ),
        (
            &["checked", "#terms"],
            serde_json::json!({"checked": false}),
            serde_json::json!({"ok": false, "message": "element is not checked"}),
        ),
        (
            &["unchecked", "#terms"],
            serde_json::json!({"checked": true}),
            serde_json::json!({"ok": false, "message": "element is checked"}),
        ),
    ];

    for (args, server_result, expected) in cases {
        let output = run_assert(true, args, server_result);

        assert_eq!(output.status.code(), Some(1), "assert {args:?}");
        assert_eq!(stdout_json(&output), expected, "assert {args:?}");
        assert!(
            output.stderr.is_empty(),
            "assert {args:?}: stderr={:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
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
