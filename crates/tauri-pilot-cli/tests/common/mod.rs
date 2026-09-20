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
        let resp = serde_json::json!({"jsonrpc": "2.0", "id": req["id"], "result": result});
        let mut bytes = serde_json::to_vec(&resp).expect("serialize");
        bytes.push(b'\n');
        writer.write_all(&bytes).expect("write");
        writer.flush().expect("flush");
        let _ = done_tx.send(());
    });
    done_rx
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
