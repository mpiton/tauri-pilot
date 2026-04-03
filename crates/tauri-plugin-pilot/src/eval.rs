use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;

/// Error types for eval operations.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EvalError {
    #[error("eval timed out after {0:?}")]
    Timeout(Duration),
    #[error("JavaScript error: {0}")]
    JsError(String),
    #[error("eval channel closed unexpectedly")]
    ChannelClosed,
}

type PendingMap = HashMap<u64, oneshot::Sender<Result<serde_json::Value, String>>>;

/// Engine for executing JS in a `WebView` and getting results via callback.
///
/// The core ADR-001 pattern: wrap script in try/catch + invoke callback,
/// await the result on a oneshot channel with timeout.
#[derive(Clone)]
pub(crate) struct EvalEngine {
    pending: Arc<Mutex<PendingMap>>,
    next_id: Arc<AtomicU64>,
    last_snapshot: Arc<Mutex<Option<serde_json::Value>>>,
}

impl EvalEngine {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            last_snapshot: Arc::new(Mutex::new(None)),
        }
    }

    /// Store the last snapshot result for later diff comparison.
    pub fn store_snapshot(&self, value: &serde_json::Value) {
        *self
            .last_snapshot
            .lock()
            .expect("last_snapshot lock poisoned") = Some(value.clone());
    }

    /// Retrieve the last stored snapshot, if any.
    pub fn get_last_snapshot(&self) -> Option<serde_json::Value> {
        self.last_snapshot
            .lock()
            .expect("last_snapshot lock poisoned")
            .clone()
    }

    /// Register a pending eval request. Returns the ID and a receiver.
    pub fn register(&self) -> (u64, oneshot::Receiver<Result<serde_json::Value, String>>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending lock poisoned")
            .insert(id, tx);
        (id, rx)
    }

    /// Resolve a pending eval by ID. Called from the IPC __callback handler.
    pub fn resolve(&self, id: u64, result: Result<serde_json::Value, String>) {
        let sender = self
            .pending
            .lock()
            .expect("pending lock poisoned")
            .remove(&id);

        match sender {
            Some(tx) => {
                let _ = tx.send(result);
            }
            None => {
                tracing::warn!(id, "resolve called for unknown eval ID");
            }
        }
    }

    /// Wrap a user script in the ADR-001 callback pattern.
    /// Uses `__TAURI_INTERNALS__.invoke` which is always available (even without
    /// `withGlobalTauri`). The expression is `await`-ed so async results resolve
    /// before serialization.
    #[must_use]
    pub fn wrap_script(id: u64, script: &str) -> String {
        format!(
            "(async()=>{{try{{let __r=await({script});\
             await window.__TAURI_INTERNALS__.invoke('plugin:pilot|__callback',\
             {{id:{id},result:JSON.stringify(__r)}});\
             }}catch(__e){{await window.__TAURI_INTERNALS__.invoke('plugin:pilot|__callback',\
             {{id:{id},error:(__e&&__e.message)||String(__e)}});}}}})();"
        )
    }

    /// Wait for a pending eval result with timeout.
    /// Cleans up the pending entry on timeout to prevent memory leaks.
    pub async fn wait(
        &self,
        id: u64,
        rx: oneshot::Receiver<Result<serde_json::Value, String>>,
        timeout: Duration,
    ) -> Result<serde_json::Value, EvalError> {
        let result = tokio::time::timeout(timeout, rx).await;

        match result {
            Ok(Ok(inner)) => inner.map_err(EvalError::JsError),
            Ok(Err(_)) => {
                // Defensive cleanup — sender dropped without sending
                self.pending
                    .lock()
                    .expect("pending lock poisoned")
                    .remove(&id);
                Err(EvalError::ChannelClosed)
            }
            Err(_) => {
                // Remove stale entry from pending map on timeout
                self.pending
                    .lock()
                    .expect("pending lock poisoned")
                    .remove(&id);
                Err(EvalError::Timeout(timeout))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_new_starts_at_id_1() {
        let engine = EvalEngine::new();
        let (id, _rx) = engine.register();
        assert_eq!(id, 1);
    }

    #[test]
    fn test_ids_increment() {
        let engine = EvalEngine::new();
        let (id1, _) = engine.register();
        let (id2, _) = engine.register();
        assert_eq!(id2, id1 + 1);
    }

    #[tokio::test]
    async fn test_resolve_success() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        engine.resolve(id, Ok(json!(42)));
        let result = rx.await.unwrap();
        assert_eq!(result, Ok(json!(42)));
    }

    #[tokio::test]
    async fn test_resolve_js_error() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        engine.resolve(id, Err("ReferenceError: x is not defined".to_owned()));
        let result = rx.await.unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_unknown_id_no_panic() {
        let engine = EvalEngine::new();
        engine.resolve(999, Ok(json!(null)));
    }

    #[tokio::test]
    async fn test_wait_timeout_cleans_pending() {
        tokio::time::pause();
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        let result = engine.wait(id, rx, Duration::from_secs(1)).await;
        assert!(matches!(result, Err(EvalError::Timeout(_))));
        // Verify pending entry was cleaned up
        assert!(!engine.pending.lock().expect("lock").contains_key(&id));
    }

    #[tokio::test]
    async fn test_wait_success() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        engine.resolve(id, Ok(json!({"title": "hello"})));
        let result = engine.wait(id, rx, Duration::from_secs(10)).await;
        assert_eq!(result.unwrap(), json!({"title": "hello"}));
    }

    #[tokio::test]
    async fn test_wait_js_error() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        engine.resolve(id, Err("boom".to_owned()));
        let result = engine.wait(id, rx, Duration::from_secs(10)).await;
        assert!(matches!(result, Err(EvalError::JsError(ref m)) if m == "boom"));
    }

    #[test]
    fn test_wrap_script_contains_id_and_code() {
        let script = EvalEngine::wrap_script(42, "document.title");
        assert!(script.contains("42"));
        assert!(script.contains("document.title"));
        assert!(script.contains("__callback"));
        assert!(script.contains("try"));
        assert!(script.contains("catch"));
    }

    #[test]
    fn test_get_last_snapshot_none_initially() {
        let engine = EvalEngine::new();
        assert!(engine.get_last_snapshot().is_none());
    }

    #[test]
    fn test_store_and_retrieve_snapshot() {
        let engine = EvalEngine::new();
        let value = json!({"elements": [{"ref": "e1", "role": "button", "depth": 1}]});
        engine.store_snapshot(&value);
        let retrieved = engine.get_last_snapshot();
        assert_eq!(retrieved, Some(value));
    }
}
