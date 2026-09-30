//! Regression test for issue #257: a socket that accepts the connection and
//! then hangs up without answering must say why that usually happens.
//!
//! An `adb forward` left over from a previous launch of an Android app does
//! exactly that: `adb` accepts locally, finds no abstract socket under the
//! old name on the device, and closes the connection.

#![cfg(unix)]

mod common;

// Rust guideline compliant 2026-02-21
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};
use std::thread;

use common::{unique_socket_path, wait_bounded};

#[test]
fn ping_against_a_socket_that_hangs_up_hints_at_a_stale_forward() {
    let socket = unique_socket_path("hangs-up");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind socket");
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        // Read the request so the close is not a reset mid-write.
        let mut line = String::new();
        BufReader::new(&stream)
            .read_line(&mut line)
            .expect("read request");
    });

    let child = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .env_remove("TAURI_PILOT_RPC_TIMEOUT")
        .arg("--socket")
        .arg(&socket)
        .arg("ping")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tauri-pilot");
    let output = wait_bounded(child);
    server.join().expect("server thread");
    let _ = std::fs::remove_file(&socket);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr: {stderr}");
    assert!(
        stderr.contains("Server closed the connection"),
        "the error keeps its first line: {stderr}"
    );
    assert!(
        stderr.contains("app may have restarted") && stderr.contains("adb forward"),
        "the error must hint at a restarted app and a stale adb forward: {stderr}"
    );
}
