//! Test for issue #284: a TOML `storage-delete` step removes one localStorage
//! key and passes whether or not the key existed, like `storage delete`. A
//! response without a boolean `deleted` fails the step.
//!
//! Same harness as `storage_get_scenario.rs`: a mock JSON-RPC unix socket
//! answers `storage.delete` (and a failure-screenshot `screenshot` if the step
//! fails), then the binary is run via `assert_cmd`.

#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::Output;
use std::sync::mpsc;
use std::thread;

use assert_cmd::Command;
use common::{SERVER_DONE_TIMEOUT, unique_socket_path};

/// Run `tauri-pilot run` against a one-step `storage-delete` scenario.
///
/// The mock answers `storage.delete` with `result` and any later `screenshot`
/// (failure capture) with a dummy data URL so the client is not left hanging.
fn run_storage_delete_scenario(
    key: &str,
    result: serde_json::Value,
) -> (Output, Vec<String>, Vec<serde_json::Value>) {
    run_storage_delete_scenario_with(key, "", result)
}

/// Like [`run_storage_delete_scenario`], with `extra` TOML appended to the step.
fn run_storage_delete_scenario_with(
    key: &str,
    extra: &str,
    result: serde_json::Value,
) -> (Output, Vec<String>, Vec<serde_json::Value>) {
    let socket = unique_socket_path("storage-delete-scenario");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind mock socket");
    let (done_tx, done_rx) = mpsc::channel();
    let (methods_tx, methods_rx) = mpsc::channel();
    let (params_tx, params_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).expect("read line");
            if n == 0 {
                break;
            }
            let req: serde_json::Value = serde_json::from_str(line.trim()).expect("parse request");
            let method = req["method"].as_str().unwrap_or_default().to_owned();
            let _ = methods_tx.send(method.clone());
            let body = if method == "storage.delete" {
                let _ = params_tx.send(
                    req.get("params")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                );
                result.clone()
            } else if method == "screenshot" {
                serde_json::json!("data:image/png;base64,AA==")
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

    let tmpdir = tempfile::tempdir().expect("tempdir");
    let scenario_path = tmpdir.path().join("scenario.toml");
    std::fs::write(
        &scenario_path,
        format!(
            r#"
[scenario]
name = "storage-delete"
[[step]]
name = "drop key"
action = "storage-delete"
key = "{key}"
{extra}
"#
        ),
    )
    .expect("write scenario");

    let output = Command::cargo_bin("tauri-pilot")
        .expect("cargo_bin")
        .current_dir(tmpdir.path())
        .args([
            "--socket",
            socket.to_str().expect("socket path is UTF-8"),
            "run",
            scenario_path.to_str().expect("scenario path is UTF-8"),
        ])
        .output()
        .expect("run tauri-pilot");

    let done = done_rx.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    if let Err(err) = done {
        panic!(
            "mock server did not finish: {err}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let methods: Vec<String> = methods_rx.try_iter().collect();
    let storage_params: Vec<serde_json::Value> = params_rx.try_iter().collect();
    (output, methods, storage_params)
}

fn assert_storage_delete_called(methods: &[String], params: &[serde_json::Value], key: &str) {
    assert_storage_delete_called_in(methods, params, key, false);
}

/// Checks the step sent one `storage.delete` for `key` with this `session`.
fn assert_storage_delete_called_in(
    methods: &[String],
    params: &[serde_json::Value],
    key: &str,
    session: bool,
) {
    assert!(
        methods.iter().any(|m| m == "storage.delete"),
        "storage-delete step must call storage.delete, got {methods:?}"
    );
    assert_eq!(
        params,
        [serde_json::json!({"key": key, "session": session})],
        "storage-delete sends one request to the selected storage"
    );
}

#[test]
fn storage_delete_scenario_existing_key_passes() {
    let (output, methods, params) =
        run_storage_delete_scenario("auth_token", serde_json::json!({"deleted": true}));

    assert_storage_delete_called(&methods, &params, "auth_token");
    assert!(
        output.status.success(),
        "deleting an existing key must pass\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn storage_delete_scenario_missing_key_passes() {
    let (output, methods, params) =
        run_storage_delete_scenario("missing-key", serde_json::json!({"deleted": false}));

    assert_storage_delete_called(&methods, &params, "missing-key");
    assert!(
        output.status.success(),
        "deleting a missing key must pass\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn storage_delete_scenario_missing_deleted_fails() {
    let (output, methods, params) =
        run_storage_delete_scenario("some-key", serde_json::json!({"ok": true}));

    assert_storage_delete_called(&methods, &params, "some-key");
    assert_eq!(
        output.status.code(),
        Some(1),
        "a response without boolean deleted must fail the scenario"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("deleted"),
        "failure must name the missing deleted field, got: {stderr}"
    );
}

#[test]
fn storage_delete_scenario_session_targets_session_storage() {
    let (output, methods, params) = run_storage_delete_scenario_with(
        "tab_id",
        "session = true",
        serde_json::json!({"deleted": true}),
    );

    assert!(
        output.status.success(),
        "a sessionStorage delete must pass\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_storage_delete_called_in(&methods, &params, "tab_id", true);
}
