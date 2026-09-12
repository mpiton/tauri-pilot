//! Regression test for issue #160: `storage get` on a missing key must exit
//! non-zero, while a key holding an empty string still exits 0.
//!
//! Same harness as `snapshot_save_json.rs`: a one-shot mock JSON-RPC unix
//! socket server answers the single `storage.get` request, then the binary runs
//! against that socket via `assert_cmd`.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Output;
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

/// Run `storage get <key>` against a mock server answering with `result`.
fn run_storage_get(tag: &str, result: serde_json::Value, json: bool) -> Output {
    let socket = unique_socket_path(tag);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind mock socket");
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;
        let mut line = String::new();
        reader.read_line(&mut line).expect("read line");
        let req: serde_json::Value = serde_json::from_str(line.trim()).expect("parse request");
        assert_eq!(req["method"], "storage.get");
        let resp = serde_json::json!({"jsonrpc": "2.0", "id": req["id"], "result": result});
        let mut bytes = serde_json::to_vec(&resp).expect("serialize");
        bytes.push(b'\n');
        writer.write_all(&bytes).expect("write");
        writer.flush().expect("flush");
    });

    let mut args = vec!["--socket", socket.to_str().expect("socket path is UTF-8")];
    if json {
        args.push("--json");
    }
    args.extend(["storage", "get", "some-key"]);
    let output = Command::cargo_bin("tauri-pilot")
        .expect("cargo_bin")
        .args(&args)
        .output()
        .expect("run tauri-pilot");

    // The binary connected and sent a request unless it failed before that
    // (e.g. arg-parse error); do not block on accept() in that case.
    if output.status.code() == Some(2) {
        let _ = std::fs::remove_file(&socket);
        panic!(
            "binary exited before connecting.\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    handle.join().expect("mock server join");
    let _ = std::fs::remove_file(&socket);
    output
}

#[test]
fn storage_get_missing_key_exits_1_with_empty_stdout() {
    let output = run_storage_get(
        "storage-missing",
        serde_json::json!({"found": false}),
        false,
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
    let output = run_storage_get(
        "storage-missing-json",
        serde_json::json!({"found": false}),
        true,
    );

    assert_eq!(output.status.code(), Some(1), "missing key must exit 1");
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is JSON");
    assert_eq!(stdout, serde_json::json!({"found": false}));
}

#[test]
fn storage_get_empty_value_exits_0() {
    let output = run_storage_get(
        "storage-empty",
        serde_json::json!({"found": true, "value": ""}),
        false,
    );

    assert!(output.status.success(), "present-but-empty key must exit 0");
    assert_eq!(output.stdout, b"\n");
}
