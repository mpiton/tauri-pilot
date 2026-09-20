//! #230: a watched signal must still end the app, not just unlink the socket.
//!
//! This file owns both halves of the signal path: the unlinked socket (#217)
//! and the 128+n exit status (#230). Neither is coverable in process: every
//! mock app shares the test process, so the re-raise that ends the app ends the
//! whole run with it. Each test below re-execs this binary as a child app
//! instead, signals it, and asserts on the socket file it leaves behind as well
//! as on the status it dies with. The one in-process test left in the plugin
//! covers `RunEvent::Exit` (#194), not signals.
//!
//! Windows has the same path under another name (#229): a console control
//! event, the instance file in place of the socket, and the exit status of the
//! default handler in place of 128+n.
#![cfg(all(any(unix, windows), not(target_os = "android"), debug_assertions))]

// Rust guideline compliant 2026-08-29

use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::fs::DirBuilderExt;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
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

#[cfg(unix)]
#[test]
fn sigint_ends_the_app_with_the_signal_status() {
    signal_ends_the_app(libc::SIGINT, "int");
}

#[cfg(unix)]
#[test]
fn sigterm_ends_the_app_with_the_signal_status() {
    signal_ends_the_app(libc::SIGTERM, "term");
}

#[cfg(unix)]
#[test]
fn sighup_ends_the_app_with_the_signal_status() {
    signal_ends_the_app(libc::SIGHUP, "hup");
}

/// Asserts `signum` kills a child app and unlinks its socket.
///
/// `tag` keeps the socket directory of each caller distinct.
#[cfg(unix)]
fn signal_ends_the_app(signum: i32, tag: &str) {
    // A private XDG_RUNTIME_DIR of our own makes the child's socket path
    // predictable here, without reaching for the crate-private
    // `server::socket_address`. Under /tmp rather than `temp_dir()`: macOS
    // returns a path long enough to push the socket past the `sun_path` limit.
    let dir = PathBuf::from(format!("/tmp/tp230-{}-{tag}", std::process::id()));
    // Created non-recursively with its mode set up front: a name pre-planted in
    // world-writable /tmp then fails with EEXIST instead of being adopted, where
    // `create_dir_all` plus `set_permissions` would chmod a symlink's target
    // instead. 0700 is also what keeps the plugin from rejecting the directory
    // and falling back to /tmp.
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .expect("socket dir must be ours and private");
    let identifier = format!("tp230-{tag}");
    let socket = dir.join(format!("tauri-pilot-{identifier}.sock"));

    let mut child = spawn_child_app("XDG_RUNTIME_DIR", &dir, &identifier);
    let ready = wait_for_ready(&mut child, EXIT_TIMEOUT);
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

/// Ctrl+Break must end a child app and remove its instance file (#229).
///
/// Ctrl+Break is the only watched event that can be aimed at one process
/// group: Ctrl+C with a group id is delivered to nobody, and with none it
/// reaches this test binary and cargo too. It stands for Ctrl+C, which takes
/// the same branch of tokio's handler. Console close takes the other one, where
/// the handler parks so the watcher can run (the reason for the tokio 1.44
/// floor), and cannot be generated at all: it is only checked by hand.
#[cfg(windows)]
#[test]
fn ctrl_break_ends_the_app_and_removes_the_instance_file() {
    use windows::Win32::Foundation::STATUS_CONTROL_C_EXIT;
    use windows::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};

    // A private LOCALAPPDATA of our own keeps the child out of the real
    // instance registry and makes its instance file predictable here.
    let dir = std::env::temp_dir().join(format!("tp229-{}", std::process::id()));
    // An interrupted run whose pid this one drew may have left its instance
    // file here, which would pass for the child's registration below.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create private LOCALAPPDATA");
    // The pid again: the named pipe is machine-wide, unlike LOCALAPPDATA, so a
    // fixed name collides with a second `cargo test` or a stranded child.
    let identifier = format!("tp229-break-{}", std::process::id());
    let instance = dir
        .join("tauri-pilot")
        .join("instances")
        .join(format!("{identifier}.json"));

    let mut child = spawn_child_app("LOCALAPPDATA", &dir, &identifier);
    let ready = wait_for_ready(&mut child, EXIT_TIMEOUT);
    // The Windows bind runs on the server task (#115), after `build` returns,
    // so the ready line can come before the instance file does.
    let deadline = Instant::now() + EXIT_TIMEOUT;
    while !instance.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let registered = instance.exists();

    // SAFETY: `GenerateConsoleCtrlEvent` has no preconditions. The child's pid
    // is its group id, as it was spawned with `CREATE_NEW_PROCESS_GROUP`.
    let sent = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, child.id()) };
    let status = wait_for_exit(&mut child, EXIT_TIMEOUT);

    let left = instance.exists();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(ready, "child app never reported ready");
    assert!(registered, "child app must register its instance file");
    assert!(
        sent.is_ok(),
        "failed to send Ctrl+Break to the child app: {sent:?}"
    );
    let Some(status) = status else {
        panic!("Ctrl+Break must end the app, it still ran after {EXIT_TIMEOUT:?} (#229)");
    };
    assert!(!left, "Ctrl+Break must remove the instance file (#229)");
    assert_eq!(
        status.code(),
        Some(STATUS_CONTROL_C_EXIT.0),
        "app must exit with the status the default console handler gives (#229)"
    );
}

/// Re-execs this test binary as a mock app, with `dir_env` pointing at `dir`.
///
/// `dir_env` is the variable the plugin derives its socket or instance file
/// location from, so the child binds under a directory this test owns.
fn spawn_child_app(dir_env: &str, dir: &Path, identifier: &str) -> Child {
    let mut command = Command::new(std::env::current_exe().expect("test binary path"));
    command
        .args(["child_app", "--exact", "--ignored", "--nocapture"])
        .env(CHILD_ENV, identifier)
        .env(dir_env, dir)
        .stdout(Stdio::piped());
    // A group of its own: a console control event is sent to a whole group, and
    // the inherited one holds this test binary and cargo too.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut command,
        windows::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP.0,
    );
    command.spawn().expect("spawn child app")
}

/// Reads the child's output until it reports ready, or `timeout` elapses.
///
/// Bounded like `wait_for_exit`, and for the same reason: `lines()` has no
/// deadline, so a child wedged before its ready line would hang the whole
/// `cargo test` run, Rust having no per-test timeout. Killing the child on
/// timeout also ends the reader thread, whose only block is that stdout.
fn wait_for_ready(child: &mut Child, timeout: Duration) -> bool {
    let stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let ready = BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .any(|line| line.contains(READY));
        let _ = tx.send(ready);
    });
    if let Ok(ready) = rx.recv_timeout(timeout) {
        return ready;
    }
    let _ = child.kill();
    let _ = child.wait();
    false
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

    // The watcher runs on tauri's runtime, so this thread only has to outlive
    // one signal round trip. Bounded rather than parked forever: a parent killed
    // from outside the process group (IDE stop button, CI step timeout) would
    // strand this child with its socket bound, and a later run drawing the same
    // pid would then fail on a socket it cannot bind rather than on the signal
    // it means to test.
    std::thread::sleep(EXIT_TIMEOUT * 3);
}
