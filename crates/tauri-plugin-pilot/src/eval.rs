use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::Url;
use tokio::sync::{oneshot, watch};

/// Callback id the bridge sends its hello with when a page loads (#153).
///
/// Eval request ids start at 1 (see [`EvalEngine::new`]), so 0 never collides
/// with a pending eval. `bridge.js` hardcodes the same value.
pub(crate) const HELLO_ID: u64 = 0;

/// Origins whose bridge said hello, each with the hello count at its latest hello.
#[derive(Debug, Default)]
struct Bridges {
    hellos: u64,
    latest: HashMap<String, u64>,
}

/// Key a URL by scheme, host and port.
///
/// `Url::origin` is opaque for custom schemes such as `tauri://`, which would
/// make every app page its own origin, so the key is built by hand.
/// Default ports are always written (`https://host` and `https://host:443`
/// share a key). Hostless URLs (`file:`, `data:`) keep the rest of the URL
/// so they do not inherit each other's hello.
pub(crate) fn origin_key(url: &Url) -> String {
    match (
        url.host_str().filter(|host| !host.is_empty()),
        url.port_or_known_default(),
    ) {
        (Some(host), Some(port)) => format!("{}://{host}:{port}", url.scheme()),
        (Some(host), None) => format!("{}://{host}", url.scheme()),
        (None, _) => match url.as_str().split_once('#') {
            Some((without_fragment, _)) => without_fragment.to_owned(),
            None => url.as_str().to_owned(),
        },
    }
}

/// Origin keys `url` can match, including an `http` → `https` upgrade.
///
/// HSTS (and similar) hops store the hello under `https://host` while
/// `navigate` still asked for `http://host`. Same host and port, new scheme.
fn origin_keys(url: &Url) -> Vec<String> {
    let key = origin_key(url);
    if url.scheme() != "http" {
        return vec![key];
    }
    let mut https = url.clone();
    if https.set_scheme("https").is_err() {
        return vec![key];
    }
    let https_key = origin_key(&https);
    if https_key == key {
        vec![key]
    } else {
        vec![key, https_key]
    }
}

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

/// Engine for executing JS in a `WebView` and resolving eval results delivered via the `__callback` IPC command.
///
/// The core ADR-001 pattern: wrap script in try/catch + return a callback payload,
/// await the result on a oneshot channel with timeout.
#[derive(Clone)]
pub(crate) struct EvalEngine {
    pending: Arc<Mutex<PendingMap>>,
    next_id: Arc<AtomicU64>,
    last_snapshot: Arc<Mutex<Option<serde_json::Value>>>,
    bridges: Arc<watch::Sender<Bridges>>,
}

