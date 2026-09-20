//! Cleanup for the exits that never reach `RunEvent::Exit`.
//!
//! A Unix signal (#217) or a Windows console control event (#229) ends the
//! process before tao emits `RunEvent::Exit`, so the guard in plugin state
//! never drops. Each platform gets a watcher that releases it, then ends the
//! app the way the default handler would have.

// Rust guideline compliant 2026-08-29

use crate::PilotGuard;

/// Unlinks the socket when a signal kills the app before `RunEvent::Exit` (#217).
///
/// SIGINT, SIGTERM and SIGHUP end the process before tao emits `RunEvent::Exit`,
/// so the guard in plugin state never drops and the socket file stays behind.
/// Watching the signals suppresses their default kill, hence the re-raise.
#[cfg(unix)]
pub(crate) fn spawn_cleanup(guard: &PilotGuard) {
    use tokio::signal::unix::{SignalKind, signal};

    for (kind, signum) in [
        (SignalKind::interrupt(), libc::SIGINT),
        (SignalKind::terminate(), libc::SIGTERM),
        // Closing the terminal running `cargo tauri dev` sends SIGHUP, whose
        // default disposition kills just as silently as the other two.
        (SignalKind::hangup(), libc::SIGHUP),
    ] {
        // Registered here, not inside the task: between `setup` returning and
        // the task's first poll the default disposition is still in force and a
        // signal in that window leaks the socket. `signal` needs a runtime
        // context and `setup` runs outside one (#115), hence `block_on`.
        //
        // One stream and one task per signal rather than a single `select!`:
        // a kind that fails to register then costs only itself, instead of
        // dropping a stream tokio has already hooked and leaving that signal
        // handled by nobody, which would make the app ignore Ctrl+C outright.
        let opened = tauri::async_runtime::block_on(async { signal(kind) });
        let Ok(mut stream) = opened.inspect_err(|e| {
            tracing::warn!(
                signum,
                "tauri-pilot cannot watch this signal, it leaves the socket behind: {e}"
            );
        }) else {
            continue;
        };
        let guard = guard.clone();
        tauri::async_runtime::spawn(async move {
            stream.recv().await;
            guard.release();
            re_raise(signum);
        });
    }
}

/// Restores the default disposition and re-raises `signum`.
///
/// The host app then dies from the signal it was sent, with the usual 128+n
/// status, instead of ignoring it because the plugin installed a handler.
#[cfg(unix)]
fn re_raise(signum: i32) {
    // SAFETY: `signal` and `raise` are libc entry points with no preconditions.
    unsafe {
        // Restored first: tokio keeps its handler installed for the life of the
        // process, so the raise below would land back in the watcher instead
        // of killing the app.
        libc::signal(signum, libc::SIG_DFL);
        // No `cfg!(test)` guard here: a watcher firing inside the test binary
        // ends the whole run, so the signal path is covered out of process in
        // tests/signal_exit.rs (#230) rather than by skipping the raise.
        libc::raise(signum);
    }
}

/// Removes the instance file when a console event kills the app before `RunEvent::Exit` (#229).
///
/// Ctrl+C, Ctrl+Break and closing the console end the process before tao emits
/// `RunEvent::Exit`, so the registry guard in plugin state never drops and
/// `instances/{identifier}.json` keeps listing an app that is gone.
///
/// Shutdown and logoff are not watched: Windows never calls a console handler
/// for them in a process that has loaded user32.dll, which every Tauri app
/// has. The file stays, and the CLI falls back on its check of the entry's pid.
#[cfg(windows)]
pub(crate) fn spawn_cleanup(guard: &PilotGuard) {
    use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close};

    // Registered here, not inside the tasks: between `setup` returning and a
    // task's first poll the default handler is still in force and an event in
    // that window leaks the instance file. Unlike `signal` on Unix these need
    // no runtime context, so no `block_on`.
    //
    // One stream and one task per event, for the reason the Unix watcher gives:
    // a kind that fails to register then costs only itself. The three streams
    // are distinct types with no shared trait, hence the `map` to a future.
    release_on(
        guard,
        "ctrl_c",
        ctrl_c().map(|mut s| async move {
            s.recv().await;
        }),
    );
    release_on(
        guard,
        "ctrl_break",
        ctrl_break().map(|mut s| async move {
            s.recv().await;
        }),
    );
    release_on(
        guard,
        "ctrl_close",
        ctrl_close().map(|mut s| async move {
            s.recv().await;
        }),
    );
}

/// Releases `guard` once `fired` resolves, then ends the process.
///
/// Watching a console event makes tokio's handler claim it. Windows calls
/// handlers last registered first and stops at the first claim, so neither a
/// handler the host app registered before the plugin's `setup` nor the default
/// handler's `ExitProcess` runs. The exit here stands in for the default, with
/// the status it gives, so the app still dies of its Ctrl+C.
#[cfg(windows)]
fn release_on<F>(guard: &PilotGuard, event: &'static str, fired: std::io::Result<F>)
where
    F: Future<Output = ()> + Send + 'static,
{
    use windows::Win32::Foundation::STATUS_CONTROL_C_EXIT;

    let Ok(fired) = fired.inspect_err(|e| {
        tracing::warn!(
            event,
            "tauri-pilot cannot watch this console event, it leaves the instance file behind: {e}"
        );
    }) else {
        return;
    };
    let guard = guard.clone();
    tauri::async_runtime::spawn(async move {
        fired.await;
        // The release is the only work done here: on close Windows kills the
        // process a few seconds after the event.
        guard.release();
        std::process::exit(STATUS_CONTROL_C_EXIT.0);
    });
}
