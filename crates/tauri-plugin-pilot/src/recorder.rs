use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A single recorded user action.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct RecordEntry {
    pub action: String,
    pub timestamp: u64,
    #[serde(flatten)]
    pub params: serde_json::Map<String, Value>,
}

struct RecorderState {
    active: bool,
    start_time: Option<Instant>,
    entries: Vec<RecordEntry>,
}

/// Recording engine — wraps state in `Arc<Mutex<...>>` so it can be cloned
/// and shared across connection tasks (same pattern as `EvalEngine`).
#[derive(Clone)]
pub(crate) struct Recorder {
    state: Arc<Mutex<RecorderState>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RecorderState {
                active: false,
                start_time: None,
                entries: Vec::new(),
            })),
        }
    }

    /// Activate recording, reset entries and start the clock.
    pub fn start(&self) {
        let mut s = self.state.lock().expect("recorder lock poisoned");
        s.active = true;
        s.start_time = Some(Instant::now());
        s.entries.clear();
    }

    /// Deactivate recording and return all collected entries.
    pub fn stop(&self) -> Vec<RecordEntry> {
        let mut s = self.state.lock().expect("recorder lock poisoned");
        s.active = false;
        std::mem::take(&mut s.entries)
    }

    pub fn is_active(&self) -> bool {
        self.state.lock().expect("recorder lock poisoned").active
    }

    /// Record a method call if recording is active and the method is recordable.
    ///
    /// The `"window"` key is stripped from params internally — callers should
    /// pass the *original* params (before `extract_window` strips them) so the
    /// recorder can clean up itself.
    pub fn record(&self, method: &str, params: Option<&Value>) {
        if !is_recordable(method) {
            return;
        }

        let mut s = self.state.lock().expect("recorder lock poisoned");
        if !s.active {
            return;
        }

        #[allow(clippy::cast_possible_truncation)]
        let timestamp = s
            .start_time
            .expect("start_time must be set when active")
            .elapsed()
            .as_millis() as u64;

        let mut map = params
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        map.remove("window");

        s.entries.push(RecordEntry {
            action: method.to_string(),
            timestamp,
            params: map,
        });
    }

    /// Explicitly add an entry (used by CLI-side `record.add`).
    /// Only adds if recording is active.
    pub fn add_entry(&self, entry: RecordEntry) {
        let mut s = self.state.lock().expect("recorder lock poisoned");
        if s.active {
            s.entries.push(entry);
        }
    }

    /// Return a JSON status snapshot: active flag, entry count, elapsed ms.
    pub fn status(&self) -> Value {
        let s = self.state.lock().expect("recorder lock poisoned");
        #[allow(clippy::cast_possible_truncation)]
        let elapsed_ms: u64 = s.start_time.map_or(0, |t| t.elapsed().as_millis() as u64);
        serde_json::json!({
            "active": s.active,
            "count": s.entries.len(),
            "elapsed_ms": elapsed_ms,
        })
    }
}

/// Returns `true` for methods that should be captured during recording.
fn is_recordable(method: &str) -> bool {
    matches!(
        method,
        "click"
            | "fill"
            | "type"
            | "press"
            | "select"
            | "check"
            | "scroll"
            | "drag"
            | "drop"
            | "navigate"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_start_activates_recording() {
        let rec = Recorder::new();
        assert!(!rec.is_active());
        rec.start();
        assert!(rec.is_active());
    }

    #[test]
    fn test_stop_returns_entries_and_deactivates() {
        let rec = Recorder::new();
        rec.start();
        rec.record("click", Some(&json!({"ref": "e1"})));
        let entries = rec.stop();
        assert_eq!(entries.len(), 1);
        assert!(!rec.is_active());
    }

    #[test]
    fn test_record_adds_entry_when_active() {
        let rec = Recorder::new();
        rec.start();
        rec.record("click", Some(&json!({"ref": "e1"})));
        let entries = rec.stop();
        assert_eq!(entries[0].action, "click");
        assert_eq!(entries[0].params.get("ref").unwrap(), "e1");
    }

    #[test]
    fn test_record_ignores_when_inactive() {
        let rec = Recorder::new();
        rec.record("click", Some(&json!({"ref": "e1"})));
        // No entries since recorder was never started
        rec.start();
        let entries = rec.stop();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_record_strips_window_param() {
        let rec = Recorder::new();
        rec.start();
        rec.record(
            "fill",
            Some(&json!({"ref": "e1", "value": "hello", "window": "main"})),
        );
        let entries = rec.stop();
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].params.contains_key("window"));
        assert_eq!(entries[0].params.get("value").unwrap(), "hello");
    }

    #[test]
    fn test_timestamp_relative_to_start() {
        let rec = Recorder::new();
        rec.start();
        std::thread::sleep(std::time::Duration::from_millis(10));
        rec.record("click", Some(&json!({"ref": "e1"})));
        let entries = rec.stop();
        assert!(
            entries[0].timestamp >= 10,
            "timestamp should be at least 10ms"
        );
    }

    #[test]
    fn test_add_entry_explicit() {
        let rec = Recorder::new();
        rec.start();
        let entry = RecordEntry {
            action: "navigate".to_string(),
            timestamp: 100,
            params: {
                let mut m = serde_json::Map::new();
                m.insert("url".to_string(), json!("/home"));
                m
            },
        };
        rec.add_entry(entry);
        let entries = rec.stop();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "navigate");
        assert_eq!(entries[0].timestamp, 100);
    }

    #[test]
    fn test_status_reports_correctly() {
        let rec = Recorder::new();
        let status = rec.status();
        assert_eq!(status["active"], false);
        assert_eq!(status["count"], 0);

        rec.start();
        rec.record("click", Some(&json!({"ref": "e1"})));
        let status = rec.status();
        assert_eq!(status["active"], true);
        assert_eq!(status["count"], 1);
    }

    #[test]
    fn test_add_entry_ignores_when_inactive() {
        let rec = Recorder::new();
        let entry = RecordEntry {
            action: "click".to_string(),
            timestamp: 0,
            params: serde_json::Map::new(),
        };
        rec.add_entry(entry);
        rec.start();
        let entries = rec.stop();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_record_ignores_non_recordable_method() {
        let rec = Recorder::new();
        rec.start();
        rec.record("snapshot", None);
        rec.record("ping", None);
        rec.record("eval", None);
        let entries = rec.stop();
        assert!(entries.is_empty());
    }
}