impl EvalEngine {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            last_snapshot: Arc::new(Mutex::new(None)),
            bridges: Arc::new(watch::Sender::new(Bridges::default())),
        }
    }

    /// Record a hello from `page` (the invoking webview's URL).
    ///
    /// Only pages whose origin may call `__callback` can say hello, so the
    /// hellos tell which origins answer bridge commands (#153).
    pub fn bridge_hello(&self, page: &str) {
        let Ok(url) = Url::parse(page) else {
            tracing::warn!(page, "bridge hello with an invalid page URL");
            return;
        };
        let key = origin_key(&url);
        self.bridges.send_modify(|b| {
            b.hellos += 1;
            b.latest.insert(key, b.hellos);
        });
    }

    /// Number of bridge hellos received so far.
    pub fn hellos(&self) -> u64 {
        self.bridges.borrow().hellos
    }

    /// Whether a bridge on the origin of `url` can answer.
    ///
    /// Also `true` before any hello: without one the engine cannot tell app
    /// origins from foreign ones, so callers keep the plain eval path.
    /// An `http` URL also matches a hello from the `https` upgrade of the
    /// same host and port, so an HSTS hop still counts.
    pub fn has_bridge(&self, url: &Url) -> bool {
        let bridges = self.bridges.borrow();
        if bridges.hellos == 0 {
            return true;
        }
        origin_keys(url)
            .iter()
            .any(|key| bridges.latest.contains_key(key))
    }

    /// Origins whose bridge said hello, sorted.
    pub fn bridge_origins(&self) -> Vec<String> {
        let mut origins: Vec<String> = self.bridges.borrow().latest.keys().cloned().collect();
        origins.sort();
        origins
    }

    /// Wait for a hello from the origin of `url` newer than hello number `since`.
    ///
    /// Returns `false` when none arrives within `limit`. An `http` destination
    /// also succeeds when the hello comes from the `https` upgrade of the
    /// same host and port.
    pub async fn wait_bridge(&self, url: &Url, since: u64, limit: Duration) -> bool {
        let keys = origin_keys(url);
        let mut rx = self.bridges.subscribe();
        let hello = rx.wait_for(move |b| {
            keys.iter()
                .any(|key| b.latest.get(key).is_some_and(|&n| n > since))
        });
        tokio::time::timeout(limit, hello)
            .await
            .is_ok_and(|seen| seen.is_ok())
    }

    /// Store the last snapshot result for later diff comparison.
    ///
    /// Note: `store_snapshot` and `get_last_snapshot` each acquire the lock independently.
    /// Concurrent snapshot/diff calls may observe non-deterministic ordering — acceptable
    /// for this single-connection CLI debugging tool.
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

    /// Resolve a pending eval by ID from a `__callback` IPC payload.
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
    ///
    /// `WebView` eval result callbacks are not reliable across platforms and CI
    /// modes, so the script `await`s the result and delivers it back through the
    /// `__callback` IPC command, which dispatches fine from eval-injected code.
    ///
    /// Normalizes `undefined` results to `null` (string `"null"`) so Tauri does
    /// not drop the `result` field — otherwise a void expression (e.g.,
    /// `element.click()`) would cause the handler to log a bogus "neither
    /// result nor error" warning (#48).
    #[must_use]
    pub fn wrap_script(id: u64, script: &str) -> String {
        format!(
            "(async()=>{{try{{let __r=await({script});\
             await window.__TAURI_INTERNALS__.invoke('plugin:pilot|__callback',\
             {{id:{id},result:__r===undefined?'null':JSON.stringify(__r)}});\
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
        let result = rx.await.expect("resolve channel dropped");
        assert_eq!(result, Ok(json!(42)));
    }

    #[tokio::test]
    async fn test_resolve_js_error() {
        let engine = EvalEngine::new();
        let (id, rx) = engine.register();
        engine.resolve(id, Err("ReferenceError: x is not defined".to_owned()));
        let result = rx.await.expect("resolve channel dropped");
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
        assert_eq!(result.expect("wait succeeds"), json!({"title": "hello"}));
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
        assert!(script.contains("await("));
        assert!(script.contains("try"));
        assert!(script.contains("catch"));
    }

    // #110/#126: The wrapped script must await the JavaScript result and send it
    // through `__callback`, avoiding native eval result callbacks entirely.
    #[test]
    fn test_wrap_script_uses_ipc_callback_delivery() {
        let script = EvalEngine::wrap_script(7, "document.title");
        assert!(
            script.contains("__TAURI_INTERNALS__.invoke('plugin:pilot|__callback'"),
            "wrapped script must send eval results through __callback IPC; got: {script}"
        );
        assert!(script.contains("{id:7,result:"));
        assert!(script.contains("{id:7,error:"));
        assert!(
            !script.contains("return {id:7,result:"),
            "wrapped script must not return a native eval completion payload; got: {script}"
        );
    }

    #[test]
    fn test_wrap_script_normalizes_undefined_to_null() {
        // #48: A JS expression returning undefined must not cause the handler
        // to log "callback received with neither result nor error". The wrapper
        // converts undefined → the string "null" so Tauri keeps the `result`
        // field populated.
        let script = EvalEngine::wrap_script(1, "element.click()");
        assert!(
            script.contains("__r===undefined?'null':JSON.stringify(__r)"),
            "wrapped script must normalize undefined to the string 'null'; got: {script}"
        );
    }

    #[test]
    fn test_has_bridge_keys_by_scheme_host_and_port() {
        let engine = EvalEngine::new();
        let url = |text| Url::parse(text).expect("valid test URL");
        // Before any hello the engine cannot tell origins apart.
        assert!(engine.has_bridge(&url("https://example.com/")));

        engine.bridge_hello("tauri://localhost/index.html");
        // `Url::origin` is opaque for custom schemes; app pages must share a key.
        assert!(engine.has_bridge(&url("tauri://localhost/settings")));
        assert!(!engine.has_bridge(&url("https://example.com/")));

        engine.bridge_hello("http://127.0.0.1:8080/");
        assert!(!engine.has_bridge(&url("http://127.0.0.1:9090/")));
        assert_eq!(
            engine.bridge_origins(),
            ["http://127.0.0.1:8080", "tauri://localhost"]
        );
    }

    #[test]
    fn test_origin_key_normalizes_default_ports_and_opaque_urls() {
        let engine = EvalEngine::new();
        let url = |text| Url::parse(text).expect("valid test URL");
        engine.bridge_hello("https://example.com/");
        assert!(engine.has_bridge(&url("https://example.com:443/login")));
        assert!(
            engine.has_bridge(&url("http://example.com/")),
            "http dest must match an https hello (HSTS)"
        );
        assert!(!engine.has_bridge(&url("https://other.example/")));

        engine.bridge_hello("file:///tmp/a.html");
        assert!(engine.has_bridge(&url("file:///tmp/a.html")));
        assert!(
            !engine.has_bridge(&url("file:///tmp/b.html")),
            "hostless pages must not share a key"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_wait_bridge_ignores_hello_from_another_origin() {
        let engine = EvalEngine::new();
        let app = Url::parse("tauri://localhost/").expect("valid test URL");
        let foreign = Url::parse("https://example.com/").expect("valid test URL");
        engine.bridge_hello(app.as_str());
        let since = engine.hellos();
        engine.bridge_hello(app.as_str());
        assert!(
            !engine
                .wait_bridge(&foreign, since, Duration::from_secs(1))
                .await,
            "a later hello from the app origin must not count for a foreign dest"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_wait_bridge_ignores_hellos_before_since() {
        let engine = EvalEngine::new();
        let app = Url::parse("tauri://localhost/").expect("valid test URL");
        engine.bridge_hello(app.as_str());
        let since = engine.hellos();
        assert!(
            !engine
                .wait_bridge(&app, since, Duration::from_secs(1))
                .await,
            "an old hello must not count as the new page's"
        );
        engine.bridge_hello(app.as_str());
        assert!(
            engine
                .wait_bridge(&app, since, Duration::from_secs(1))
                .await
        );
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
