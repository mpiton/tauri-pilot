mod error;
#[allow(dead_code)]
pub(crate) mod eval;
mod handler;
pub(crate) mod protocol;
pub(crate) mod server;

pub use error::Error;

use eval::EvalEngine;
use server::EvalFn;
use std::sync::Arc;
use tauri::Manager;

const BRIDGE_JS: &str = include_str!("../js/bridge.js");

/// Initialize the tauri-pilot plugin.
///
/// Injects the JS bridge, stores an `EvalEngine` in app state,
/// and starts a Unix socket server at `/tmp/tauri-pilot-{identifier}.sock`.
#[must_use]
pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("pilot")
        .js_init_script(BRIDGE_JS.to_owned())
        .setup(|app, _api| {
            let engine = EvalEngine::new();
            app.manage(engine.clone());

            let identifier = app.config().identifier.clone();
            let socket_path =
                std::path::PathBuf::from(format!("/tmp/tauri-pilot-{identifier}.sock"));

            tracing::info!(path = %socket_path.display(), "starting tauri-pilot socket server");

            let eval_fn = make_eval_fn(app);

            tauri::async_runtime::spawn(server::start(socket_path, engine, Some(eval_fn)));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![handler::__callback])
        .build()
}

/// Create an eval function from the app handle that evaluates JS in a webview.
///
/// Tries the "main" window label first for deterministic targeting,
/// falls back to the first available window if "main" doesn't exist.
fn make_eval_fn<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> EvalFn {
    let handle = app.clone();
    Arc::new(move |script| {
        if let Some(w) = handle.get_webview_window("main") {
            return w.eval(&script).map_err(|e| e.to_string());
        }
        let windows = handle.webview_windows();
        let window = windows
            .values()
            .next()
            .ok_or_else(|| "No webview window available".to_owned())?;
        window.eval(&script).map_err(|e| e.to_string())
    })
}
