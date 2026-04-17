use crate::diff;
use crate::eval::EvalEngine;
#[cfg(feature = "press")]
use crate::key;
use crate::protocol::RpcError;
use crate::recorder::{RecordEntry, Recorder};
use crate::server::{EvalFn, FocusFn, ListWindowsFn};

use std::time::Duration;
#[cfg(feature = "press")]
use tokio::sync::Mutex as AsyncMutex;

/// Delay after requesting window focus before injecting OS-level keyboard
/// events, so the window manager has time to actually transfer focus. Tuned
/// empirically — too short and the first key on Wayland drops; too long and
/// press feels sluggish.
#[cfg(feature = "press")]
const FOCUS_SETTLE_MS: u64 = 80;

/// Serializes the full `focus → settle → inject` sequence across concurrent
/// `press` calls. The inner `key::PRESS_LOCK` only covers the OS injection,
/// so without this outer lock two calls targeting different windows could
/// race on the focus step and deliver both keys to whichever window won the
/// focus race.
#[cfg(feature = "press")]
static PRESS_ORDER_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(30);
/// Extra headroom added to the JS-side watch timeout so the Rust oneshot channel
/// doesn't expire before the JS `MutationObserver` can resolve/reject.
const WATCH_BUFFER_MS: u64 = 2_000;

/// Extract and remove the optional `"window"` key from params.
///
/// Returns `(window_label, cleaned_params)`:
/// - When `"window"` is present: `cleaned_params` is `Some(...)` with the key stripped.
/// - When `"window"` is absent: `cleaned_params` is `None` — the caller must fall back
///   to the original `params` reference (e.g. via `.as_ref().or(params)`).
fn extract_window(
    params: Option<&serde_json::Value>,
) -> (Option<String>, Option<serde_json::Value>) {
    let window = params
        .and_then(|o| o.get("window"))
        .and_then(|v| v.as_str())
        .map(String::from);
    match (window, params) {
        (Some(w), Some(p)) => {
            let mut cleaned = p.clone();
            if let Some(obj) = cleaned.as_object_mut() {
                obj.remove("window");
            }
            (Some(w), Some(cleaned))
        }
        (w, _) => (w, None),
    }
}

