//! Regression tests for #312: `replay --export` only converts a recording
//! file, so it must work with no Tauri app running, while a plain `replay`
//! still needs one.

#![cfg(unix)]

use std::path::Path;

use assert_cmd::Command;

/// A one-step recording with a stable selector.
const RECORDING: &str = r##"[{"action": "click", "selector": "#submit", "timestamp": 0}]"##;

/// Builds the CLI command with socket auto-detection pointed at `dir`.
///
/// `dir` holds no socket, so auto-detection only finds the stale sockets
/// other tests may leave in `/tmp`; the explicit-socket tests below do not
/// depend on that.
fn tauri_pilot(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("tauri-pilot").expect("tauri-pilot binary builds");
    cmd.env("XDG_RUNTIME_DIR", dir);
    cmd
}

/// Writes [`RECORDING`] to `dir/rec.json` and returns its path.
fn write_recording(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("rec.json");
    std::fs::write(&path, RECORDING).expect("write recording");
    path
}

#[test]
fn replay_export_sh_prints_script_without_a_socket() {
    let dir = tempfile::tempdir().expect("tempdir");
    let recording = write_recording(dir.path());
    let missing_socket = dir.path().join("missing.sock");

    let assert = tauri_pilot(dir.path())
        .arg("--socket")
        .arg(&missing_socket)
        .arg("replay")
        .arg(&recording)
        .args(["--export", "sh"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.starts_with("#!/bin/bash\n"),
        "expected a shell script on stdout, got: {stdout}"
    );
    assert!(
        stdout.contains("tauri-pilot click '#submit'"),
        "expected the recorded click in the script, got: {stdout}"
    );
}

#[test]
fn replay_export_sh_works_without_socket_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    let recording = write_recording(dir.path());

    let assert = tauri_pilot(dir.path())
        .arg("replay")
        .arg(&recording)
        .args(["--export", "sh"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("tauri-pilot click '#submit'"),
        "expected the recorded click in the script, got: {stdout}"
    );
}

#[test]
fn replay_export_unknown_format_fails_without_a_socket() {
    let dir = tempfile::tempdir().expect("tempdir");
    let recording = write_recording(dir.path());
    let missing_socket = dir.path().join("missing.sock");

    let assert = tauri_pilot(dir.path())
        .arg("--socket")
        .arg(&missing_socket)
        .arg("replay")
        .arg(&recording)
        .args(["--export", "py"])
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("unsupported export format: py"),
        "expected the format error, not a connection error, got: {stderr}"
    );
}

#[test]
fn replay_without_export_still_needs_an_app() {
    let dir = tempfile::tempdir().expect("tempdir");
    let recording = write_recording(dir.path());
    let missing_socket = dir.path().join("missing.sock");

    let assert = tauri_pilot(dir.path())
        .arg("--socket")
        .arg(&missing_socket)
        .arg("replay")
        .arg(&recording)
        .assert()
        .failure();

    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.is_empty(),
        "a replay with no app must not print a result, got: {stdout}"
    );
    assert!(
        stderr.contains("Cannot connect to socket"),
        "expected the connection error, got: {stderr}"
    );
}
