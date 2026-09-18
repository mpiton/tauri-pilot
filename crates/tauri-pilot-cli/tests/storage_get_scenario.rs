//! Regression test for issue #184: a TOML `storage-get` step must fail when
//! the key is missing (`found: false`), while a key holding an empty string
//! still passes. Until the action exists, the runner bails with
//! `unknown step action` before it can apply that check.
//!
//! Same harness as `storage_get_exit.rs`: a mock JSON-RPC unix socket answers
//! `storage.get` (and a failure-screenshot `screenshot` if the step fails),
//! then the binary is run via `assert_cmd`.

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

/// Run `tauri-pilot run` against a one-step `storage-get` scenario.
///
/// The mock answers `storage.get` with `result` and any later `screenshot`
/// (failure capture) with a dummy data URL so the client is not left hanging.
fn run_storage_get_scenario(
    key: &str,
    result: serde_json::Value,
) -> (Output, Vec<String>, Vec<serde_json::Value>) {
    let socket = unique_socket_path("storage-get-scenario");
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
            let body = if method == "storage.get" {
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
name = "storage-get"
[[step]]
name = "read key"
action = "storage-get"
key = "{key}"
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

fn assert_storage_get_called(methods: &[String], params: &[serde_json::Value], key: &str) {
    assert!(
        methods.iter().any(|m| m == "storage.get"),
        "storage-get step must call storage.get, got {methods:?}"
    );
    assert_eq!(
        params.len(),
        1,
        "expected one storage.get request, got {params:?}"
    );
    assert_eq!(params[0]["key"], key, "storage.get key");
    assert_eq!(params[0]["session"], false, "storage.get uses localStorage");
}

#[test]
fn storage_get_scenario_missing_key_fails() {
    let (output, methods, params) =
        run_storage_get_scenario("missing-key", serde_json::json!({"found": false}));

    assert_storage_get_called(&methods, &params, "missing-key");
    assert_eq!(
        output.status.code(),
        Some(1),
        "missing key must fail the scenario"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unknown step action"),
        "storage-get must be a real action, not unknown: {stderr}"
    );
    assert!(
        stderr.contains("missing-key") && stderr.contains("not found"),
        "failure must name the missing key, got: {stderr}"
    );
}

#[test]
fn storage_get_scenario_empty_value_passes() {
    let (output, methods, params) =
        run_storage_get_scenario("empty-key", serde_json::json!({"found": true, "value": ""}));

    assert_storage_get_called(&methods, &params, "empty-key");
    assert!(
        output.status.success(),
        "present-but-empty key must pass the scenario\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn storage_get_scenario_missing_found_fails() {
    let (output, methods, params) =
        run_storage_get_scenario("some-key", serde_json::json!({"value": ""}));

    assert_storage_get_called(&methods, &params, "some-key");
    assert_eq!(
        output.status.code(),
        Some(1),
        "a response without boolean found must fail the scenario"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("found"),
        "failure must name the missing found field, got: {stderr}"
    );
    assert!(
        !stderr.contains("not found"),
        "missing found is not a missing key, got: {stderr}"
    );
}
