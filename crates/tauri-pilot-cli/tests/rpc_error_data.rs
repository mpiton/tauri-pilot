//! Regression test for issue #149, bug 1: structured `error.data` returned by
//! the plugin must reach the user instead of being dropped by the shared RPC
//! error path in `client/mod.rs`.
//!
//! Same harness as `snapshot_save_json.rs`: a one-shot mock JSON-RPC unix
//! socket server answers the single request, then the binary runs against that
//! socket via `assert_cmd` and its stderr is inspected.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use assert_cmd::Command;

static SOCK_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn unique_socket_path(tag: &str) -> PathBuf {
    let n = SOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!(
        "/tmp/tauri-pilot-it-{}-{}-{}.sock",
        tag,
        std::process::id(),
        n
    ))
}

/// Answer one request with `error` as the JSON-RPC error object.
fn spawn_mock_error_server(socket: &PathBuf, error: serde_json::Value) -> thread::JoinHandle<()> {
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket).expect("bind mock socket");
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;
        let mut line = String::new();
        reader.read_line(&mut line).expect("read line");
        let req: serde_json::Value = serde_json::from_str(line.trim()).expect("parse request");
        let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let resp = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": error,
        });
        let mut bytes = serde_json::to_vec(&resp).expect("serialize");
        bytes.push(b'\n');
        writer.write_all(&bytes).expect("write");
        writer.flush().expect("flush");
    })
}

/// Answer one request with the exact `WINDOW_NOT_FOUND` payload the macOS
/// plugin builds in `screenshot/ipc.rs`: `RPC_INVALID_PARAMS`, a message, and
/// an `available_windows` list carried under `error.data`.
fn spawn_mock_window_not_found_server(socket: &PathBuf) -> thread::JoinHandle<()> {
    spawn_mock_error_server(
        socket,
        serde_json::json!({
            "code": -32602,
            "message": "window_id 999 not found",
            "data": {
                "error": "WINDOW_NOT_FOUND",
                "message": "window_id 999 not found",
                "available_windows": [
                    {"window_id": 10042, "owner": "Prism", "title": "Prism — Home", "layer": 0},
                    {"window_id": 20087, "owner": "Finder", "title": "Downloads", "layer": 0}
                ]
            }
        }),
    )
}

/// Run the binary against `socket` with a `screenshot_native` call that is
/// guaranteed to reach the RPC error path, and return its stderr.
fn stderr_of_failed_screenshot(socket: &Path) -> String {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let output_path = tmpdir.path().join("shot.png");

    let output = Command::cargo_bin("tauri-pilot")
        .expect("cargo_bin")
        .args([
            "--socket",
            socket.to_str().expect("socket path is UTF-8"),
            "screenshot_native",
            "--window-id",
            "999",
            "--output",
            output_path.to_str().expect("output path is UTF-8"),
        ])
        .output()
        .expect("run tauri-pilot");

    assert!(
        !output.status.success(),
        "a JSON-RPC error must exit non-zero"
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn test_rpc_error_with_data_prints_available_windows_on_stderr() {
    let socket = unique_socket_path("errdata");
    let handle = spawn_mock_window_not_found_server(&socket);

    let tmpdir = tempfile::tempdir().expect("tempdir");
    let output_path = tmpdir.path().join("shot.png");

    let output = Command::cargo_bin("tauri-pilot")
        .expect("cargo_bin")
        .args([
            "--socket",
            socket.to_str().expect("socket path is UTF-8"),
            "screenshot_native",
            "--window-id",
            "999",
            "--output",
            output_path.to_str().expect("output path is UTF-8"),
        ])
        .output()
        .expect("run tauri-pilot");

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    // Fail fast before join() so a binary that exits before connecting (e.g.
    // arg-parse error) cannot leave the mock server blocked on accept() and
    // hang the test indefinitely.
    if !stderr.contains("window_id 999 not found") {
        let _ = std::fs::remove_file(&socket);
        panic!("binary never reached the RPC error path.\n--- stderr ---\n{stderr}\n--- end ---");
    }
    handle.join().expect("mock server join");
    let _ = std::fs::remove_file(&socket);

    assert!(
        !output.status.success(),
        "a JSON-RPC error must exit non-zero"
    );

    assert_eq!(
        stderr.matches("window_id 999 not found").count(),
        1,
        "the `message` mirrored into `error.data` must not be printed twice.\n\
         --- stderr ---\n{stderr}\n--- end ---"
    );

    for needle in ["10042", "Prism", "20087", "Finder"] {
        assert!(
            stderr.contains(needle),
            "issue #149: `error.data.available_windows` must reach the user, \
             missing `{needle}`.\n--- stderr ---\n{stderr}\n--- end ---"
        );
    }
}

/// A `data.message` that differs from `error.message` is detail, not the
/// plugin's mirror of the top-level message, so it must survive the dedup.
#[test]
fn test_rpc_error_data_message_distinct_from_error_message_is_kept() {
    let socket = unique_socket_path("errdata-distinct");
    let handle = spawn_mock_error_server(
        &socket,
        serde_json::json!({
            "code": -32602,
            "message": "window_id 999 not found",
            "data": {
                "error": "WINDOW_NOT_FOUND",
                "message": "the window closed between enumeration and capture"
            }
        }),
    );

    let stderr = stderr_of_failed_screenshot(&socket);
    handle.join().expect("mock server join");
    let _ = std::fs::remove_file(&socket);

    assert!(
        stderr.contains("the window closed between enumeration and capture"),
        "a `data.message` that is not the mirrored `error.message` must reach \
         the user.\n--- stderr ---\n{stderr}\n--- end ---"
    );
}
