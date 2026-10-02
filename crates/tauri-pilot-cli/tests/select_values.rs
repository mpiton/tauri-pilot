//! Regression tests for issue #306: `select` takes one or more values, from
//! the CLI and from a scenario step, and sends a list when given several.
//!
//! A looping mock JSON-RPC unix socket records every request and answers
//! `{"ok": true}`. The binary is then spawned against it.

#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;

use common::{SERVER_DONE_TIMEOUT, unique_socket_path};
use serde_json::{Value, json};

/// Runs the binary with `args` against a mock that answers `{"ok": true}`.
///
/// Returns the binary's output and the requests the mock received.
///
/// # Panics
///
/// Panics if the socket cannot be bound or the mock server does not finish.
fn run_against_mock(args: &[&str]) -> (Output, Vec<Value>) {
    let socket = unique_socket_path("select-values");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind mock socket");
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;
        let mut requests = Vec::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).expect("read line") == 0 {
                break;
            }
            let req: Value = serde_json::from_str(line.trim()).expect("parse request");
            let resp = json!({"jsonrpc": "2.0", "id": req["id"], "result": {"ok": true}});
            requests.push(req);
            let mut bytes = serde_json::to_vec(&resp).expect("serialize");
            bytes.push(b'\n');
            writer.write_all(&bytes).expect("write");
            writer.flush().expect("flush");
        }
        let _ = done_tx.send(requests);
    });

    let mut full = vec!["--socket", socket.to_str().expect("socket path is UTF-8")];
    full.extend_from_slice(args);
    let output = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .args(&full)
        .output()
        .expect("run tauri-pilot");
    let requests = done_rx.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    match requests {
        Ok(requests) => (output, requests),
        Err(err) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            panic!("mock server did not finish: {err}\n--- stderr ---\n{stderr}");
        }
    }
}

#[test]
fn select_with_several_values_sends_the_list() {
    let (output, requests) = run_against_mock(&["select", "select[name=skills]", "rust", "go"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "select");
    assert_eq!(
        requests[0]["params"],
        json!({"selector": "select[name=skills]", "value": ["rust", "go"]})
    );
}

/// One value still goes out as a string, so an older plugin keeps working.
#[test]
fn select_with_one_value_sends_a_string() {
    let (output, requests) = run_against_mock(&["select", "@e5", "admin"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        requests[0]["params"],
        json!({"ref": "e5", "value": "admin"})
    );
}

#[test]
fn scenario_select_step_sends_a_list_value() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("scenario.toml");
    std::fs::write(
        &path,
        r#"
[[step]]
action = "select"
target = "select[name=skills]"
value = ["rust", "go"]
"#,
    )
    .expect("write scenario");
    let (output, requests) = run_against_mock(&["run", path.to_str().expect("UTF-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");

    let select: Vec<&Value> = requests
        .iter()
        .filter(|r| r["method"] == "select")
        .collect();
    assert_eq!(select.len(), 1, "requests: {requests:?}");
    assert_eq!(
        select[0]["params"],
        json!({"selector": "select[name=skills]", "value": ["rust", "go"]})
    );
}

/// A recorded multi-select step replays with its whole list.
#[test]
fn replay_sends_the_recorded_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("rec.json");
    let recording = json!([{
        "action": "select", "timestamp": 0,
        "selector": "select[name=skills]", "value": ["rust", "go"],
    }]);
    std::fs::write(&path, recording.to_string()).expect("write recording");
    let (output, requests) =
        run_against_mock(&["--json", "replay", path.to_str().expect("UTF-8 path")]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "select");
    assert_eq!(
        requests[0]["params"],
        json!({"selector": "select[name=skills]", "value": ["rust", "go"]})
    );
}
