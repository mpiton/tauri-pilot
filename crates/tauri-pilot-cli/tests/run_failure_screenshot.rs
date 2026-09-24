//! Regression test for issue #215: a failed `run` step must say where its
//! screenshot went, and `--screenshots-dir` must decide where that is.
//!
//! A mock JSON-RPC unix socket fails the `click` step and answers the
//! follow-up `screenshot`, then the binary is spawned against it.

#![cfg(unix)]

mod common;

// Rust guideline compliant 2026-08-29
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;

use common::{SERVER_DONE_TIMEOUT, closed_pipe, unique_socket_path, wait_bounded};

/// Runs a one-step scenario whose `click` fails, with extra CLI arguments.
///
/// # Panics
///
/// Panics if the socket cannot be bound or the mock server does not finish.
fn run_failing_scenario(cwd: &Path, extra_args: &[&str]) -> Output {
    run_failing_scenario_with_stdout(cwd, extra_args, Stdio::piped())
}

/// Same, with `stdout` wired to the caller's choice.
///
/// `common::spawn_mock_server` answers a single request; a failing step sends
/// `click` and then `screenshot`, so this keeps a local looping mock.
///
/// # Panics
///
/// Panics if the socket cannot be bound or the mock server does not finish.
fn run_failing_scenario_with_stdout(cwd: &Path, extra_args: &[&str], stdout: Stdio) -> Output {
    let socket = unique_socket_path("run-shot");
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
            let resp = match req["method"].as_str().unwrap_or_default() {
                "click" => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req["id"],
                    "error": {"code": -32000, "message": "click failed"},
                }),
                "screenshot" => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req["id"],
                    "result": "data:image/png;base64,AA==",
                }),
                _ => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req["id"],
                    "result": {"ok": true},
                }),
            };
            let mut bytes = serde_json::to_vec(&resp).expect("serialize");
            bytes.push(b'\n');
            writer.write_all(&bytes).expect("write");
            writer.flush().expect("flush");
        }
        let _ = done_tx.send(());
    });

    let scenario_path = cwd.join("scenario.toml");
    std::fs::write(
        &scenario_path,
        r##"
[scenario]
name = "failing"
[[step]]
name = "click missing"
action = "click"
target = "#nope"
"##,
    )
    .expect("write scenario");

    let mut args = vec![
        "--socket",
        socket.to_str().expect("socket path is UTF-8"),
        "run",
        scenario_path.to_str().expect("scenario path is UTF-8"),
    ];
    args.extend_from_slice(extra_args);
    let child = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .current_dir(cwd)
        .args(args)
        .stdout(stdout)
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
    output
}

/// Reads the `--json` report from a run that is expected to fail.
fn failing_report(output: &Output) -> serde_json::Value {
    assert_eq!(output.status.code(), Some(1), "failing scenario exits 1");
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("--json must print a JSON report ({err}): {stdout}"))
}

/// Returns the `screenshot` path the first step reported.
fn reported_screenshot(report: &serde_json::Value) -> PathBuf {
    let saved = report["steps"][0]["screenshot"]
        .as_str()
        .unwrap_or_else(|| panic!("failed step must report its screenshot: {report}"));
    PathBuf::from(saved)
}

#[test]
fn run_json_reports_screenshot_from_requested_directory() {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let shots = tmpdir.path().join("shots");
    let junit = tmpdir.path().join("results.xml");
    let output = run_failing_scenario(
        tmpdir.path(),
        &[
            "--json",
            "--screenshots-dir",
            shots.to_str().expect("shots path is UTF-8"),
            "--junit",
            junit.to_str().expect("junit path is UTF-8"),
        ],
    );

    let report = failing_report(&output);
    let saved = reported_screenshot(&report);
    assert!(
        saved.starts_with(&shots),
        "screenshot {} ignored --screenshots-dir {}",
        saved.display(),
        shots.display()
    );
    assert!(saved.is_file(), "screenshot {} is missing", saved.display());
    assert!(
        !tmpdir.path().join("tauri-pilot-failures").exists(),
        "nothing should land in the working directory"
    );

    let xml = std::fs::read_to_string(&junit).expect("read junit xml");
    assert!(
        xml.contains(&format!(
            "<system-out>failure screenshot: {}</system-out>",
            saved.display()
        )),
        "junit must carry the screenshot path: {xml}"
    );
}

