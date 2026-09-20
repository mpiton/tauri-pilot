//! Regression test for issue #215: a failed `run` step must say where its
//! screenshot went, and `--screenshots-dir` must decide where that is.
//!
//! A mock JSON-RPC unix socket fails the `click` step and answers the
//! follow-up `screenshot`, then the binary is run via `assert_cmd`.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use assert_cmd::Command;

/// How long to wait for the mock server once the binary has exited.
const SERVER_DONE_TIMEOUT: Duration = Duration::from_secs(10);

fn unique_socket_path() -> PathBuf {
    PathBuf::from(format!(
        "/tmp/tauri-pilot-it-run-shot-{}.sock",
        std::process::id()
    ))
}

/// Run a one-step scenario whose `click` fails, with extra CLI arguments.
fn run_failing_scenario(cwd: &Path, extra_args: &[&str]) -> Output {
    let socket = unique_socket_path();
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
    let output = Command::cargo_bin("tauri-pilot")
        .expect("cargo_bin")
        .current_dir(cwd)
        .args(args)
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
    output
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

    assert_eq!(output.status.code(), Some(1), "failing scenario exits 1");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("--json must print a JSON report ({err}): {stdout}"));
    let saved = report["steps"][0]["screenshot"]
        .as_str()
        .unwrap_or_else(|| panic!("failed step must report its screenshot: {report}"));
    assert!(
        Path::new(saved).starts_with(&shots),
        "screenshot {saved} ignored --screenshots-dir {}",
        shots.display()
    );
    assert!(Path::new(saved).is_file(), "screenshot {saved} is missing");
    assert!(
        !tmpdir.path().join("tauri-pilot-failures").exists(),
        "nothing should land in the working directory"
    );

    let xml = std::fs::read_to_string(&junit).expect("read junit xml");
    assert!(
        xml.contains(&format!(
            "<system-out>failure screenshot: {saved}</system-out>"
        )),
        "junit must carry the screenshot path: {xml}"
    );
}
