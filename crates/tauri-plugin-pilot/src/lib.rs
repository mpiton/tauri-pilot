mod error;
#[allow(dead_code)]
pub(crate) mod eval;
mod handler;
pub(crate) mod protocol;
mod server;

pub use error::Error;

use eval::EvalEngine;
use tauri::Manager;

/// Initialize the tauri-pilot plugin.
///
/// Stores an `EvalEngine` in app state and starts a Unix socket server
/// at `/tmp/tauri-pilot-{identifier}.sock`. Registers the `__callback`
/// IPC handler for the eval+callback pattern (ADR-001).
#[must_use]
pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("pilot")
        .setup(|app, _api| {
            app.manage(EvalEngine::new());

            let identifier = app.config().identifier.clone();
            let socket_path =
                std::path::PathBuf::from(format!("/tmp/tauri-pilot-{identifier}.sock"));

            // Remove stale socket from a previous run
            let _ = std::fs::remove_file(&socket_path);

            tracing::info!(path = %socket_path.display(), "starting tauri-pilot socket server");

            tauri::async_runtime::spawn(server::start(socket_path));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![handler::__callback])
        .build()
}
