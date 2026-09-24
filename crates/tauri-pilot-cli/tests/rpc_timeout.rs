//! Regression test for issue #241: a socket that accepts the connection but
//! never answers must not block the CLI forever.
//!
//! The listener is bound and never serviced, like the issue's reproduction:
//! the kernel completes the connection from the backlog, the request lands in
//! the socket buffer, and no answer ever comes back.

#![cfg(unix)]

mod common;

// Rust guideline compliant 2026-08-29
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::{unique_socket_path, wait_bounded};

/// Binds a socket nobody ever reads from.
///
/// The listener must outlive the binary: dropping it would refuse the
/// connection instead of leaving it unanswered.
///
/// # Panics
///
/// Panics if the socket cannot be bound.
fn silent_socket(tag: &str) -> (UnixListener, PathBuf) {
    let socket = unique_socket_path(tag);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind silent socket");
    (listener, socket)
}

/// Runs `tauri-pilot --socket <socket> <args>` from `cwd`.
///
/// # Panics
///
/// Panics if the binary cannot be spawned or is still running after
/// `common::SERVER_DONE_TIMEOUT`.
fn run(cwd: &Path, socket: &Path, args: &[&str]) -> Output {
    let child = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .current_dir(cwd)
        .env_remove("TAURI_PILOT_RPC_TIMEOUT")
        .arg("--socket")
        .arg(socket)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tauri-pilot");
    wait_bounded(child)
}

#[test]
fn ping_against_a_silent_socket_gives_up_after_rpc_timeout() {
    let (_listener, socket) = silent_socket("rpc-timeout");
    let output = run(Path::new("."), &socket, &["--rpc-timeout", "1", "ping"]);
    let _ = std::fs::remove_file(&socket);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(
        stderr.contains(&format!(
            "No response from the app after 1s: {} accepted the connection but did not answer.",
            socket.display()
        )),
        "stderr must say who stayed silent and for how long: {stderr}"
    );
}

/// A step that hits its `timeout_ms` leaves its request unanswered on the
/// connection. The failure screenshot that follows must not wait on it: with
/// the default deadline it would outlast `common::SERVER_DONE_TIMEOUT`.
#[test]
fn run_step_timeout_does_not_wait_on_the_failure_screenshot() {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let scenario = tmpdir.path().join("scenario.toml");
    std::fs::write(
        &scenario,
        r##"
[scenario]
name = "silent"
[[step]]
name = "click silent"
action = "click"
target = "#nope"
timeout_ms = 200
"##,
    )
    .expect("write scenario");
    let (_listener, socket) = silent_socket("rpc-timeout-run");
    let scenario = scenario.to_str().expect("scenario path is UTF-8");
    let output = run(tmpdir.path(), &socket, &["--json", "run", scenario]);
    let _ = std::fs::remove_file(&socket);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "failing scenario exits 1");
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("--json must print a JSON report ({err}): {stdout}"));
    let step = &report["steps"][0];
    assert!(
        step["message"]
            .as_str()
            .is_some_and(|m| m.contains("timed out after 200ms")),
        "the step reports its own timeout: {report}"
    );
    assert!(
        step["screenshot_error"]
            .as_str()
            .is_some_and(|e| e.contains("out of sync")),
        "the screenshot is refused, not left waiting: {report}"
    );
}