/// Without `--screenshots-dir` the shot lands in `./tauri-pilot-failures`, and
/// the report still names it absolutely — the other half of #215.
#[test]
fn run_json_reports_absolute_path_under_the_default_directory() {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    // `std::path::absolute` joins `getcwd()`, which on macOS resolves the
    // `/var` -> `/private/var` symlink the tempdir path keeps.
    let cwd = tmpdir.path().canonicalize().expect("canonicalize tempdir");
    let output = run_failing_scenario(&cwd, &["--json"]);

    let report = failing_report(&output);
    let saved = reported_screenshot(&report);
    assert!(
        saved.is_absolute(),
        "reported screenshot {} is not absolute",
        saved.display()
    );
    let expected = cwd.join("tauri-pilot-failures");
    assert!(
        saved.starts_with(&expected),
        "screenshot {} is not under the default directory {}",
        saved.display(),
        expected.display()
    );
    assert!(saved.is_file(), "screenshot {} is missing", saved.display());
}

/// A step with a key its action does not read fails before `run` connects, so
/// the valid step ahead of it never runs and no failure screenshot is taken
/// (#243).
#[test]
fn run_rejects_an_invalid_step_before_connecting() {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let scenario_path = tmpdir.path().join("sel.toml");
    std::fs::write(
        &scenario_path,
        "[[step]]\naction = \"click\"\ntarget = \"#go\"\n\n\
         [[step]]\naction = \"assert-exists\"\nselector = \"#login-form\"\n",
    )
    .expect("write scenario");
    // A live socket that never answers: a connect lands in the accept queue,
    // checked below, and `--rpc-timeout` ends the run instead of a hang.
    let socket = unique_socket_path("run-invalid-step");
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind mock socket");
    let child = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .current_dir(tmpdir.path())
        .args([
            "--socket",
            socket.to_str().expect("socket path is UTF-8"),
            "--rpc-timeout",
            "1",
            "run",
            scenario_path.to_str().expect("scenario path is UTF-8"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tauri-pilot");
    let output = wait_bounded(child);

    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let accepted = listener.accept();
    let _ = std::fs::remove_file(&socket);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr={stderr}");
    assert!(
        matches!(&accepted, Err(err) if err.kind() == std::io::ErrorKind::WouldBlock),
        "run connected before rejecting the scenario: {accepted:?}"
    );
    assert!(
        stderr.contains("step 2: step 'assert-exists' does not accept 'selector'; use 'target'"),
        "stderr={stderr}"
    );
    assert!(
        !tmpdir.path().join("tauri-pilot-failures").exists(),
        "an invalid scenario must not leave a failure screenshot"
    );
}

/// `run --json | head -1` must not turn a failing scenario into a 0, and the
/// `JUnit` XML must survive the early exit (#213 meeting #215).
#[test]
fn run_json_into_closed_pipe_keeps_exit_1_and_writes_junit() {
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let shots = tmpdir.path().join("shots");
    let junit = tmpdir.path().join("results.xml");
    let output = run_failing_scenario_with_stdout(
        tmpdir.path(),
        &[
            "--json",
            "--screenshots-dir",
            shots.to_str().expect("shots path is UTF-8"),
            "--junit",
            junit.to_str().expect("junit path is UTF-8"),
        ],
        closed_pipe(),
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "a failing scenario must exit 1 even when stdout is closed: stderr={stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "closed stdout pipe must not print a panic: stderr={stderr}"
    );
    let xml = std::fs::read_to_string(&junit).expect("junit xml must be written anyway");
    assert!(
        xml.contains("<failure"),
        "junit must record the failure: {xml}"
    );
}
