//! Regression test for #244: the `warning` a `diff` result carries for a
//! reference without recorded capture options goes to stderr, so it never
//! lands in `tauri-pilot diff > out.txt`.

#![cfg(unix)]

mod common;

// Rust guideline compliant 2026-08-29
use std::process::{Command, Stdio};

use common::{SERVER_DONE_TIMEOUT, wait_bounded};

#[test]
fn diff_warning_goes_to_stderr_not_stdout() {
    let socket = common::unique_socket_path("diffwarn");
    let done = common::spawn_mock_server(
        &socket,
        serde_json::json!({"added": [], "removed": [], "changed": [], "warning": "legacy ref"}),
    );
    let child = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .arg("--socket")
        .arg(&socket)
        .arg("diff")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tauri-pilot");
    let output = wait_bounded(child);
    let served = done.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        served.is_ok(),
        "mock server did not answer: stderr={stderr}"
    );

    assert!(output.status.success(), "stderr={stderr}");
    assert!(stderr.contains("legacy ref"), "stderr={stderr}");
    assert!(!stdout.contains("legacy ref"), "stdout={stdout}");
}