/// Dispatch a JSON-RPC method call to the appropriate handler.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub(crate) async fn dispatch(
    method: &str,
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
    list_fn: Option<&ListWindowsFn>,
    #[cfg_attr(not(feature = "press"), allow(unused_variables))] focus_fn: Option<&FocusFn>,
    recorder: &Recorder,
) -> Result<serde_json::Value, RpcError> {
    // Save original params before window extraction so the recorder can strip
    // "window" internally.
    let original_params = params.cloned();

    let (window, owned_params) = extract_window(params);
    let params = owned_params.as_ref().or(params);
    let win = window.as_deref();

    let result = match method {
        "ping" => Ok(serde_json::json!({"status": "ok"})),
        "windows.list" => {
            if let Some(f) = list_fn {
                Ok(f())
            } else {
                Err(RpcError {
                    code: -32603,
                    message: "No window manager available".to_owned(),
                    data: None,
                })
            }
        }
        "snapshot" => {
            let result =
                handle_eval_method("snapshot", params, engine, eval_fn, win, DEFAULT_TIMEOUT)
                    .await?;
            engine.store_snapshot(&result);
            Ok(result)
        }
        "diff" => handle_diff(params, engine, eval_fn, win).await,
        #[cfg(feature = "press")]
        "press" => handle_press(params, focus_fn, win).await,
        #[cfg(not(feature = "press"))]
        "press" => Err(RpcError {
            code: -32601,
            message: "press disabled (compile `tauri-plugin-pilot` with the `press` feature)"
                .to_owned(),
            data: None,
        }),
        "click" | "fill" | "type" | "select" | "check" | "scroll" | "drag" | "drop" | "text"
        | "html" | "value" | "attrs" | "eval" | "ipc" | "navigate" | "url" | "title" | "state"
        | "wait" | "visible" | "count" | "checked" => {
            handle_eval_method(method, params, engine, eval_fn, win, DEFAULT_TIMEOUT).await
        }
        "watch" => {
            let timeout_ms = params
                .and_then(|p| p.get("timeout"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(10_000);
            let timeout = Duration::from_millis(timeout_ms.saturating_add(WATCH_BUFFER_MS));
            handle_eval_method(method, params, engine, eval_fn, win, timeout).await
        }
        "screenshot" => {
            handle_eval_method(method, params, engine, eval_fn, win, SCREENSHOT_TIMEOUT).await
        }
        "console.getLogs" => {
            handle_eval_method("consoleLogs", params, engine, eval_fn, win, DEFAULT_TIMEOUT).await
        }
        "console.clear" => {
            handle_eval_method("clearLogs", params, engine, eval_fn, win, DEFAULT_TIMEOUT).await
        }
        "network.getRequests" => {
            handle_eval_method(
                "networkRequests",
                params,
                engine,
                eval_fn,
                win,
                DEFAULT_TIMEOUT,
            )
            .await
        }
        "network.clear" => {
            handle_eval_method(
                "clearNetwork",
                params,
                engine,
                eval_fn,
                win,
                DEFAULT_TIMEOUT,
            )
            .await
        }
        "storage.get" => {
            handle_eval_method("storageGet", params, engine, eval_fn, win, DEFAULT_TIMEOUT).await
        }
        "storage.set" => {
            handle_eval_method("storageSet", params, engine, eval_fn, win, DEFAULT_TIMEOUT).await
        }
        "storage.list" => {
            handle_eval_method("storageList", params, engine, eval_fn, win, DEFAULT_TIMEOUT).await
        }
        "storage.clear" => {
            handle_eval_method(
                "storageClear",
                params,
                engine,
                eval_fn,
                win,
                DEFAULT_TIMEOUT,
            )
            .await
        }
        "forms.dump" => {
            handle_eval_method("formDump", params, engine, eval_fn, win, DEFAULT_TIMEOUT).await
        }
        "record.start" => {
            recorder.start();
            Ok(serde_json::json!({"status": "recording"}))
        }
        "record.stop" => {
            let entries = recorder.stop();
            let count = entries.len();
            Ok(serde_json::json!({"entries": entries, "count": count}))
        }
        "record.status" => Ok(recorder.status()),
        "record.add" => {
            let entry: RecordEntry =
                serde_json::from_value(params.cloned().unwrap_or(serde_json::Value::Null))
                    .map_err(|e| RpcError {
                        code: -32602,
                        message: e.to_string(),
                        data: None,
                    })?;
            recorder.add_entry(entry);
            Ok(serde_json::json!({"status": "ok"}))
        }
        _ => Err(RpcError {
            code: -32601,
            message: format!("Method not found: {method}"),
            data: None,
        }),
    };

    // Auto-record on successful dispatches
    if result.is_ok() && recorder.is_active() {
        recorder.record(method, original_params.as_ref());
    }

    result
}

/// Handle the "diff" method: take a new snapshot, compare with the reference, and return `DiffResult`.
async fn handle_diff(
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
    window: Option<&str>,
) -> Result<serde_json::Value, RpcError> {
    let eval_fn = eval_fn.ok_or_else(|| RpcError {
        code: -32603,
        message: "No webview available for eval".to_owned(),
        data: None,
    })?;

    // Determine reference snapshot: from params["reference"] or last stored snapshot
    let reference = if let Some(ref_val) = params.and_then(|p| p.get("reference")) {
        ref_val.clone()
    } else {
        engine.get_last_snapshot().ok_or_else(|| RpcError {
            code: -32602,
            message:
                "No previous snapshot available. Run `snapshot` first or use `diff --ref <file>`"
                    .to_owned(),
            data: None,
        })?
    };

    // Take a new snapshot using the bridge — strip "reference" to avoid embedding
    // the entire old snapshot in the JS eval string (the bridge doesn't use it).
    let snapshot_params = params.map(|p| {
        let mut cleaned = p.clone();
        if let Some(obj) = cleaned.as_object_mut() {
            obj.remove("reference");
        }
        cleaned
    });
    let script =
        build_bridge_call("snapshot", snapshot_params.as_ref()).map_err(|msg| RpcError {
            code: -32602,
            message: msg,
            data: None,
        })?;
    let (id, rx) = engine.register();
    let wrapped = EvalEngine::wrap_script(id, &script);

    if let Err(e) = eval_fn(window, wrapped) {
        engine.resolve(id, Err(format!("Eval failed: {e}")));
        return Err(RpcError {
            code: -32603,
            message: format!("Eval failed: {e}"),
            data: None,
        });
    }

    let result = engine
        .wait(id, rx, DEFAULT_TIMEOUT)
        .await
        .map_err(|e| RpcError {
            code: -32603,
            message: format!("Eval error: {e}"),
            data: None,
        })?;

    // Parse both snapshots: extract "elements" arrays
    let old_elements: Vec<diff::SnapshotElement> = reference
        .get("elements")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|e| RpcError {
            code: -32602,
            message: format!("Failed to parse reference snapshot elements: {e}"),
            data: None,
        })?
        .unwrap_or_default();

    let new_elements: Vec<diff::SnapshotElement> = result
        .get("elements")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|e| RpcError {
            code: -32603,
            message: format!("Failed to parse new snapshot elements: {e}"),
            data: None,
        })?
        .unwrap_or_default();

    let diff_result = diff::compute_diff(&old_elements, &new_elements);

    // Store the new snapshot for subsequent diffs
    engine.store_snapshot(&result);

    serde_json::to_value(&diff_result).map_err(|e| RpcError {
        code: -32603,
        message: format!("Serialization error: {e}"),
        data: None,
    })
}

