pub mod diff;
mod error;
#[cfg(any(unix, windows))]
pub(crate) mod eval;
#[cfg(any(unix, windows))]
mod handler;
#[cfg(feature = "press")]
pub(crate) mod key;
pub(crate) mod protocol;
pub(crate) mod recorder;
#[cfg(any(unix, windows))]
pub(crate) mod server;

pub use error::Error;

#[cfg(any(unix, windows))]
use eval::EvalEngine;
#[cfg(any(unix, windows))]
use recorder::Recorder;
#[cfg(any(unix, windows))]
use server::{EvalFn, FocusFn, ListWindowsFn};
#[cfg(any(unix, windows))]
use std::sync::Arc;
#[cfg(any(unix, windows))]
use tauri::Manager;

#[cfg(all(any(unix, windows), debug_assertions))]
const BRIDGE_JS: &str = concat!(
    include_str!("../js/vendor/html-to-image.iife.js"),
    "\n",
    include_str!("../js/bridge.js"),
);

/// Initialize the tauri-pilot plugin.
///
/// On non-Unix, non-Windows platforms or in release builds, returns a no-op plugin.
/// In debug builds on Unix, injects the JS bridge, stores an `EvalEngine`,
/// and starts a Unix socket server at `$XDG_RUNTIME_DIR/tauri-pilot-{identifier}.sock` (falls back to `/tmp` if unavailable).
/// In debug builds on Windows, starts a Named Pipe server at
/// `\\.\pipe\tauri-pilot-{identifier}` and registers the instance under `%LOCALAPPDATA%\tauri-pilot\instances\`.
#[must_use]
pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    #[cfg(not(all(any(unix, windows), debug_assertions)))]
    {
        return tauri::plugin::Builder::new("pilot").build();
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    {
        tauri::plugin::Builder::new("pilot")
            .js_init_script(BRIDGE_JS.to_owned())
            .setup(|app, _api| {
                let engine = EvalEngine::new();
                app.manage(engine.clone());

                let identifier = sanitize_identifier(&app.config().identifier);
                let socket_path = server::socket_path(&identifier);

                let eval_fn = make_eval_fn(app);
                let list_fn = make_list_fn(app);
                let focus_fn = make_focus_fn(app);

                let (listener, guard) = server::bind(&socket_path).map_err(|e| {
                    tracing::error!(path = %socket_path.display(), "failed to bind socket: {e}");
                    e
                })?;

                let recorder = Recorder::new();

                tauri::async_runtime::spawn(server::run(
                    listener,
                    guard,
                    engine,
                    Some(eval_fn),
                    Some(list_fn),
                    Some(focus_fn),
                    recorder,
                ));

                Ok(())
            })
            .invoke_handler(tauri::generate_handler![handler::__callback])
            .build()
    }
}

/// Strip path separators and unsafe characters from the app identifier
/// so it can be safely used in a socket filename.
#[cfg(all(any(unix, windows), debug_assertions))]
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
/// If `window` is `Some(label)`, targets that specific window (error if not found).
/// If `window` is `None`, tries "main" first then falls back to the first available window.
#[cfg(all(any(unix, windows), debug_assertions))]
fn make_eval_fn<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> EvalFn {
    let handle = app.clone();
    Arc::new(move |window: Option<&str>, script: String| {
        if let Some(label) = window {
            return handle
                .get_webview_window(label)
                .ok_or_else(|| format!("Window '{label}' not found"))
                .and_then(|w| w.eval(&script).map_err(|e| e.to_string()));
        }
        if let Some(w) = handle.get_webview_window("main") {
            return w.eval(&script).map_err(|e| e.to_string());
        }
        let windows = handle.webview_windows();
        windows
            .values()
            .next()
            .ok_or_else(|| "No webview available".to_owned())
            .and_then(|w| w.eval(&script).map_err(|e| e.to_string()))
    })
}

/// Create a focus function that requests OS focus for a webview window.
///
/// Resolution mirrors `make_eval_fn`: explicit label first, then `"main"`, then
/// the first window. The call is best-effort — failures are returned to the
/// caller (which logs and continues), since the press still has a chance of
/// landing on whatever window currently holds focus.
#[cfg(all(any(unix, windows), debug_assertions))]
fn make_focus_fn<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> FocusFn {
    let handle = app.clone();
    Arc::new(move |window: Option<&str>| {
        let target = if let Some(label) = window {
            handle
                .get_webview_window(label)
                .ok_or_else(|| format!("Window '{label}' not found"))?
        } else if let Some(w) = handle.get_webview_window("main") {
            w
        } else {
            handle
                .webview_windows()
                .values()
                .next()
                .cloned()
                .ok_or_else(|| "No webview available".to_owned())?
        };
        target.set_focus().map_err(|e| e.to_string())
    })
}

