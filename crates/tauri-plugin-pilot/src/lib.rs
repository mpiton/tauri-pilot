mod error;
pub(crate) mod protocol;
mod server;

pub use error::Error;

/// Initialize the tauri-pilot plugin.
///
/// Starts a Unix socket server at `/tmp/tauri-pilot-{identifier}.sock`
/// during the plugin setup phase. The server listens for JSON-RPC 2.0
/// requests from the CLI.
#[must_use]
pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("pilot")
        .setup(|app, _api| {
            let identifier = app.config().identifier.clone();
            let socket_path =
                std::path::PathBuf::from(format!("/tmp/tauri-pilot-{identifier}.sock"));

            // Remove stale socket from a previous run
            let _ = std::fs::remove_file(&socket_path);

            tracing::info!(path = %socket_path.display(), "starting tauri-pilot socket server");

            tauri::async_runtime::spawn(server::start(socket_path));

            Ok(())
        })
        .build()
}