/// Handle the "press" method by injecting an OS-level keyboard event.
///
/// JS-dispatched `KeyboardEvent`s are flagged `isTrusted: false` and never
/// reach Tauri accelerators or window-manager-level shortcut handlers (#45).
/// Native injection via `enigo` produces real keyboard events that traverse
/// the full pipeline.
#[cfg(feature = "press")]
async fn handle_press(
    params: Option<&serde_json::Value>,
    focus_fn: Option<&FocusFn>,
    window: Option<&str>,
) -> Result<serde_json::Value, RpcError> {
    let key_str = params
        .and_then(|p| p.get("key"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RpcError {
            code: -32602,
            message: "press requires a non-empty \"key\" string param".to_owned(),
            data: None,
        })?;

    // Parse the combo up front: a bad combo is a client input error, so we
    // shouldn't take the serialization lock, steal focus, or sleep for it —
    // and we report it as -32602 (invalid params) instead of letting the
    // later spawn_blocking path surface it as -32603 (internal error).
    key::parse_combo(key_str).map_err(|e| RpcError {
        code: -32602,
        message: format!("invalid press combo: {e}"),
        data: None,
    })?;

    // An explicit `--window <label>` with no focus hook installed would
    // otherwise silently drop the focus step and inject into whatever window
    // currently has focus. Reject before taking any lock.
    if window.is_some() && focus_fn.is_none() {
        return Err(RpcError {
            code: -32603,
            message: "cannot focus target window: no focus hook installed".to_owned(),
            data: None,
        });
    }

    // Hold this lock across the whole focus → settle → inject sequence so
    // two concurrent `press` calls cannot interleave their focus steps (call
    // A focuses window X, call B focuses window Y, then both keys land on Y).
    let _order_guard = PRESS_ORDER_LOCK.lock().await;

    if let Some(focus) = focus_fn {
        match focus(window) {
            Ok(()) => {
                // Only wait if the WM actually accepted the focus request —
                // a failed focus call won't transfer focus, so sleeping
                // would just delay the press for nothing.
                tokio::time::sleep(Duration::from_millis(FOCUS_SETTLE_MS)).await;
            }
            Err(e) => {
                if let Some(label) = window {
                    // The caller explicitly targeted a window; silently
                    // falling through would deliver the key to whatever
                    // window currently has focus and still return ok.
                    return Err(RpcError {
                        code: -32603,
                        message: format!("failed to focus window '{label}': {e}"),
                        data: None,
                    });
                }
                tracing::warn!(error = %e, "focus before press failed (continuing)");
            }
        }
    }

    let combo = key_str.to_owned();
    tokio::task::spawn_blocking(move || key::simulate_press(&combo))
        .await
        .map_err(|e| {
            // A JoinError can be a panic, a cancellation, or a runtime
            // shutdown — reporting every one as "panicked" misleads during
            // teardown.
            let message = if e.is_panic() {
                format!("press task panicked: {e}")
            } else if e.is_cancelled() {
                "press task was cancelled".to_owned()
            } else {
                format!("press task failed: {e}")
            };
            RpcError {
                code: -32603,
                message,
                data: None,
            }
        })?
        .map_err(|e| RpcError {
            code: -32603,
            message: format!("press failed: {e}"),
            data: None,
        })?;

    Ok(serde_json::json!({"ok": true}))
}

/// Handle a method that requires JS evaluation via the bridge.
async fn handle_eval_method(
    method: &str,
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
    window: Option<&str>,
    timeout: Duration,
) -> Result<serde_json::Value, RpcError> {
    let eval_fn = eval_fn.ok_or_else(|| RpcError {
        code: -32603,
        message: "No webview available for eval".to_owned(),
        data: None,
    })?;

    let script = build_bridge_call(method, params).map_err(|msg| RpcError {
        code: -32602,
        message: msg,
        data: None,
    })?;
    let (id, rx) = engine.register();
    let wrapped = EvalEngine::wrap_script(id, &script);

    if let Err(e) = eval_fn(window, wrapped) {
        // Clean up pending entry on eval_fn failure
        engine.resolve(id, Err(format!("Eval failed: {e}")));
        return Err(RpcError {
            code: -32603,
            message: format!("Eval failed: {e}"),
            data: None,
        });
    }

    engine.wait(id, rx, timeout).await.map_err(|e| RpcError {
        code: -32603,
        message: format!("Eval error: {e}"),
        data: None,
    })
}

/// Build a `window.__PILOT__.<method>(params)` JS call string.
/// Returns `Err` with a message for invalid params (e.g. missing ipc command).
fn build_bridge_call(method: &str, params: Option<&serde_json::Value>) -> Result<String, String> {
    let args = match params {
        Some(v) if !v.is_null() => v.to_string(),
        _ => "{}".to_owned(),
    };

    if method == "ipc" {
        // ipc calls Tauri's backend invoke directly
        // serde_json::to_string produces a valid JS string literal (escaped quotes, backslashes, etc.)
        let command = params
            .and_then(|p| p.get("command"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "ipc requires a non-empty \"command\" string param".to_owned())?;
        let command_js = serde_json::to_string(command).unwrap_or_else(|_| "\"\"".to_owned());
        let ipc_args = params
            .and_then(|p| p.get("args"))
            .map_or("{}".to_owned(), ToString::to_string);
        return Ok(format!(
            "window.__TAURI_INTERNALS__.invoke({command_js}, {ipc_args})"
        ));
    }

    Ok(format!("window.__PILOT__.{method}({args})"))
}

/// Process the IPC callback from the JS bridge (ADR-001).
pub(crate) fn handle_callback(
    engine: &EvalEngine,
    id: u64,
    result: Option<String>,
    error: Option<String>,
) {
    if let Some(err) = error {
        engine.resolve(id, Err(err));
    } else if let Some(res) = result {
        match serde_json::from_str(&res) {
            Ok(val) => engine.resolve(id, Ok(val)),
            Err(_) => engine.resolve(id, Ok(serde_json::Value::String(res))),
        }
    } else {
        tracing::warn!(id, "callback received with neither result nor error");
        engine.resolve(id, Ok(serde_json::Value::Null));
    }
}

/// Tauri IPC command for the `__callback` handler.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn __callback(
    eval_engine: tauri::State<'_, EvalEngine>,
    id: u64,
    result: Option<String>,
    error: Option<String>,
) {
    handle_callback(&eval_engine, id, result, error);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_dispatch_ping_returns_ok() {
        let engine = EvalEngine::new();
        let result = dispatch("ping", None, &engine, None, None, None, &Recorder::new()).await;
        assert_eq!(result.unwrap(), json!({"status": "ok"}));
    }

    #[cfg(feature = "press")]
    #[tokio::test]
    async fn test_dispatch_press_with_invalid_combo_returns_invalid_params() {
        // A malformed combo must not steal focus or acquire the serialization
        // lock — it should short-circuit with -32602 (invalid params).
        let engine = EvalEngine::new();
        let result = dispatch(
            "press",
            Some(&json!({"key": "Control++P"})),
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("invalid press combo"));
    }

    #[cfg(feature = "press")]
    #[tokio::test]
    async fn test_dispatch_press_with_explicit_window_and_no_focus_fn_errors() {
        // --window <label> with no focus hook must not silently inject into
        // the currently focused window. We can pass `window` through params
        // (handler extracts it before dispatch).
        let engine = EvalEngine::new();
        let result = dispatch(
            "press",
            Some(&json!({"key": "Enter", "window": "settings"})),
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("focus"));
    }

    #[cfg(feature = "press")]
    #[tokio::test]
    async fn test_dispatch_press_with_missing_key_returns_invalid_params() {
        let engine = EvalEngine::new();
        let result =
            dispatch("press", None, &engine, None, None, None, &Recorder::new()).await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32602);
    }

    #[tokio::test]
    async fn test_dispatch_unknown_method_returns_error() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "nonexistent",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32601);
    }

    #[tokio::test]
    async fn test_dispatch_snapshot_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "snapshot",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_diff_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch("diff", None, &engine, None, None, None, &Recorder::new()).await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_diff_without_previous_snapshot() {
        let engine = EvalEngine::new();
        // Provide a dummy eval_fn that always succeeds (won't be called because we fail before)
        // Actually diff needs eval_fn first, then checks reference.
        // We need an eval_fn that returns something, but there's no previous snapshot.
        // Use an eval_fn that will block forever — but we check reference before eval.
        // Wait: handle_diff checks eval_fn first, then reference.
        // So with eval_fn but no previous snapshot, we get -32602 after eval completes.
        // Let's use a sync eval_fn and resolve manually.
        // Actually the reference check happens BEFORE the eval call, so we can check:
        // eval_fn present + no reference in params + no last_snapshot → -32602
        let eval_fn: crate::server::EvalFn =
            std::sync::Arc::new(|_w: Option<&str>, _script: String| Ok(()));
        let result = dispatch(
            "diff",
            None,
            &engine,
            Some(&eval_fn),
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("No previous snapshot"));
    }

    #[test]
    fn test_build_bridge_call_snapshot() {
        let params = json!({"interactive": true, "selector": null, "depth": 3});
        let script = build_bridge_call("snapshot", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.snapshot("));
        assert!(script.contains("\"interactive\":true"));
    }

    #[test]
    fn test_build_bridge_call_no_params() {
        let script = build_bridge_call("snapshot", None).unwrap();
        assert_eq!(script, "window.__PILOT__.snapshot({})");
    }

    #[test]
    fn test_build_bridge_call_ipc_missing_command() {
        let result = build_bridge_call("ipc", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("command"));
    }

    #[tokio::test]
    async fn test_callback_with_json_result() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        handle_callback(&engine, id, Some(r#"{"title":"hello"}"#.to_owned()), None);
        let val = rx.await.unwrap().unwrap();
        assert_eq!(val, json!({"title": "hello"}));
    }

    #[tokio::test]
    async fn test_callback_with_error() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        handle_callback(&engine, id, None, Some("TypeError: x".to_owned()));
        let result = rx.await.unwrap();
        assert_eq!(result, Err("TypeError: x".to_owned()));
    }

    #[tokio::test]
    async fn test_dispatch_console_get_logs_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "console.getLogs",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_console_clear_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "console.clear",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[test]
    fn test_build_bridge_call_console_logs() {
        let params = json!({"level": "error", "last": 10});
        let script = build_bridge_call("consoleLogs", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.consoleLogs("));
        assert!(script.contains("\"level\":\"error\""));
    }

    #[test]
    fn test_build_bridge_call_clear_logs() {
        let script = build_bridge_call("clearLogs", None).unwrap();
        assert_eq!(script, "window.__PILOT__.clearLogs({})");
    }

    #[tokio::test]
    async fn test_dispatch_network_get_requests_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "network.getRequests",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_network_clear_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "network.clear",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[test]
    fn test_build_bridge_call_network_requests() {
        let params = json!({"filter": "/api", "failedOnly": true, "last": 10});
        let script = build_bridge_call("networkRequests", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.networkRequests("));
        assert!(script.contains("\"filter\":\"/api\""));
    }

    #[test]
    fn test_build_bridge_call_clear_network() {
        let script = build_bridge_call("clearNetwork", None).unwrap();
        assert_eq!(script, "window.__PILOT__.clearNetwork({})");
    }

    #[test]
    fn test_build_bridge_call_visible() {
        let params = json!({"ref": "el-1"});
        let script = build_bridge_call("visible", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.visible("));
        assert!(script.contains("\"ref\":\"el-1\""));
    }

    #[test]
    fn test_build_bridge_call_count() {
        let params = json!({"selector": ".item"});
        let script = build_bridge_call("count", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.count("));
        assert!(script.contains("\"selector\":\".item\""));
    }

    #[test]
    fn test_build_bridge_call_checked() {
        let params = json!({"ref": "el-2"});
        let script = build_bridge_call("checked", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.checked("));
        assert!(script.contains("\"ref\":\"el-2\""));
    }

    #[tokio::test]
    async fn test_dispatch_watch_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch("watch", None, &engine, None, None, None, &Recorder::new()).await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[test]
    fn test_build_bridge_call_watch() {
        let params = json!({"timeout": 5000, "selector": ".results", "stable": 500});
        let script = build_bridge_call("watch", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.watch("));
        assert!(script.contains("\"timeout\":5000"));
    }

    #[tokio::test]
    async fn test_dispatch_drag_routes_to_eval() {
        let engine = EvalEngine::new();
        let result = dispatch("drag", None, &engine, None, None, None, &Recorder::new()).await;
        let err = result.unwrap_err();
        assert_ne!(err.code, -32601);
    }

    #[tokio::test]
    async fn test_dispatch_drop_routes_to_eval() {
        let engine = EvalEngine::new();
        let result = dispatch("drop", None, &engine, None, None, None, &Recorder::new()).await;
        let err = result.unwrap_err();
        assert_ne!(err.code, -32601);
    }

    #[test]
    fn test_build_bridge_call_drag() {
        let params = json!({"source": {"ref": "e5"}, "target": {"ref": "e6"}});
        let script = build_bridge_call("drag", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.drag("));
    }

    #[test]
    fn test_build_bridge_call_drop() {
        let params = json!({"ref": "e3", "files": []});
        let script = build_bridge_call("drop", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.drop("));
    }

    #[tokio::test]
    async fn test_dispatch_storage_get_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "storage.get",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_storage_set_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "storage.set",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_storage_list_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "storage.list",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_storage_clear_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "storage.clear",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[test]
    fn test_build_bridge_call_storage_get() {
        let params = json!({"key": "auth_token", "session": false});
        let script = build_bridge_call("storageGet", Some(&params)).unwrap();
        assert_eq!(
            script,
            r#"window.__PILOT__.storageGet({"key":"auth_token","session":false})"#
        );
    }

    #[test]
    fn test_build_bridge_call_storage_set() {
        let params = json!({"key": "theme", "value": "dark", "session": false});
        let script = build_bridge_call("storageSet", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.storageSet("));
        assert!(script.contains("\"key\":\"theme\""));
        assert!(script.contains("\"value\":\"dark\""));
        assert!(script.contains("\"session\":false"));
    }

    #[test]
    fn test_build_bridge_call_storage_list() {
        let params = json!({"session": true});
        let script = build_bridge_call("storageList", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.storageList("));
        assert!(script.contains("\"session\":true"));
    }

    #[test]
    fn test_build_bridge_call_storage_clear() {
        let params = json!({"session": false});
        let script = build_bridge_call("storageClear", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.storageClear("));
        assert!(script.contains("\"session\":false"));
    }

    #[test]
    fn test_build_bridge_call_form_dump() {
        let script = build_bridge_call("formDump", None).unwrap();
        assert_eq!(script, "window.__PILOT__.formDump({})");
    }

    #[test]
    fn test_build_bridge_call_form_dump_with_selector() {
        let params = json!({"selector": "#login-form"});
        let script = build_bridge_call("formDump", Some(&params)).unwrap();
        assert!(script.starts_with("window.__PILOT__.formDump("));
        assert!(script.contains("\"selector\":\"#login-form\""));
    }

    #[tokio::test]
    async fn test_dispatch_forms_dump_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "forms.dump",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_windows_list_without_list_fn() {
        let engine = EvalEngine::new();
        let result = dispatch(
            "windows.list",
            None,
            &engine,
            None,
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No window manager"));
    }

    #[tokio::test]
    async fn test_dispatch_windows_list_with_list_fn() {
        let engine = EvalEngine::new();
        let list_fn: crate::server::ListWindowsFn = std::sync::Arc::new(
            || serde_json::json!({"windows": [{"label": "main", "url": "http://localhost", "title": "Test"}]}),
        );
        let result = dispatch(
            "windows.list",
            None,
            &engine,
            None,
            Some(&list_fn),
            None,
            &Recorder::new(),
        )
        .await;
        let val = result.unwrap();
        let windows = val.get("windows").unwrap().as_array().unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].get("label").unwrap(), "main");
    }

    #[tokio::test]
    async fn test_dispatch_window_param_extracted_from_params() {
        let engine = EvalEngine::new();
        // The "window" key must be stripped before being forwarded to the bridge.
        // We verify this by inspecting the script received by eval_fn.
        let captured: std::sync::Arc<std::sync::Mutex<String>> =
            std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let captured_clone = captured.clone();
        let engine_clone = engine.clone();
        let eval_fn: crate::server::EvalFn =
            std::sync::Arc::new(move |_w: Option<&str>, script: String| {
                *captured_clone.lock().unwrap() = script;
                // Resolve the callback immediately to avoid blocking for the default 10s timeout.
                // ID 1 is the first registered callback on a fresh EvalEngine.
                engine_clone.resolve(1, Ok(serde_json::json!({"ok": true})));
                Ok(())
            });
        let params = serde_json::json!({"ref": "el-1", "window": "settings"});
        let _ = dispatch(
            "click",
            Some(&params),
            &engine,
            Some(&eval_fn),
            None,
            None,
            &Recorder::new(),
        )
        .await;
        let script = captured.lock().unwrap().clone();
        // "window" param must not appear in the JS call args
        assert!(!script.contains("\"window\""));
        assert!(script.contains("\"ref\""));
    }

    #[tokio::test]
    async fn test_dispatch_record_start_returns_recording() {
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        let result = dispatch("record.start", None, &engine, None, None, None, &recorder)
            .await
            .unwrap();
        assert_eq!(result["status"], "recording");
        assert!(recorder.is_active());
    }

    #[tokio::test]
    async fn test_dispatch_record_stop_returns_entries() {
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        recorder.start();
        recorder.record("click", Some(&json!({"ref": "e1"})));
        let result = dispatch("record.stop", None, &engine, None, None, None, &recorder)
            .await
            .unwrap();
        assert_eq!(result["count"], 1);
        assert!(result["entries"].as_array().is_some());
        assert!(!recorder.is_active());
    }

    #[tokio::test]
    async fn test_dispatch_record_status() {
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        recorder.start();
        let result = dispatch("record.status", None, &engine, None, None, None, &recorder)
            .await
            .unwrap();
        assert_eq!(result["active"], true);
        assert_eq!(result["count"], 0);
    }

    #[tokio::test]
    async fn test_dispatch_record_add_entry() {
        let engine = EvalEngine::new();
        let recorder = Recorder::new();
        recorder.start();
        let params = json!({"action": "navigate", "timestamp": 100, "url": "/home"});
        let result = dispatch(
            "record.add",
            Some(&params),
            &engine,
            None,
            None,
            None,
            &recorder,
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "ok");
        let entries = recorder.stop();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "navigate");
    }
}
