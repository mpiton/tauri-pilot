//! Regression test for issue #213: when stdout is a closed pipe (e.g.
//! `tauri-pilot snapshot | head -1`), the CLI must exit quietly with status 0
//! instead of panicking with "failed printing to stdout: Broken pipe".

#![cfg(unix)]

// Rust guideline compliant 2026-08-29
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

#[test]
fn snapshot_into_closed_pipe_exits_quietly() {
    let socket = PathBuf::from(format!(
        "/tmp/tauri-pilot-it-epipe-{}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).expect("bind mock socket");
    let (closed_tx, closed_rx) = mpsc::channel::<()>();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut writer = stream;
        let mut line = String::new();
        reader.read_line(&mut line).expect("read line");
        let req: serde_json::Value = serde_json::from_str(line.trim()).expect("parse request");
        // Answer only once the test has closed the read end of stdout, so the
        // CLI's first write is guaranteed to hit a closed pipe.
        closed_rx.recv().expect("stdout closed signal");
        let resp = serde_json::json!({
            "jsonrpc": "2.0",
            "id": req.get("id").cloned().unwrap_or(serde_json::Value::Null),
            "result": {
                "elements": [
                    {"depth": 0, "role": "heading", "name": "Pilot Test App", "ref": "e1"},
                    {"depth": 0, "role": "button", "name": "Click", "ref": "e2"}
                ]
            }
        });
        let mut bytes = serde_json::to_vec(&resp).expect("serialize");
        bytes.push(b'\n');
        writer.write_all(&bytes).expect("write");
        writer.flush().expect("flush");
    });

    let mut child = Command::new(env!("CARGO_BIN_EXE_tauri-pilot"))
        .args([
            "--socket",
            socket.to_str().expect("socket path is UTF-8"),
            "snapshot",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tauri-pilot");
    drop(child.stdout.take());
    let _ = closed_tx.send(());
    let output = child.wait_with_output().expect("wait for tauri-pilot");
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Fail fast before join() so a binary that exits before connecting cannot
    // leave the mock server blocked on accept() and hang the test.
    if !output.status.success() {
        let _ = std::fs::remove_file(&socket);
        panic!(
            "expected exit 0 on a closed stdout pipe, got {:?}: stderr={stderr}",
            output.status.code()
        );
    }
    server.join().expect("mock server join");
    let _ = std::fs::remove_file(&socket);
    assert!(
        !stderr.contains("panicked") && !stderr.contains("Broken pipe"),
        "closed stdout pipe must not print a panic: stderr={stderr}"
    );
}
