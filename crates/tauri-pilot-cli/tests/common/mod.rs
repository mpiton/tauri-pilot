//! Shared harness for CLI integration tests: a one-shot mock JSON-RPC server
//! on a unix socket, plus the process helpers that drive the binary against
//! it.

// Each test binary compiles this module on its own and uses a different
// subset of it, so `expect` would fire "unfulfilled expectation" in the
// binaries that do use everything.
#![allow(dead_code)]

// Rust guideline compliant 2026-08-29
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long to wait for the mock server, or the binary, to finish.
///
/// The binary only exits after reading the reply, so the server is done or a
/// few instructions away from it; the margin covers slow CI hosts. A binary
/// that exits before connecting leaves the server blocked on `accept()`, and
/// this bound turns that into a test failure instead of a hung suite.
pub const SERVER_DONE_TIMEOUT: Duration = Duration::from_secs(10);

static SOCK_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Returns a socket path unique to this process and call.
pub fn unique_socket_path(tag: &str) -> PathBuf {
    let n = SOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!(
        "/tmp/tauri-pilot-it-{}-{}-{}.sock",
        tag,
        std::process::id(),
        n
    ))
}

/// Serves one JSON-RPC request on `socket`, answering with `result`.
///
/// The returned receiver gets a message once the reply is flushed; it
/// disconnects if the server thread panics.
///
/// # Panics
///
/// Panics if the socket cannot be bound.
pub fn spawn_mock_server(socket: &Path, result: serde_json::Value) -> mpsc::Receiver<()> {
    spawn_mock_responder(socket, move |_| serde_json::json!({"result": result}))
}

/// Serves one JSON-RPC request on `socket`, answering with `respond(request)`.
///
/// `respond` returns the response body without `jsonrpc` and `id`, so either
/// `{"result": ...}` or `{"error": ...}`; it may also assert on the request.
/// The returned receiver gets a message once the reply is flushed; it
/// disconnects if the server thread panics, including on a failed assertion
/// inside `respond`.
///
/// # Panics
///
/// Panics if the socket cannot be bound.
pub fn spawn_mock_responder(
    socket: &Path,
    respond: impl FnOnce(&serde_json::Value) -> serde_json::Value + Send + 'static,
) -> mpsc::Receiver<()> {
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket).expect("bind mock socket");
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;
        let mut line = String::new();
        reader.read_line(&mut line).expect("read line");
        let req: serde_json::Value = serde_json::from_str(line.trim()).expect("parse request");
        let mut resp = respond(&req);
        resp["jsonrpc"] = "2.0".into();
        resp["id"] = req["id"].clone();
        let mut bytes = serde_json::to_vec(&resp).expect("serialize");
        bytes.push(b'\n');
        writer.write_all(&bytes).expect("write");
        writer.flush().expect("flush");
        let _ = done_tx.send(());
    });
    done_rx
}

/// Runs the binary with `args` against a mock that answers every request.
///
/// The mock serves one connection, answers each request on it with `result`
/// until the binary closes it, and records the requests. `tag` names the
/// socket. Returns the binary's output and the requests in order.
///
/// # Panics
///
/// Panics if the socket cannot be bound, the binary cannot be run, or the
/// mock server does not finish within `SERVER_DONE_TIMEOUT`.
pub fn run_against_looping_mock(
    tag: &str,
    args: &[&str],
    result: serde_json::Value,
) -> (Output, Vec<serde_json::Value>) {
    let socket = unique_socket_path(tag);
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
            let req: serde_json::Value = serde_json::from_str(line.trim()).expect("parse request");
            let resp = serde_json::json!({"jsonrpc": "2.0", "id": req["id"], "result": result});
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
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
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

/// Returns a stdout whose reader is already gone, so the first write fails
/// with `EPIPE`.
///
/// # Panics
///
/// Panics if the pipe cannot be created.
pub fn closed_pipe() -> Stdio {
    let (reader, writer) = std::io::pipe().expect("create pipe");
    drop(reader);
    writer.into()
}

/// Waits for `child` to exit within `SERVER_DONE_TIMEOUT`, then collects it.
///
/// # Panics
///
/// Panics if the child is still running at the deadline: a binary that hangs
/// on a closed stdout then fails its own test instead of the whole suite.
pub fn wait_bounded(mut child: Child) -> Output {
    let deadline = Instant::now() + SERVER_DONE_TIMEOUT;
    while child.try_wait().expect("poll tauri-pilot").is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("tauri-pilot did not exit within {SERVER_DONE_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().expect("wait for tauri-pilot")
}
