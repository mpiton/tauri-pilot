//! Tests for issue #305: TOML scenario steps for the storage and assertion
//! checks the CLI already has. `storage-get` reads sessionStorage with
//! `session = true` and compares the value with `expected`; `storage-set`,
//! `assert-checked`, `assert-unchecked`, `assert-count` and
//! `assert-contains` are new steps.
//!
//! Each test runs `tauri-pilot run` on a one-step scenario against a mock
//! JSON-RPC unix socket. The mock answers the step's method with a canned
//! result and any failure-capture `screenshot` with a dummy data URL.

#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;

use common::{SERVER_DONE_TIMEOUT, unique_socket_path, wait_bounded};
use serde_json::{Value, json};

/// What the binary did with one scenario step.
struct Run {
    output: Output,
    /// `(method, params)` of every request but the failure screenshot.
    requests: Vec<(String, Value)>,
}

impl Run {
    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }

    fn assert_passed(&self) {
        assert!(
            self.output.status.success(),
            "step must pass\n--- stderr ---\n{}",
            self.stderr()
        );
    }

    /// Checks the run exited 1 and its stderr holds every part of `want`.
    fn assert_failed_with(&self, want: &[&str]) {
        let stderr = self.stderr();
        assert_eq!(
            self.output.status.code(),
            Some(1),
            "step must fail the scenario\n--- stderr ---\n{stderr}"
        );
        for part in want {
            assert!(stderr.contains(part), "missing {part:?} in: {stderr}");
        }
    }

    /// Checks the step sent exactly one request, `method` with `params`.
    fn assert_sent(&self, method: &str, params: &Value) {
        assert_eq!(
            self.requests,
            [(method.to_owned(), params.clone())],
            "unexpected requests"
        );
    }
}

/// Runs a scenario whose only step is `step`, answering `method` with `result`.
fn run_step(step: &str, method: &'static str, result: &Value) -> Run {
    run_step_replying(step, method, json!({"result": result}))
}

