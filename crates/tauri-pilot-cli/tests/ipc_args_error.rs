//! Regression test for issue #163: malformed `ipc --args` JSON must fail with
//! an error that names `--args` and echoes the value, keeping serde's message
//! as the cause.
//!
//! The mock socket only listens and never answers: the binary must give up on
//! `--args` before sending a request, and a pending connection needs no
//! `accept()` to succeed.

#![cfg(unix)]

// Rust guideline compliant 2026-08-29

use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::Duration;

use assert_cmd::Command;

/// How long the binary may run before the test kills it.
///
/// A binary that sends the request waits forever for a reply the mock never
/// writes; the kill turns that hang into a failed exit-code check instead of a
/// hung suite. The margin covers slow CI hosts.
const RUN_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn ipc_malformed_args_error_names_the_flag() {
    let socket = PathBuf::from(format!(
        "/tmp/tauri-pilot-it-ipc-args-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&socket);
    let _listener = UnixListener::bind(&socket).expect("bind mock socket");

    let output = Command::cargo_bin("tauri-pilot")
        .expect("cargo_bin")
        .args([
            "--socket",
            socket.to_str().expect("socket path is UTF-8"),
            "ipc",
            "some_command",
            "--args",
            "not-json",
        ])
        .timeout(RUN_TIMEOUT)
        .output()
        .expect("run tauri-pilot");
    let _ = std::fs::remove_file(&socket);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "malformed --args must exit 1.\n--- stderr ---\n{stderr}"
    );
    assert!(
        stderr.contains("--args must be a JSON object, got: not-json"),
        "the error must name --args and the bad value.\n--- stderr ---\n{stderr}"
    );
    assert!(
        stderr.contains("expected ident at line 1 column 2"),
        "serde's message must stay as the cause.\n--- stderr ---\n{stderr}"
    );
}
