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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_ping_returns_ok() {
        let result = dispatch("ping", None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), serde_json::json!({"status": "ok"}));
    }

    #[test]
    fn test_dispatch_unknown_method_returns_error() {
        let result = dispatch("nonexistent", None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("nonexistent"));
    }
}
