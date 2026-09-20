//! #230: a watched signal must still end the app, not just unlink the socket.
//!
//! The unlink half is cheap to cover in process, and the plugin's unit tests
//! do. The exit status is not: every mock app shares the test process, so a
//! re-raise there ends the whole run. Each test below re-execs this binary as a
//! child app instead, signals it, and asserts on the status it dies with as
//! well as on the socket file it leaves behind.
#![cfg(all(unix, not(target_os = "android"), debug_assertions))]

// Rust guideline compliant 2026-08-29

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Carries the mock app's identifier to the child, and marks it as the child.
const CHILD_ENV: &str = "TAURI_PILOT_ISSUE230_IDENTIFIER";

/// Printed by the child once the plugin has bound and armed its watcher.
const READY: &str = "issue230-ready";

/// How long the child may take to die of the signal it was sent.
///
/// Generous: it only has to cover a loaded CI runner waking one tokio task.
/// A watcher that swallows the signal never exits, so this bounds how long
/// the regression takes to report rather than tuning a race.
const EXIT_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn sigint_ends_the_app_with_the_signal_status() {
    signal_ends_the_app(libc::SIGINT, "int");
}

#[test]
fn sigterm_ends_the_app_with_the_signal_status() {
    signal_ends_the_app(libc::SIGTERM, "term");
}

#[test]
fn sighup_ends_the_app_with_the_signal_status() {
    signal_ends_the_app(libc::SIGHUP, "hup");
}

/// Asserts `signum` kills a child app and unlinks its socket.
///
/// `tag` keeps the socket directory of each caller distinct.
fn signal_ends_the_app(signum: i32, tag: &str) {
    // A private XDG_RUNTIME_DIR of our own makes the child's socket path
    // predictable here, without reaching for the crate-private
    // `server::socket_address`. Under /tmp rather than `temp_dir()`: macOS
    // returns a path long enough to push the socket past the `sun_path` limit.
    let dir = PathBuf::from(format!("/tmp/tp230-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create socket dir");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .expect("socket dir must be private, else the plugin falls back to /tmp");
    let identifier = format!("tp230-{tag}");
    let socket = dir.join(format!("tauri-pilot-{identifier}.sock"));

    let mut child = spawn_child_app(&dir, &identifier);
    let ready = wait_for_ready(&mut child);
    let bound = socket.exists();

    let pid = i32::try_from(child.id()).expect("child pid fits in pid_t");
    // SAFETY: `kill` is a libc entry point with no preconditions.
    let sent = unsafe { libc::kill(pid, signum) };
    let status = wait_for_exit(&mut child, EXIT_TIMEOUT);

    let left = socket.exists();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(ready, "child app never reported ready");
    assert!(bound, "child app must bind a pathname socket");
    assert_eq!(sent, 0, "failed to send signal {signum} to the child app");
    let Some(status) = status else {
        panic!("signal {signum} must end the app, it still ran after {EXIT_TIMEOUT:?} (#230)");
    };
    assert!(!left, "signal {signum} must unlink the socket (#217)");
    assert_eq!(
        status.signal(),
        Some(signum),
        "app must die of signal {signum}, with the usual 128+n status (#230)"
    );
}

/// Re-execs this test binary as a mock app bound in `socket_dir`.
fn spawn_child_app(socket_dir: &Path, identifier: &str) -> Child {
    Command::new(std::env::current_exe().expect("test binary path"))
        .args(["child_app", "--exact", "--ignored", "--nocapture"])
        .env(CHILD_ENV, identifier)
        .env("XDG_RUNTIME_DIR", socket_dir)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn child app")
}

/// Reads the child's output until it reports ready, or its stdout ends.
fn wait_for_ready(child: &mut Child) -> bool {
    let stdout = child.stdout.take().expect("piped stdout");
    BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .any(|line| line.contains(READY))
}

/// Waits up to `timeout` for the child to exit, killing it if it outlives that.
fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().expect("poll child app") {
            Some(status) => return Some(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            // Short enough that the tests stay quick, long enough not to spin.
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// The app under test, re-exec'd by the tests above.
///
/// Ignored so a plain `cargo test` never runs it. Without `CHILD_ENV` (a hand
/// written `--ignored` run) there is no parent to serve, so it does nothing.
#[test]
#[ignore = "child process of the signal tests, needs TAURI_PILOT_ISSUE230_IDENTIFIER"]
fn child_app() {
    let Some(identifier) = std::env::var_os(CHILD_ENV) else {
        return;
    };

    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().identifier = identifier.to_string_lossy().into_owned();
    let _app = tauri::test::mock_builder()
        .plugin(tauri_plugin_pilot::init())
        .build(context)
        .expect("mock app starts");

    println!("{READY}");
    std::io::stdout().flush().expect("flush ready line");

    // The watcher runs on tauri's runtime, so park this thread and let the
    // parent's signal end the process.
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}
