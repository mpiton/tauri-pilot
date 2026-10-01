//! Regression test for issue #286: `assert unchecked` passes when the bridge
//! reports `checked: false` and fails with exit 1 when it reports `true`.
//!
//! Same harness as `storage_get_exit.rs`: a one-shot mock JSON-RPC unix
//! socket server answers the single request, then the binary runs against that
//! socket via `assert_cmd`.

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
///
/// The binary only exits after reading the reply, so the server is done or a
/// few instructions away from it; the margin covers slow CI hosts. A binary
/// that exits before connecting leaves the server blocked on `accept()`, and
/// this bound turns that into a test failure instead of a hung suite.
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

/// Run `assert unchecked <target>` against a mock bridge whose `checked`
/// method answers `{"checked": checked}`.
fn run_assert_unchecked(target: &'static str, checked: bool) -> Output {
    let socket = unique_socket_path("assert-unchecked");
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
        assert_eq!(req["method"], "checked");
        assert_eq!(req["params"]["selector"], target);
        let resp = serde_json::json!({
            "jsonrpc": "2.0",
            "id": req["id"],
            "result": {"checked": checked},
        });
        let mut bytes = serde_json::to_vec(&resp).expect("serialize");
        bytes.push(b'\n');
        writer.write_all(&bytes).expect("write");
        writer.flush().expect("flush");
        let _ = done_tx.send(());
    });

    let output = Command::cargo_bin("tauri-pilot")
        .expect("cargo_bin")
        .args([
            "--socket",
            socket.to_str().expect("socket path is UTF-8"),
            "assert",
            "unchecked",
            target,
        ])
        .output()
        .expect("run tauri-pilot");

    // Disconnected means the server panicked (e.g. wrong method); timeout means
    // the binary never connected.
    let done = done_rx.recv_timeout(SERVER_DONE_TIMEOUT);
    let _ = std::fs::remove_file(&socket);
    if let Err(err) = done {
        panic!(
            "mock server did not answer checked: {err}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output
}

#[test]
fn assert_unchecked_passes_when_not_checked() {
    let output = run_assert_unchecked("#remember", false);

    assert!(
        output.status.success(),
        "unchecked box must exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("ok"));
}

#[test]
fn assert_unchecked_fails_with_exit_1_when_checked() {
    let output = run_assert_unchecked("#remember", true);

    assert_eq!(output.status.code(), Some(1), "checked box must exit 1");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("FAIL: element is checked"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
