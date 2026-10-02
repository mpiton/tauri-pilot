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
    ///
    /// Returns `None` when no recording is in progress, so a caller cannot
    /// mistake "nothing to stop" for "stopped with nothing recorded". The check
    /// and the take share one lock, so two concurrent stops cannot both win.
    pub fn stop(&self) -> Option<Vec<RecordEntry>> {
        let mut s = self.state.lock().expect("recorder lock poisoned");
        if !s.active {
            return None;
        }
        s.active = false;
        s.start_time = None;
        Some(std::mem::take(&mut s.entries))
    }

    pub fn is_active(&self) -> bool {
        self.state.lock().expect("recorder lock poisoned").active
    }

    /// Record a method call if recording is active and the method is recordable.
    ///
    /// Callers pass the *original* params: the `"window"` key stays on the
    /// entry so a replay targets the window the step ran in (#276).
    /// `locators` is the bridge's `locate` answer for [`Self::locate_request`]:
    /// each located ref gets its `selector` and `expect` fingerprint next to it.
    pub fn record(&self, method: &str, params: Option<&Value>, locators: Option<&Value>) {
        if !is_recordable(method) {
            return;
        }

        let mut s = self.state.lock().expect("recorder lock poisoned");
        if !s.active {
            return;
        }

        let timestamp = u64::try_from(
            s.start_time
                .expect("start_time must be set when active")
                .elapsed()
                .as_millis(),
        )
        .unwrap_or(u64::MAX);

        let mut map = params
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        for (key, located) in locators.and_then(Value::as_object).into_iter().flatten() {
            let target = if key == SELF_TARGET {
                Some(&mut map)
            } else {
                map.get_mut(key).and_then(Value::as_object_mut)
            };
            if let (Some(target), Some(located)) = (target, located.as_object()) {
                target.extend(located.clone());
            }
        }

        s.entries.push(RecordEntry {
            action: method.to_string(),
            timestamp,
            params: map,
        });
    }

    /// The bridge `locate` params for the refs a step targets, if any.
    ///
    /// Returns `None` unless recording is active and the step is recordable
    /// and targets a ref, so the extra round trip only happens while
    /// recording. The step's own ref is keyed [`SELF_TARGET`]; `drag` refs
    /// are keyed `source` and `target`.
    pub fn locate_request(&self, method: &str, params: Option<&Value>) -> Option<Value> {
        if !is_recordable(method) || !self.is_active() {
            return None;
        }
        let refs: serde_json::Map<String, Value> = ref_targets(params?)
            .map(|(key, target)| (key.to_owned(), target["ref"].clone()))
            .collect();
        (!refs.is_empty()).then(|| serde_json::json!({ "refs": refs }))
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
        let elapsed_ms: u64 = s.start_time.map_or(0, |t| {
            u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX)
        });
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

/// Key of a step's own ref in `locate` requests and answers.
const SELF_TARGET: &str = "self";

/// The ref targets of a step: `(key, object holding "ref")`.
///
/// The step itself is keyed [`SELF_TARGET`]; `drag` nests its refs under
/// `source` and `target`.
fn ref_targets(params: &Value) -> impl Iterator<Item = (&str, &Value)> {
    [
        (SELF_TARGET, Some(params)),
        ("source", params.get("source")),
        ("target", params.get("target")),
    ]
    .into_iter()
    .filter_map(|(key, target)| Some((key, target?)))
    .filter(|(_, target)| target.get("ref").is_some_and(Value::is_string))
}

