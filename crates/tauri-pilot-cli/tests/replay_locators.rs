//! Regression tests for issue #276: recordings carry stable locators and the
//! window of each step, and `replay` / `record stop` handle them.
//!
//! A looping mock JSON-RPC unix socket records every request and answers
//! `{"ok": true}` (or the `record.stop` result it was given). The binary is
//! then spawned against it.

#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;

use common::{SERVER_DONE_TIMEOUT, unique_socket_path};
use serde_json::{Value, json};

/// Runs the binary with `args` against an always-answering mock.
///
/// The mock answers every request with `result`. Returns the binary's output
/// and the requests the mock received.
///
/// # Panics
///
/// Panics if the socket cannot be bound or the mock server does not finish.
fn run_against_mock(args: &[&str], result: Value) -> (Output, Vec<Value>) {
    let socket = unique_socket_path("replay-locators");
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
            let resp = json!({"jsonrpc": "2.0", "id": req["id"], "result": result});
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

fn write_recording(dir: &Path, recording: &Value) -> String {
    let path = dir.join("rec.json");
    std::fs::write(&path, recording.to_string()).expect("write recording");
    path.to_str().expect("recording path is UTF-8").to_owned()
}

/// A step recorded with a locator in `settings`, then an old ref-only step.
fn recording() -> Value {
    json!([
        {
            "action": "check", "timestamp": 0, "window": "settings",
            "ref": "e9", "selector": "#plan-pro",
            "expect": {"tag": "input", "role": "radio", "name": "Pro"},
        },
        {"action": "fill", "timestamp": 0, "ref": "e5", "value": "recorded"},
    ])
}

#[test]
fn replay_sends_each_step_to_its_recorded_window_with_its_locator() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_recording(dir.path(), &recording());
    let (output, requests) = run_against_mock(&["--json", "replay", &path], json!({"ok": true}));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");

    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["method"], "check");
    assert_eq!(
        requests[0]["params"],
        json!({
            "window": "settings", "ref": "e9", "selector": "#plan-pro",
            "expect": {"tag": "input", "role": "radio", "name": "Pro"},
        })
    );
    // An entry without a window goes to the default window.
    assert!(requests[1]["params"].get("window").is_none());
}

#[test]
fn replay_window_flag_overrides_every_recorded_window() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_recording(dir.path(), &recording());
    let (output, requests) = run_against_mock(
        &["--json", "--window", "main", "replay", &path],
        json!({"ok": true}),
    );
    assert_eq!(output.status.code(), Some(0));
    let windows: Vec<&Value> = requests.iter().map(|r| &r["params"]["window"]).collect();
    assert_eq!(windows, [&json!("main"), &json!("main")]);
}

#[test]
fn replay_warns_on_each_step_that_relies_on_an_ephemeral_ref() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_recording(dir.path(), &recording());
    let (output, _) = run_against_mock(&["--json", "replay", &path], json!({"ok": true}));
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The old step still replays as before.
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");
    let warning = "[2/2] fill relies on snapshot ref e5, which only exists in the snapshot \
                   that numbered it; re-record for a stable replay";
    assert!(stderr.contains(warning), "--- stderr ---\n{stderr}");
    assert!(
        !stderr.contains("[1/2] check relies"),
        "--- stderr ---\n{stderr}"
    );

    let result: Value =
        serde_json::from_slice(&output.stdout).expect("--json prints the replay result");
    assert_eq!(
        result["steps"],
        json!([
            {"action": "check", "status": "passed"},
            {
                "action": "fill", "status": "passed",
                "warning": "relies on snapshot ref e5, which only exists in the snapshot \
                            that numbered it; re-record for a stable replay",
            },
        ])
    );
}

#[test]
fn record_stop_reports_the_steps_without_a_stable_locator() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("rec.json");
    let stop = json!({
        "count": 2,
        "entries": [
            {"action": "check", "timestamp": 0, "ref": "e9", "selector": "#pro", "expect": {"tag": "input"}},
            {"action": "click", "timestamp": 5, "ref": "e4", "expect": {"tag": "div"}},
        ],
        "unstable": [{"step": 2, "action": "click", "ref": "e4"}],
    });
    let (output, _) = run_against_mock(
        &["record", "stop", "--output", out.to_str().expect("UTF-8")],
        stop,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "--- stderr ---\n{stderr}");
    assert!(
        stdout.contains("step 2 (click): no stable locator for ref e4"),
        "--- stdout ---\n{stdout}"
    );
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("recording written"))
            .expect("recording is JSON");
    assert_eq!(saved.as_array().map(Vec::len), Some(2));
}
