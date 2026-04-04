use crate::diff;
use crate::eval::EvalEngine;
use crate::protocol::RpcError;
use crate::server::EvalFn;

use std::time::Duration;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(30);
/// Extra headroom added to the JS-side watch timeout so the Rust oneshot channel
/// doesn't expire before the JS `MutationObserver` can resolve/reject.
const WATCH_BUFFER_MS: u64 = 2_000;

/// Dispatch a JSON-RPC method call to the appropriate handler.
pub(crate) async fn dispatch(
    method: &str,
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
) -> Result<serde_json::Value, RpcError> {
    match method {
        "ping" => Ok(serde_json::json!({"status": "ok"})),
        "snapshot" => {
            let result =
                handle_eval_method("snapshot", params, engine, eval_fn, DEFAULT_TIMEOUT).await?;
            engine.store_snapshot(&result);
            Ok(result)
        }
        "diff" => handle_diff(params, engine, eval_fn).await,
        "click" | "fill" | "type" | "press" | "select" | "check" | "scroll" | "drag" | "drop"
        | "text" | "html" | "value" | "attrs" | "eval" | "ipc" | "navigate" | "url" | "title"
        | "state" | "wait" | "visible" | "count" | "checked" => {
            handle_eval_method(method, params, engine, eval_fn, DEFAULT_TIMEOUT).await
        }
        "watch" => {
            let timeout_ms = params
                .and_then(|p| p.get("timeout"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(10_000);
            let timeout = Duration::from_millis(timeout_ms.saturating_add(WATCH_BUFFER_MS));
            handle_eval_method(method, params, engine, eval_fn, timeout).await
        }
        "screenshot" => {
            handle_eval_method(method, params, engine, eval_fn, SCREENSHOT_TIMEOUT).await
        }
        "console.getLogs" => {
            handle_eval_method("consoleLogs", params, engine, eval_fn, DEFAULT_TIMEOUT).await
        }
        "console.clear" => {
            handle_eval_method("clearLogs", params, engine, eval_fn, DEFAULT_TIMEOUT).await
        }
        "network.getRequests" => {
            handle_eval_method("networkRequests", params, engine, eval_fn, DEFAULT_TIMEOUT).await
        }
        "network.clear" => {
            handle_eval_method("clearNetwork", params, engine, eval_fn, DEFAULT_TIMEOUT).await
        }
        _ => Err(RpcError {
            code: -32601,
            message: format!("Method not found: {method}"),
            data: None,
        }),
    }
}

/// Handle the "diff" method: take a new snapshot, compare with the reference, and return `DiffResult`.
async fn handle_diff(
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
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

    if let Err(e) = eval_fn(wrapped) {
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

/// Handle a method that requires JS evaluation via the bridge.
async fn handle_eval_method(
    method: &str,
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
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

    if let Err(e) = eval_fn(wrapped) {
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
        let result = dispatch("ping", None, &engine, None).await;
        assert_eq!(result.unwrap(), json!({"status": "ok"}));
    }

    #[tokio::test]
    async fn test_dispatch_unknown_method_returns_error() {
        let engine = EvalEngine::new();
        let result = dispatch("nonexistent", None, &engine, None).await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32601);
    }

    #[tokio::test]
    async fn test_dispatch_snapshot_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch("snapshot", None, &engine, None).await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_diff_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch("diff", None, &engine, None).await;
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
        let eval_fn: crate::server::EvalFn = std::sync::Arc::new(|_script| Ok(()));
        let result = dispatch("diff", None, &engine, Some(&eval_fn)).await;
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
        let result = dispatch("console.getLogs", None, &engine, None).await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_console_clear_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch("console.clear", None, &engine, None).await;
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
        let result = dispatch("network.getRequests", None, &engine, None).await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("No webview"));
    }

    #[tokio::test]
    async fn test_dispatch_network_clear_without_eval_fn() {
        let engine = EvalEngine::new();
        let result = dispatch("network.clear", None, &engine, None).await;
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
        let result = dispatch("watch", None, &engine, None).await;
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
        let result = dispatch("drag", None, &engine, None).await;
        let err = result.unwrap_err();
        assert_ne!(err.code, -32601);
    }

    #[tokio::test]
    async fn test_dispatch_drop_routes_to_eval() {
        let engine = EvalEngine::new();
        let result = dispatch("drop", None, &engine, None).await;
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
}