/// Steps that target a ref but carry no stable selector.
///
/// Each item is `{"step": n, "action": ..., "ref": ...}` with `step`
/// counted from 1. These steps can only replay against the snapshot that
/// numbered the ref, so `record stop` reports them (#276).
pub(crate) fn unstable_steps(entries: &[RecordEntry]) -> Vec<Value> {
    entries
        .iter()
        .enumerate()
        .flat_map(|(i, entry)| {
            let params = Value::Object(entry.params.clone());
            ref_targets(&params)
                .filter(|(_, target)| target.get("selector").is_none())
                .map(|(_, target)| {
                    serde_json::json!({
                        "step": i + 1,
                        "action": entry.action,
                        "ref": target["ref"],
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect()
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
        rec.record("click", Some(&json!({"ref": "e1"})), None);
        let entries = rec.stop().expect("recording active");
        assert_eq!(entries.len(), 1);
        assert!(!rec.is_active());
    }

    #[test]
    fn test_stop_without_start_returns_none() {
        assert!(Recorder::new().stop().is_none());
    }

    #[test]
    fn test_second_stop_returns_none() {
        let rec = Recorder::new();
        rec.start();
        rec.record("click", Some(&json!({"ref": "e1"})), None);
        assert!(rec.stop().is_some());
        assert!(rec.stop().is_none());
    }

    #[test]
    fn test_record_adds_entry_when_active() {
        let rec = Recorder::new();
        rec.start();
        rec.record("click", Some(&json!({"ref": "e1"})), None);
        let entries = rec.stop().expect("recording active");
        assert_eq!(entries[0].action, "click");
        assert_eq!(entries[0].params.get("ref").expect("ref recorded"), "e1");
    }

    #[test]
    fn test_record_ignores_when_inactive() {
        let rec = Recorder::new();
        rec.record("click", Some(&json!({"ref": "e1"})), None);
        // No entries since recorder was never started
        rec.start();
        let entries = rec.stop().expect("recording active");
        assert!(entries.is_empty());
    }

    /// #276: a session that drives several windows must replay each step in
    /// the window it was recorded in.
    #[test]
    fn test_record_keeps_window_param() {
        let rec = Recorder::new();
        rec.start();
        rec.record(
            "fill",
            Some(&json!({"ref": "e1", "value": "hello", "window": "settings"})),
            None,
        );
        let entries = rec.stop().expect("recording active");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].params["window"], "settings");
        assert_eq!(
            entries[0].params.get("value").expect("value recorded"),
            "hello"
        );
    }

    /// #276: the locator the bridge computed is stored next to the ref.
    #[test]
    fn test_record_stores_locator_next_to_the_ref() {
        let rec = Recorder::new();
        rec.start();
        let fingerprint = json!({"tag": "input", "role": "radio", "name": "Pro"});
        rec.record(
            "check",
            Some(&json!({"ref": "e9"})),
            Some(&json!({"self": {"selector": "#plan-pro", "expect": fingerprint}})),
        );
        let entries = rec.stop().expect("recording active");
        let entry = serde_json::to_value(&entries[0]).expect("entry serializes");
        assert_eq!(
            entry,
            json!({
                "action": "check",
                "timestamp": entries[0].timestamp,
                "ref": "e9",
                "selector": "#plan-pro",
                "expect": {"tag": "input", "role": "radio", "name": "Pro"},
            })
        );
    }

    #[test]
    fn test_record_stores_drag_source_and_target_locators() {
        let rec = Recorder::new();
        rec.start();
        rec.record(
            "drag",
            Some(&json!({"source": {"ref": "e3"}, "target": {"selector": "#col-done"}})),
            Some(&json!({"source": {"selector": "#card-1", "expect": {"tag": "div"}}})),
        );
        let entries = rec.stop().expect("recording active");
        assert_eq!(
            entries[0].params["source"],
            json!({"ref": "e3", "selector": "#card-1", "expect": {"tag": "div"}})
        );
        assert_eq!(
            entries[0].params["target"],
            json!({"selector": "#col-done"})
        );
    }

    #[test]
    fn test_locate_request_lists_the_refs_of_a_recordable_step() {
        let rec = Recorder::new();
        rec.start();
        assert_eq!(
            rec.locate_request("check", Some(&json!({"ref": "e9", "window": "main"}))),
            Some(json!({"refs": {"self": "e9"}}))
        );
        assert_eq!(
            rec.locate_request(
                "drag",
                Some(&json!({"source": {"ref": "e1"}, "target": {"ref": "e2"}}))
            ),
            Some(json!({"refs": {"source": "e1", "target": "e2"}}))
        );
    }

    #[test]
    fn test_locate_request_is_none_without_a_ref_to_locate() {
        let rec = Recorder::new();
        // Not recording: no extra round trip.
        assert_eq!(
            rec.locate_request("click", Some(&json!({"ref": "e1"}))),
            None
        );
        rec.start();
        assert_eq!(
            rec.locate_request("click", Some(&json!({"selector": "#go"}))),
            None
        );
        assert_eq!(
            rec.locate_request("text", Some(&json!({"ref": "e1"}))),
            None
        );
        assert_eq!(
            rec.locate_request("navigate", Some(&json!({"url": "/"}))),
            None
        );
    }

    #[test]
    fn test_unstable_steps_lists_refs_without_a_selector() {
        let rec = Recorder::new();
        rec.start();
        rec.record(
            "check",
            Some(&json!({"ref": "e9"})),
            Some(&json!({"self": {"selector": "#pro", "expect": {"tag": "input"}}})),
        );
        rec.record(
            "click",
            Some(&json!({"ref": "e4"})),
            Some(&json!({"self": {"expect": {"tag": "div"}}})),
        );
        rec.record("click", Some(&json!({"selector": "#go"})), None);
        rec.record(
            "drag",
            Some(&json!({"source": {"ref": "e1"}, "target": {"selector": "#col"}})),
            None,
        );
        let entries = rec.stop().expect("recording active");
        assert_eq!(
            unstable_steps(&entries),
            vec![
                json!({"step": 2, "action": "click", "ref": "e4"}),
                json!({"step": 4, "action": "drag", "ref": "e1"}),
            ]
        );
    }

    #[test]
    fn test_timestamp_relative_to_start() {
        let rec = Recorder::new();
        rec.start();
        std::thread::sleep(std::time::Duration::from_millis(10));
        rec.record("click", Some(&json!({"ref": "e1"})), None);
        let entries = rec.stop().expect("recording active");
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
        let entries = rec.stop().expect("recording active");
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
        rec.record("click", Some(&json!({"ref": "e1"})), None);
        let status = rec.status();
        assert_eq!(status["active"], true);
        assert_eq!(status["count"], 1);

        rec.stop();
        let status = rec.status();
        assert_eq!(status["active"], false);
        assert_eq!(status["count"], 0);
        assert_eq!(status["elapsed_ms"], 0);
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
        let entries = rec.stop().expect("recording active");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_record_ignores_non_recordable_method() {
        let rec = Recorder::new();
        rec.start();
        rec.record("snapshot", None, None);
        rec.record("ping", None, None);
        rec.record("eval", None, None);
        let entries = rec.stop().expect("recording active");
        assert!(entries.is_empty());
    }
}
