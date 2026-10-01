//! Regression test for issue #275: `replay` must exit 1 when a step fails and
//! say why each failed step failed, on stderr and in the `--json` result.
//!
//! A looping mock JSON-RPC unix socket answers every replayed action: `fill`
//! and `type` fail with the plugin's unknown-ref error, everything else
//! succeeds. The binary is then spawned against it.

#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;

use common::{SERVER_DONE_TIMEOUT, unique_socket_path};

/// The error the plugin returns for a ref the current page does not know.
const UNKNOWN_REF: &str = "Eval error: JavaScript error: Unknown ref: e5";

/// Replays `recording` against a mock that fails every method in `failing`.
///
/// `common::spawn_mock_server` answers a single request; a replay sends one
/// per step, so this keeps a local looping mock.
///
/// # Panics
///
/// Panics if the socket cannot be bound or the mock server does not finish.
fn replay_against_mock(dir: &Path, recording: &str, failing: &'static [&str]) -> Output {
    let socket = unique_socket_path("replay-fail");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind mock socket");
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).expect("read line") == 0 {
                break;
            }
            let req: serde_json::Value = serde_json::from_str(line.trim()).expect("parse request");
            let method = req["method"].as_str().unwrap_or_default();
            let resp = if failing.contains(&method) {
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req["id"],
                    "error": {"code": -32603, "message": UNKNOWN_REF},
                })
            } else {
                serde_json::json!({"jsonrpc": "2.0", "id": req["id"], "result": {"ok": true}})
            };
            let mut bytes = serde_json::to_vec(&resp).expect("serialize");
            bytes.push(b'\n');
            writer.write_all(&bytes).expect("write");
            writer.flush().expect("flush");
        }
        let _ = done_tx.send(());
    });

    let path = dir.join("rec.json");
    std::fs::write(&path, recording).expect("write recording");
    let output = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .args([
            "--socket",
            socket.to_str().expect("socket path is UTF-8"),
            "--json",
            "replay",
            path.to_str().expect("recording path is UTF-8"),
        ])
        .output()
        .expect("run tauri-pilot");
    let done = done_rx.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    if let Err(err) = done {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("mock server did not finish: {err}\n--- stderr ---\n{stderr}");
    }
    output
}

/// Four steps: two fail on an unknown ref, one passes, one is not replayable.
const MIXED_RECORDING: &str = r##"[
  {"action": "fill", "ref": "e5", "value": "recorded", "timestamp": 0},
  {"action": "click", "selector": "#trigger-deferred", "timestamp": 0},
  {"action": "assert", "timestamp": 0},
  {"action": "type", "ref": "e10", "text": "!", "timestamp": 0}
]"##;

#[test]
fn replay_with_failed_steps_exits_1_and_reports_each_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = replay_against_mock(dir.path(), MIXED_RECORDING, &["fill", "type"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(
        output.status.code(),
        Some(1),
        "a replay with failed steps must exit 1\n--- stderr ---\n{stderr}"
    );
    for line in [
        "[1/4] fill → ✗ FAIL: RPC error (-32603): Eval error: JavaScript error: Unknown ref: e5",
        "[4/4] type → ✗ FAIL: RPC error (-32603): Eval error: JavaScript error: Unknown ref: e5",
    ] {
        assert!(
            stderr.contains(line),
            "each failed step must print its error: {line}\n--- stderr ---\n{stderr}"
        );
    }

    let result: serde_json::Value =
        serde_json::from_str(&stdout).expect("--json prints the replay result");
    assert_eq!(result["status"], "failed");
    assert_eq!(result["failed"], 2);
    assert_eq!(
        result["steps"],
        serde_json::json!([
            {
                "action": "fill",
                "status": "failed",
                "message": "RPC error (-32603): Eval error: JavaScript error: Unknown ref: e5",
            },
            {"action": "click", "status": "passed"},
            {"action": "assert", "status": "skipped"},
            {
                "action": "type",
                "status": "failed",
                "message": "RPC error (-32603): Eval error: JavaScript error: Unknown ref: e5",
            },
        ])
    );
}

#[test]
fn replay_with_every_step_passing_exits_0() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = replay_against_mock(dir.path(), MIXED_RECORDING, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(0),
        "a clean replay must exit 0\n--- stderr ---\n{stderr}"
    );
    let result: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .expect("--json prints the replay result");
    assert_eq!(result["status"], "ok");
    assert_eq!(
        result["steps"][0],
        serde_json::json!({"action": "fill", "status": "passed"})
    );
}
