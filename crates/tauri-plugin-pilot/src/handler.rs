use crate::eval::EvalEngine;
use crate::protocol::RpcError;

/// Dispatch a JSON-RPC method call to the appropriate handler.
pub(crate) fn dispatch(
    method: &str,
    _params: Option<&serde_json::Value>,
) -> Result<serde_json::Value, RpcError> {
    match method {
        "ping" => Ok(serde_json::json!({"status": "ok"})),
        _ => Err(RpcError {
            code: -32601,
            message: format!("Method not found: {method}"),
            data: None,
        }),
    }
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

    #[test]
    fn test_dispatch_ping_returns_ok() {
        let result = dispatch("ping", None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({"status": "ok"}));
    }

    #[test]
    fn test_dispatch_unknown_method_returns_error() {
        let result = dispatch("nonexistent", None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("nonexistent"));
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
    async fn test_callback_with_plain_string_result() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        handle_callback(&engine, id, Some("not json".to_owned()), None);
        let val = rx.await.unwrap().unwrap();
        assert_eq!(val, json!("not json"));
    }

    #[tokio::test]
    async fn test_callback_with_error() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        handle_callback(&engine, id, None, Some("TypeError: x".to_owned()));
        let result = rx.await.unwrap();
        assert_eq!(result, Err("TypeError: x".to_owned()));
    }

    #[test]
    fn test_callback_unknown_id_no_panic() {
        let engine = EvalEngine::new();
        handle_callback(&engine, 999, Some("{}".to_owned()), None);
    }
}
