//! Regression test for issue #161: `record stop` with no recording in progress
//! must exit 1 and leave the output path untouched, instead of saving `[]` and
//! exiting 0.
//!
//! Same harness as `storage_get_exit.rs`: a one-shot mock JSON-RPC unix socket
//! server answers the single request, then the binary runs against that socket
//! via `assert_cmd`.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
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

#[test]
fn record_stop_without_recording_exits_1_and_writes_no_file() {
    let socket = PathBuf::from(format!(
        "/tmp/tauri-pilot-it-record-stop-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind mock socket");
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;
        let mut line = String::new();
        reader.read_line(&mut line).expect("read line");
        let req: serde_json::Value = serde_json::from_str(line.trim()).expect("parse request");
        assert_eq!(req["method"], "record.stop");
        // What the plugin's `record.stop` returns when no recording is active:
        // `RPC_INVALID_PARAMS` (-32602) and the handler's message.
        let resp = serde_json::json!({
            "jsonrpc": "2.0",
            "id": req["id"],
            "error": {
                "code": -32602,
                "message": "No recording in progress. Run `record start` first",
            },
        });
        let mut bytes = serde_json::to_vec(&resp).expect("serialize");
        bytes.push(b'\n');
        writer.write_all(&bytes).expect("write");
        writer.flush().expect("flush");
        let _ = done_tx.send(());
    });

    let tmpdir = tempfile::tempdir().expect("tempdir");
    let output_path = tmpdir.path().join("rec.json");
    let output = Command::cargo_bin("tauri-pilot")
        .expect("cargo_bin")
        .args([
            "--socket",
            socket.to_str().expect("socket path is UTF-8"),
            "record",
            "stop",
            "--output",
            output_path.to_str().expect("output path is UTF-8"),
        ])
        .output()
        .expect("run tauri-pilot");

    // Disconnected means the server panicked (e.g. wrong method); timeout means
    // the binary never connected.
    let done = done_rx.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if let Err(err) = done {
        panic!("mock server did not answer record.stop: {err}\n--- stderr ---\n{stderr}");
    }

    assert_eq!(
        output.status.code(),
        Some(1),
        "idle record stop must exit 1"
    );
    assert!(
        stderr.contains("No recording in progress"),
        "the plugin error must reach stderr.\n--- stderr ---\n{stderr}"
    );
    assert!(
        !output_path.exists(),
        "idle record stop must not write {}",
        output_path.display()
    );
}
