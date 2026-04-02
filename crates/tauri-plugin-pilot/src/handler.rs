use crate::eval::EvalEngine;
use crate::protocol::RpcError;
use crate::server::EvalFn;

use std::time::Duration;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Dispatch a JSON-RPC method call to the appropriate handler.
pub(crate) async fn dispatch(
    method: &str,
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
) -> Result<serde_json::Value, RpcError> {
    match method {
        "ping" => Ok(serde_json::json!({"status": "ok"})),
        "snapshot" | "click" | "fill" | "type" | "press" | "select" | "check" | "scroll"
        | "text" | "html" | "value" | "attrs" | "eval" | "ipc" | "navigate" | "url" | "title"
        | "state" | "wait" | "screenshot" => {
            handle_eval_method(method, params, engine, eval_fn).await
        }
        "console.getLogs" => handle_eval_method("consoleLogs", params, engine, eval_fn).await,
        "console.clear" => handle_eval_method("clearLogs", params, engine, eval_fn).await,
        _ => Err(RpcError {
            code: -32601,
            message: format!("Method not found: {method}"),
            data: None,
        }),
    }
}

/// Handle a method that requires JS evaluation via the bridge.
async fn handle_eval_method(
    method: &str,
    params: Option<&serde_json::Value>,
    engine: &EvalEngine,
    eval_fn: Option<&EvalFn>,
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

    engine
        .wait(id, rx, DEFAULT_TIMEOUT)
        .await
        .map_err(|e| RpcError {
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
}
