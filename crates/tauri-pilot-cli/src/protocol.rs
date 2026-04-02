use serde::{Deserialize, Deserializer, Serialize};

/// Deserialize params, normalizing `null` to `None`.
fn deserialize_params<'de, D>(deserializer: D) -> Result<Option<serde_json::Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let val: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match val {
        Some(serde_json::Value::Null) => Ok(None),
        other => Ok(other),
    }
}

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default, deserialize_with = "deserialize_params")]
    pub params: Option<serde_json::Value>,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[cfg(test)]
impl Response {
    /// Create a success response.
    #[must_use]
    pub fn success(id: u64, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create an error response.
    #[must_use]
    pub fn error(id: u64, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_deserialize_request_with_params() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":null}"#;
        let req: Request = serde_json::from_str(raw).expect("deserialize");
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, 1);
        assert_eq!(req.method, "ping");
        assert!(req.params.is_none(), "null params should normalize to None");
    }

    #[test]
    fn test_deserialize_request_without_params() {
        let raw = r#"{"jsonrpc":"2.0","id":2,"method":"snapshot"}"#;
        let req: Request = serde_json::from_str(raw).expect("deserialize");
        assert_eq!(req.method, "snapshot");
        assert!(req.params.is_none());
    }

    #[test]
    fn test_serialize_success_response() {
        let resp = Response::success(1, json!({"status": "ok"}));
        let v: serde_json::Value = serde_json::to_value(&resp).expect("serialize");
        assert_eq!(v["result"]["status"], "ok");
        assert!(v.get("error").is_none());
    }

    #[test]
    fn test_serialize_error_response() {
        let resp = Response::error(1, -32601, "Method not found");
        let s = serde_json::to_string(&resp).expect("serialize");
        assert!(s.contains(r#""error""#));
        assert!(!s.contains(r#""result""#));
        assert!(s.contains("-32601"));
    }

    #[test]
    fn test_roundtrip_request() {
        let req = Request {
            jsonrpc: "2.0".to_owned(),
            id: 42,
            method: "click".to_owned(),
            params: Some(json!({"ref": "e1"})),
        };
        let serialized = serde_json::to_string(&req).expect("serialize");
        let deserialized: Request = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.id, 42);
        assert_eq!(deserialized.method, "click");
    }

    #[test]
    fn test_roundtrip_response() {
        let resp = Response::success(7, json!([1, 2, 3]));
        let serialized = serde_json::to_string(&resp).expect("serialize");
        let deserialized: Response = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.id, 7);
        assert_eq!(deserialized.result, Some(json!([1, 2, 3])));
        assert!(deserialized.error.is_none());
    }
}
