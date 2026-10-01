//! Regression test for #281: `assert hidden` and the scenario step
//! `assert-hidden` must ask the bridge's `visible` to treat a selector that
//! matches nothing as hidden (`missingOk: true`), and `assert visible`,
//! `assert-visible` and `assert-exists` must not.
//!
//! A mock JSON-RPC unix socket records every request and answers `visible`
//! with `{"visible": false}`, then the binary is run against it.

#![cfg(unix)]

mod common;

// Rust guideline compliant 2026-02-21
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;

use common::{SERVER_DONE_TIMEOUT, unique_socket_path, wait_bounded};

/// Runs `tauri-pilot <args>` against a mock and returns the `visible` params.
///
/// # Panics
///
/// Panics if the socket cannot be bound or the mock server does not finish.
fn visible_params_sent<I, S>(tag: &str, cwd: &Path, args: I) -> (Output, Vec<serde_json::Value>)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let socket = unique_socket_path(tag);
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind mock socket");
    let (done_tx, done_rx) = mpsc::channel();
    let (params_tx, params_rx) = mpsc::channel();
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
            let body = if req["method"] == "visible" {
                let _ = params_tx.send(req["params"].clone());
                serde_json::json!({"visible": false})
            } else {
                serde_json::json!({"ok": true})
            };
            let resp = serde_json::json!({"jsonrpc": "2.0", "id": req["id"], "result": body});
            let mut bytes = serde_json::to_vec(&resp).expect("serialize");
            bytes.push(b'\n');
            writer.write_all(&bytes).expect("write");
            writer.flush().expect("flush");
        }
        let _ = done_tx.send(());
    });

    let child = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .current_dir(cwd)
        .arg("--socket")
        .arg(&socket)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tauri-pilot");
    let output = wait_bounded(child);
    let served = done_rx.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    assert!(
        served.is_ok(),
        "mock server did not finish: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    (output, params_rx.try_iter().collect())
}

#[test]
fn assert_hidden_sends_missing_ok() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (output, params) = visible_params_sent(
        "assert-hidden",
        dir.path(),
        ["assert", "hidden", "#does-not-exist"],
    );
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(params.len(), 1, "one visible request, got {params:?}");
    assert_eq!(params[0]["selector"], "#does-not-exist");
    assert_eq!(params[0]["missingOk"], true, "params={}", params[0]);
}

#[test]
fn assert_visible_does_not_send_missing_ok() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, params) = visible_params_sent(
        "assert-visible",
        dir.path(),
        ["assert", "visible", "#does-not-exist"],
    );
    assert_eq!(params.len(), 1, "one visible request, got {params:?}");
    assert!(
        params[0].get("missingOk").is_none(),
        "assert visible must keep failing on a missing element, params={}",
        params[0]
    );
}

/// Writes a one-step scenario running `action` on `#modal` and runs it.
///
/// The path goes to the binary as an `OsStr`, so a non-UTF-8 temp dir works.
///
/// # Panics
///
/// Panics if the temp dir or scenario file cannot be created.
fn scenario_visible_params_sent(tag: &str, action: &str) -> (Output, Vec<serde_json::Value>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let scenario = dir.path().join("scenario.toml");
    std::fs::write(
        &scenario,
        format!(
            r##"
[scenario]
name = "{action}"
[[step]]
name = "modal step"
action = "{action}"
target = "#modal"
"##
        ),
    )
    .expect("write scenario");
    visible_params_sent(tag, dir.path(), [OsStr::new("run"), scenario.as_os_str()])
}

#[test]
fn scenario_assert_hidden_sends_missing_ok() {
    let (output, params) = scenario_visible_params_sent("scenario-hidden", "assert-hidden");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(params.len(), 1, "one visible request, got {params:?}");
    assert_eq!(params[0]["selector"], "#modal");
    assert_eq!(params[0]["missingOk"], true, "params={}", params[0]);
}

#[test]
fn scenario_assert_exists_does_not_send_missing_ok() {
    let (_, params) = scenario_visible_params_sent("scenario-exists", "assert-exists");
    assert_eq!(params.len(), 1, "one visible request, got {params:?}");
    assert_eq!(params[0]["selector"], "#modal");
    assert!(
        params[0].get("missingOk").is_none(),
        "assert-exists must keep failing on a missing element, params={}",
        params[0]
    );
}

#[test]
fn scenario_assert_visible_does_not_send_missing_ok() {
    let (_, params) = scenario_visible_params_sent("scenario-visible", "assert-visible");
    assert_eq!(params.len(), 1, "one visible request, got {params:?}");
    assert!(
        params[0].get("missingOk").is_none(),
        "assert-visible must keep failing on a missing element, params={}",
        params[0]
    );
}
