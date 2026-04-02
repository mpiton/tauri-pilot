mod error;
#[allow(dead_code)]
pub(crate) mod eval;
mod handler;
pub(crate) mod protocol;
#[cfg(unix)]
pub(crate) mod server;

pub use error::Error;

#[cfg(unix)]
use eval::EvalEngine;
#[cfg(unix)]
use server::EvalFn;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use tauri::Manager;

// Available on all Unix builds (include_str! is compile-time).
// The plugin itself is only active under debug_assertions; see init().
#[cfg(unix)]
const BRIDGE_JS: &str = concat!(
    include_str!("../js/vendor/html-to-image.iife.js"),
    "\n",
    include_str!("../js/bridge.js"),
);

/// Initialize the tauri-pilot plugin.
///
/// On non-Unix platforms or in release builds, returns a no-op plugin.
/// In debug builds on Unix, injects the JS bridge, stores an `EvalEngine`,
/// and starts a Unix socket server at `/tmp/tauri-pilot-{identifier}.sock`.
#[must_use]
pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    #[cfg(not(all(unix, debug_assertions)))]
    {
        return tauri::plugin::Builder::new("pilot").build();
    }

    #[cfg(all(unix, debug_assertions))]
    {
        tauri::plugin::Builder::new("pilot")
            .js_init_script(BRIDGE_JS.to_owned())
            .setup(|app, _api| {
                let engine = EvalEngine::new();
                app.manage(engine.clone());

                let identifier = sanitize_identifier(&app.config().identifier);
                let socket_path =
                    std::path::PathBuf::from(format!("/tmp/tauri-pilot-{identifier}.sock"));

                let eval_fn = make_eval_fn(app);

                let (listener, guard) = server::bind(&socket_path).map_err(|e| {
                    tracing::error!(path = %socket_path.display(), "failed to bind socket: {e}");
                    e
                })?;

                tauri::async_runtime::spawn(server::run(listener, guard, engine, Some(eval_fn)));

                Ok(())
            })
            .invoke_handler(tauri::generate_handler![handler::__callback])
            .build()
    }
}

/// Strip path separators and unsafe characters from the app identifier
/// so it can be safely used in a socket filename.
#[cfg(all(unix, debug_assertions))]
fn sanitize_identifier(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "default".to_owned()
    } else {
        sanitized
    }
}

/// Create an eval function from the app handle that evaluates JS in a webview.
///
/// Tries the "main" window label first for deterministic targeting,
/// falls back to the first available window if "main" doesn't exist.
#[cfg(all(unix, debug_assertions))]
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

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn bridge_js_contains_html_to_image_and_pilot() {
        let js = super::BRIDGE_JS;
        assert!(
            js.contains("htmlToImage"),
            "BRIDGE_JS must include the html-to-image IIFE bundle"
        );
        assert!(
            js.contains("window.__PILOT__"),
            "BRIDGE_JS must include the pilot bridge"
        );
    }
}
