//! Regression test for issue #213: when stdout is a closed pipe (e.g.
//! `tauri-pilot snapshot | head -1`), the CLI must exit quietly instead of
//! panicking with "failed printing to stdout: Broken pipe".

#![cfg(unix)]

mod common;

// Rust guideline compliant 2026-08-29
use std::process::{Command, Output, Stdio};

use common::{SERVER_DONE_TIMEOUT, closed_pipe, wait_bounded};

/// Runs `tauri-pilot <args>` against a mock server answering `result`, with
/// stdout wired to `stdout`.
///
/// Both the binary and the mock server are bounded by `SERVER_DONE_TIMEOUT`,
/// so a binary that hangs, or exits without connecting, fails the test
/// instead of hanging the suite.
fn run(args: &[&str], result: serde_json::Value, stdout: Stdio) -> Output {
    let socket = common::unique_socket_path("epipe");
    let done = common::spawn_mock_server(&socket, result);
    let child = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .arg("--socket")
        .arg(&socket)
        .args(args)
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tauri-pilot");
    let output = wait_bounded(child);

    // Disconnected means the server panicked; timeout means the binary never
    // connected.
    let served = done.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    if let Err(err) = served {
        panic!(
            "mock server did not answer: {err}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output
}

fn snapshot_result() -> serde_json::Value {
    serde_json::json!({
        "elements": [
            {"depth": 0, "role": "heading", "name": "Pilot Test App", "ref": "e1"},
            {"depth": 0, "role": "button", "name": "Click", "ref": "e2"}
        ]
    })
}

#[test]
fn snapshot_into_closed_pipe_exits_quietly() {
    let output = run(&["snapshot"], snapshot_result(), closed_pipe());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "expected exit 0 on a closed stdout pipe, got {:?}: stderr={stderr}",
        output.status.code()
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("Broken pipe"),
        "closed stdout pipe must not print a panic: stderr={stderr}"
    );
}

#[test]
fn storage_get_missing_key_into_closed_pipe_keeps_exit_1() {
    let output = run(
        &["storage", "get", "some-key", "--json"],
        serde_json::json!({"found": false}),
        closed_pipe(),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(1),
        "a missing key must exit 1 even when stdout is closed: stderr={stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "closed stdout pipe must not print a panic: stderr={stderr}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn snapshot_into_full_device_still_panics() {
    // Writes to /dev/full fail with ENOSPC, not EPIPE.
    let full = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let output = run(&["snapshot"], snapshot_result(), full.into());
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(101),
        "write errors other than a broken pipe must still panic: stderr={stderr}"
    );
    assert!(
        stderr.contains("failed printing to stdout"),
        "panic message must name the stdout write: stderr={stderr}"
    );
}
