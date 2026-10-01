//! Regression test for issue #160: `storage get` on a missing key must exit
//! non-zero, while a key holding a value (even an empty string) exits 0. The
//! other `storage` subcommands must keep exiting 0.
//!
//! Same harness as `snapshot_save_json.rs`: a one-shot mock JSON-RPC unix
//! socket server answers the single request, then the binary runs against that
//! socket via `assert_cmd`.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use assert_cmd::Command;

/// How long to wait for the mock server once the binary has exited.
///
/// The binary only exits after reading the reply, so the server is done or a
/// few instructions away from it; the margin covers slow CI hosts. A binary
/// that exits before connecting leaves the server blocked on `accept()`, and
/// this bound turns that into a test failure instead of a hung suite.
const SERVER_DONE_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Run `storage <args>` against a mock server that expects `method` and
/// answers with `result`.
fn run_storage(args: &[&str], method: &'static str, result: serde_json::Value) -> Output {
    run_storage_capture(args, method, result).0
}

/// Like [`run_storage`], also returning the request params the binary sent.
fn run_storage_capture(
    args: &[&str],
    method: &'static str,
    result: serde_json::Value,
) -> (Output, serde_json::Value) {
    let socket = unique_socket_path("storage");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind mock socket");
    let (done_tx, done_rx) = mpsc::channel();
    let (params_tx, params_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;
        let mut line = String::new();
        reader.read_line(&mut line).expect("read line");
        let req: serde_json::Value = serde_json::from_str(line.trim()).expect("parse request");
        assert_eq!(req["method"], method);
        let _ = params_tx.send(req["params"].clone());
        let resp = serde_json::json!({"jsonrpc": "2.0", "id": req["id"], "result": result});
        let mut bytes = serde_json::to_vec(&resp).expect("serialize");
        bytes.push(b'\n');
        writer.write_all(&bytes).expect("write");
        writer.flush().expect("flush");
        let _ = done_tx.send(());
    });

    let output = Command::cargo_bin("tauri-pilot")
        .expect("cargo_bin")
        .args([
            "--socket",
            socket.to_str().expect("socket path is UTF-8"),
            "storage",
        ])
        .args(args)
        .output()
        .expect("run tauri-pilot");

    // Disconnected means the server panicked (e.g. wrong method); timeout means
    // the binary never connected.
    let done = done_rx.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    if let Err(err) = done {
        panic!(
            "mock server did not answer {method}: {err}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let params = params_rx.try_recv().unwrap_or(serde_json::Value::Null);
    (output, params)
}

#[test]
fn storage_get_missing_key_exits_1_with_empty_stdout() {
    let output = run_storage(
        &["get", "some-key"],
        "storage.get",
        serde_json::json!({"found": false}),
    );

    assert_eq!(output.status.code(), Some(1), "missing key must exit 1");
    assert!(
        output.stdout.is_empty(),
        "stdout must carry only the value, got {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("(not found)"));
}

#[test]
fn storage_get_missing_key_json_exits_1() {
    let output = run_storage(
        &["get", "some-key", "--json"],
        "storage.get",
        serde_json::json!({"found": false}),
    );

    assert_eq!(output.status.code(), Some(1), "missing key must exit 1");
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    assert_eq!(stdout, serde_json::json!({"found": false}));
}

#[test]
fn storage_get_empty_value_exits_0() {
    let output = run_storage(
        &["get", "some-key"],
        "storage.get",
        serde_json::json!({"found": true, "value": ""}),
    );

    assert!(output.status.success(), "present-but-empty key must exit 0");
    assert_eq!(output.stdout, b"\n");
}

#[test]
fn storage_get_present_value_exits_0() {
    let output = run_storage(
        &["get", "some-key"],
        "storage.get",
        serde_json::json!({"found": true, "value": "dark"}),
    );

    assert!(output.status.success(), "present key must exit 0");
    assert_eq!(output.stdout, b"dark\n");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("(not found)"));
}

#[test]
fn storage_get_present_value_json_exits_0() {
    let hit = serde_json::json!({"found": true, "value": "dark"});
    let output = run_storage(&["get", "some-key", "--json"], "storage.get", hit.clone());

    assert!(output.status.success(), "present key must exit 0");
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    assert_eq!(stdout, hit);
}

#[test]
fn storage_set_exits_0() {
    let output = run_storage(
        &["set", "some-key", "dark"],
        "storage.set",
        serde_json::json!({"ok": true}),
    );

    assert!(output.status.success(), "storage set must exit 0");
}

#[test]
fn storage_delete_prints_ok_and_sends_the_key() {
    let (output, params) = run_storage_capture(
        &["--session", "delete", "auth_token"],
        "storage.delete",
        serde_json::json!({"deleted": true}),
    );

    assert!(output.status.success(), "storage delete must exit 0");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "✓ ok\n");
    assert_eq!(
        params,
        serde_json::json!({"key": "auth_token", "session": true})
    );
}

#[test]
fn storage_delete_missing_key_exits_0_with_ok() {
    let output = run_storage(
        &["delete", "missing"],
        "storage.delete",
        serde_json::json!({"deleted": false}),
    );

    assert!(
        output.status.success(),
        "deleting a missing key must exit 0"
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "✓ ok\n");
}

#[test]
fn storage_delete_json_keeps_the_deleted_flag() {
    let output = run_storage(
        &["delete", "missing", "--json"],
        "storage.delete",
        serde_json::json!({"deleted": false}),
    );

    assert!(
        output.status.success(),
        "deleting a missing key must exit 0"
    );
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    assert_eq!(stdout, serde_json::json!({"deleted": false}));
}

#[test]
fn storage_delete_without_boolean_deleted_exits_1() {
    let output = run_storage(
        &["delete", "some-key"],
        "storage.delete",
        serde_json::json!({"ok": true}),
    );

    assert_eq!(
        output.status.code(),
        Some(1),
        "a response without boolean deleted must exit 1"
    );
    assert!(
        output.stdout.is_empty(),
        "no success line on a malformed response, got {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("deleted"),
        "error must name the missing deleted field, got: {stderr}"
    );
}

#[test]
fn storage_clear_exits_0() {
    let output = run_storage(
        &["clear"],
        "storage.clear",
        serde_json::json!({"cleared": true}),
    );

    assert!(output.status.success(), "storage clear must exit 0");
}