/// Create a list function that enumerates all available webview windows.
#[cfg(all(any(unix, windows), debug_assertions))]
fn make_list_fn<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> ListWindowsFn {
    let handle = app.clone();
    Arc::new(move || {
        let windows = handle.webview_windows();
        // BTreeMap iterates in sorted key order — no explicit sort needed
        let list: Vec<serde_json::Value> = windows
            .iter()
            .map(|(label, wv)| {
                serde_json::json!({
                    "label": label,
                    "url": wv.url().map(|u| u.to_string()).unwrap_or_default(),
                    "title": wv.title().unwrap_or_default(),
                })
            })
            .collect();
        serde_json::json!({"windows": list})
    })
}

#[cfg(test)]
mod tests {
    #[cfg(all(any(unix, windows), debug_assertions))]
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
        let html_idx = js.find("htmlToImage").expect("htmlToImage missing");
        let pilot_idx = js
            .find("window.__PILOT__")
            .expect("window.__PILOT__ missing");
        assert!(
            html_idx < pilot_idx,
            "html-to-image must be injected before pilot bridge code"
        );
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_click_dispatches_pointer_sequence() {
        let js = super::BRIDGE_JS;
        let js_normalized: String = js.lines().collect::<Vec<_>>().join("\n");
        let pointer_down_idx = js
            .find(r#"dispatchPointerEvent(el, "pointerdown""#)
            .expect("click must dispatch pointerdown for Radix triggers");
        let mouse_down_idx = js
            .find(r#"MouseEvent("mousedown""#)
            .expect("click must keep mousedown compatibility");
        let pointer_up_idx = js
            .find(r#"dispatchPointerEvent(el, "pointerup""#)
            .expect("click must dispatch pointerup for Radix triggers");
        let mouse_up_idx = js
            .find(r#"MouseEvent("mouseup""#)
            .expect("click must keep mouseup compatibility");
        let click_idx = js
            .find(r#"dispatchPointerEvent(el, "click""#)
            .expect("click must dispatch as a pointer event");

        assert!(
            pointer_down_idx < mouse_down_idx
                && mouse_down_idx < pointer_up_idx
                && pointer_up_idx < mouse_up_idx
                && mouse_up_idx < click_idx,
            "click must dispatch pointerdown -> mousedown -> pointerup -> mouseup -> click"
        );
        assert!(
            js.contains(r#"pointerType: "mouse""#),
            "pointer events must include mouse pointer metadata"
        );
        assert!(
            js_normalized.contains(
                "if (pointerDownOk) {\n      const mouseDownOk = el.dispatchEvent(new MouseEvent(\"mousedown\""
            ),
            "mousedown must only dispatch when pointerdown was not canceled"
        );
        assert!(
            js_normalized.contains(
                "if (pointerDownOk) {\n      el.dispatchEvent(new MouseEvent(\"mouseup\""
            ),
            "mouseup must only dispatch when pointerdown was not canceled"
        );
    }

    #[cfg(all(any(unix, windows), debug_assertions))]
    #[test]
    fn bridge_scroll_handles_top_and_bottom_directions() {
        let js = super::BRIDGE_JS;
        assert!(
            js.contains(r#"if (dir === "top")"#),
            "scroll must handle direction \"top\""
        );
        assert!(
            js.contains(r#"if (dir === "bottom")"#),
            "scroll must handle direction \"bottom\""
        );
        assert!(
            js.contains("target.scrollTo(window.scrollX, 0)"),
            "scroll top on window must preserve window.scrollX and set Y=0"
        );
        assert!(
            js.contains("target.scrollTo(window.scrollX, Math.max(0, max))"),
            "scroll bottom on window must preserve window.scrollX and clamp negative max"
        );
        assert!(
            js.contains("Math.max(")
                && js.contains("docEl ? docEl.scrollHeight : 0")
                && js.contains("body ? body.scrollHeight : 0"),
            "scroll bottom on window must use Math.max(documentElement.scrollHeight, body.scrollHeight) for quirks-mode safety"
        );
        assert!(
            js.contains("docEl ? docEl.clientHeight : window.innerHeight"),
            "scroll bottom on window must subtract docEl.clientHeight (excludes horizontal scrollbar) instead of window.innerHeight"
        );
        assert!(
            js.contains("String(dir).slice(0, 64)"),
            "scroll error message must cap user-supplied direction length"
        );
        assert!(
            js.contains("target.scrollTop = 0"),
            "scroll top on element must set scrollTop = 0"
        );
        assert!(
            js.contains(
                "target.scrollTop = Math.max(0, target.scrollHeight - target.clientHeight)"
            ),
            "scroll bottom on element must use scrollHeight - clientHeight (not raw scrollHeight)"
        );
        assert!(
            js.contains("Unknown scroll direction:"),
            "scroll must throw on unknown direction instead of silently no-op"
        );
    }
}
