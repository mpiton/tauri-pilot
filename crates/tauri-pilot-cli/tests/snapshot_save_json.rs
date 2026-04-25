//! Integration test for issue #80: `tauri-pilot --json snapshot --save <path>` must
//! produce parseable JSON on stdout. Status messages must not pollute stdout.
//!
//! Pattern: spawn mock JSON-RPC unix socket server in a tokio runtime on a worker
//! thread, then run the binary via `assert_cmd` against that socket and inspect
//! stdout/stderr separately.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
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

/// Spawn a one-shot mock server that answers one `snapshot` request with a fixed
/// elements tree, then exits. Returns the join handle so the test can wait on it.
fn spawn_mock_snapshot_server(socket: &PathBuf) -> thread::JoinHandle<()> {
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
            "result": {
                "elements": [
                    {"depth": 0, "role": "root", "name": "Root"},
                    {"depth": 1, "role": "button", "name": "Click"}
                ]
            }
        });
        let mut bytes = serde_json::to_vec(&resp).expect("serialize");
        bytes.push(b'\n');
        writer.write_all(&bytes).expect("write");
        writer.flush().expect("flush");
    })
}

#[test]
fn snapshot_save_json_stdout_is_pure_parseable_json() {
    let socket = unique_socket_path("snap-save");
    let handle = spawn_mock_snapshot_server(&socket);

    let tmpdir = tempfile::tempdir().expect("tempdir");
    let save_path = tmpdir.path().join("snap.json");

    let output = Command::cargo_bin("tauri-pilot")
        .expect("cargo_bin")
        .args([
            "--socket",
            socket.to_str().unwrap(),
            "--json",
            "snapshot",
            "--save",
            save_path.to_str().unwrap(),
        ])
        .output()
        .expect("run tauri-pilot");

    handle.join().expect("mock server join");
    let _ = std::fs::remove_file(&socket);

    assert!(
        output.status.success(),
        "binary exited with non-zero status: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not parseable JSON (issue #80): {e}\n--- stdout ---\n{stdout}\n--- end ---"
        )
    });

    let path_in_json = parsed
        .get("path")
        .and_then(|v| v.as_str())
        .expect("JSON should expose saved file path");
    assert_eq!(path_in_json, save_path.to_str().unwrap());

    let elements = parsed
        .get("elements")
        .and_then(|v| v.as_array())
        .expect("snapshot elements must remain in JSON");
    assert_eq!(elements.len(), 2);

    assert!(save_path.exists(), "save file should exist");
    let written = std::fs::read_to_string(&save_path).expect("read written file");
    let parsed_file: serde_json::Value = serde_json::from_str(&written).expect("file is JSON");
    assert!(parsed_file.get("elements").is_some());
}