/// Like [`run_step`], answering `method` with `reply`: `{"result": ...}` or
/// `{"error": ...}`.
fn run_step_replying(step: &str, method: &'static str, reply: Value) -> Run {
    let socket = unique_socket_path("scenario-steps");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind mock socket");
    let (done_tx, done_rx) = mpsc::channel();
    let (req_tx, req_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).expect("read line") == 0 {
                break;
            }
            let req: Value = serde_json::from_str(line.trim()).expect("parse request");
            let name = req["method"].as_str().unwrap_or_default().to_owned();
            let mut resp = if name == "screenshot" {
                json!({"result": "data:image/png;base64,AA=="})
            } else {
                let params = req.get("params").cloned().unwrap_or(Value::Null);
                let _ = req_tx.send((name.clone(), params));
                if name == method {
                    reply.clone()
                } else {
                    json!({"result": {"ok": true}})
                }
            };
            resp["jsonrpc"] = json!("2.0");
            resp["id"] = req["id"].clone();
            let mut bytes = serde_json::to_vec(&resp).expect("serialize");
            bytes.push(b'\n');
            writer.write_all(&bytes).expect("write");
            writer.flush().expect("flush");
        }
        let _ = done_tx.send(());
    });

    let tmpdir = tempfile::tempdir().expect("tempdir");
    let scenario_path = tmpdir.path().join("scenario.toml");
    std::fs::write(&scenario_path, format!("[[step]]\nname = \"s\"\n{step}\n"))
        .expect("write scenario");

    // Bounded: a CLI regression that hangs fails this test, not the suite.
    let child = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .current_dir(tmpdir.path())
        .arg("--socket")
        .arg(&socket)
        .arg("run")
        .arg(&scenario_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tauri-pilot");
    let output = wait_bounded(child);

    let done = done_rx.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    if let Err(err) = done {
        panic!(
            "mock server did not finish: {err}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Run {
        output,
        requests: req_rx.try_iter().collect(),
    }
}

// ── storage-get ──────────────────────────────────────────────────────────────

#[test]
fn storage_get_session_reads_session_storage() {
    let run = run_step(
        "action = \"storage-get\"\nkey = \"csrf_token\"\nsession = true",
        "storage.get",
        &json!({"found": true, "value": "abc"}),
    );
    run.assert_passed();
    run.assert_sent(
        "storage.get",
        &json!({"key": "csrf_token", "session": true}),
    );
}

#[test]
fn storage_get_session_missing_key_fails() {
    let run = run_step(
        "action = \"storage-get\"\nkey = \"csrf_token\"\nsession = true",
        "storage.get",
        &json!({"found": false}),
    );
    run.assert_failed_with(&["\"csrf_token\" was not found"]);
}

#[test]
fn storage_get_expected_matching_value_passes() {
    let run = run_step(
        "action = \"storage-get\"\nkey = \"theme\"\nexpected = \"dark\"",
        "storage.get",
        &json!({"found": true, "value": "dark"}),
    );
    run.assert_passed();
    run.assert_sent("storage.get", &json!({"key": "theme", "session": false}));
}

#[test]
fn storage_get_expected_other_value_fails_with_both() {
    let run = run_step(
        "action = \"storage-get\"\nkey = \"theme\"\nexpected = \"dark\"",
        "storage.get",
        &json!({"found": true, "value": "light"}),
    );
    run.assert_failed_with(&["expected \"dark\"", "got \"light\""]);
}

#[test]
fn storage_get_expected_without_string_value_fails() {
    let run = run_step(
        "action = \"storage-get\"\nkey = \"theme\"\nexpected = \"dark\"",
        "storage.get",
        &json!({"found": true}),
    );
    run.assert_failed_with(&["value"]);
}

// ── storage-set ──────────────────────────────────────────────────────────────

#[test]
fn storage_set_writes_local_storage() {
    let run = run_step(
        "action = \"storage-set\"\nkey = \"theme\"\nvalue = \"dark\"",
        "storage.set",
        &json!({"ok": true}),
    );
    run.assert_passed();
    run.assert_sent(
        "storage.set",
        &json!({"key": "theme", "value": "dark", "session": false}),
    );
}

#[test]
fn storage_set_session_writes_session_storage() {
    let run = run_step(
        "action = \"storage-set\"\nkey = \"tab\"\nvalue = \"\"\nsession = true",
        "storage.set",
        &json!({"ok": true}),
    );
    run.assert_passed();
    run.assert_sent(
        "storage.set",
        &json!({"key": "tab", "value": "", "session": true}),
    );
}

#[test]
fn storage_set_without_ok_result_fails() {
    let run = run_step(
        "action = \"storage-set\"\nkey = \"theme\"\nvalue = \"dark\"",
        "storage.set",
        &Value::Null,
    );
    run.assert_failed_with(&["storage.set returned invalid response"]);
}

#[test]
fn storage_set_rpc_error_fails() {
    let run = run_step_replying(
        "action = \"storage-set\"\nkey = \"theme\"\nvalue = \"dark\"",
        "storage.set",
        json!({"error": {"code": -32000, "message": "QuotaExceededError"}}),
    );
    run.assert_failed_with(&["QuotaExceededError"]);
}

// ── assert-checked / assert-unchecked ────────────────────────────────────────

#[test]
fn assert_checked_passes_on_checked_element() {
    let run = run_step(
        "action = \"assert-checked\"\ntarget = \"#notif-check\"",
        "checked",
        &json!({"checked": true}),
    );
    run.assert_passed();
    run.assert_sent("checked", &json!({"selector": "#notif-check"}));
}

#[test]
fn assert_checked_fails_on_unchecked_element() {
    let run = run_step(
        "action = \"assert-checked\"\ntarget = \"#notif-check\"",
        "checked",
        &json!({"checked": false}),
    );
    run.assert_failed_with(&["element is not checked"]);
}

#[test]
fn assert_unchecked_passes_on_unchecked_element() {
    let run = run_step(
        "action = \"assert-unchecked\"\ntarget = \"#notif-check\"",
        "checked",
        &json!({"checked": false}),
    );
    run.assert_passed();
    run.assert_sent("checked", &json!({"selector": "#notif-check"}));
}

#[test]
fn assert_unchecked_fails_on_checked_element() {
    let run = run_step(
        "action = \"assert-unchecked\"\ntarget = \"#notif-check\"",
        "checked",
        &json!({"checked": true}),
    );
    run.assert_failed_with(&["element is checked"]);
}

// ── assert-count ─────────────────────────────────────────────────────────────

#[test]
fn assert_count_passes_on_matching_count() {
    let run = run_step(
        "action = \"assert-count\"\nselector = \"li.item\"\nexpected = 3",
        "count",
        &json!({"count": 3}),
    );
    run.assert_passed();
    run.assert_sent("count", &json!({"selector": "li.item"}));
}

#[test]
fn assert_count_fails_with_expected_and_found() {
    let run = run_step(
        "action = \"assert-count\"\nselector = \"li.item\"\nexpected = 3",
        "count",
        &json!({"count": 2}),
    );
    run.assert_failed_with(&["expected 3 elements, found 2"]);
}

// ── assert-contains ──────────────────────────────────────────────────────────

#[test]
fn assert_contains_passes_on_substring() {
    let run = run_step(
        "action = \"assert-contains\"\ntarget = \"#status\"\nexpected = \"saved\"",
        "text",
        &json!("Profile saved at 10:02"),
    );
    run.assert_passed();
    run.assert_sent("text", &json!({"selector": "#status"}));
}

#[test]
fn assert_contains_fails_with_expected_and_actual() {
    let run = run_step(
        "action = \"assert-contains\"\ntarget = \"#status\"\nexpected = \"saved\"",
        "text",
        &json!("Saving failed"),
    );
    run.assert_failed_with(&["text does not contain \"saved\"", "got \"Saving failed\""]);
}

// ── assert-text / assert-value / assert-url / assert-visible / assert-hidden ─
//
// These steps run the `assert` command's checks, so a response without the
// field the check reads fails the step instead of passing on a default.

#[test]
fn assert_text_non_string_response_fails() {
    let run = run_step(
        "action = \"assert-text\"\ntarget = \"#title\"\nexpected = \"\"",
        "text",
        &Value::Null,
    );
    run.assert_failed_with(&["expected string response from server"]);
}

#[test]
fn assert_text_fails_with_the_assert_command_message() {
    let run = run_step(
        "action = \"assert-text\"\ntarget = \"#title\"\nexpected = 'say \"hi\"'",
        "text",
        &json!("say hello"),
    );
    run.assert_failed_with(&["expected text \"say \"hi\"\", got \"say hello\""]);
}

#[test]
fn assert_value_non_string_response_fails() {
    let run = run_step(
        "action = \"assert-value\"\ntarget = \"#email\"\nexpected = \"\"",
        "value",
        &Value::Null,
    );
    run.assert_failed_with(&["expected string response from server"]);
}

#[test]
fn assert_url_non_string_response_fails() {
    let run = run_step(
        "action = \"assert-url\"\nexpected = \"\"",
        "url",
        &Value::Null,
    );
    run.assert_failed_with(&["expected string response from server"]);
}

#[test]
fn assert_visible_without_visible_field_fails_naming_it() {
    let run = run_step(
        "action = \"assert-visible\"\ntarget = \"#panel\"",
        "visible",
        &json!({}),
    );
    run.assert_failed_with(&["missing 'visible' field"]);
}

#[test]
fn assert_hidden_without_visible_field_fails_naming_it() {
    let run = run_step(
        "action = \"assert-hidden\"\ntarget = \"#panel\"",
        "visible",
        &json!({}),
    );
    run.assert_failed_with(&["missing 'visible' field"]);
}
