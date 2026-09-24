use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotElement {
    #[serde(rename = "ref")]
    pub ref_id: String,
    pub role: String,
    pub depth: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangedEntry {
    pub old: SnapshotElement,
    pub new: SnapshotElement,
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffResult {
    pub added: Vec<SnapshotElement>,
    pub removed: Vec<SnapshotElement>,
    pub changed: Vec<ChangedEntry>,
}

/// Identity key for stable matching between snapshots.
/// Uses (role, name, depth) since refs reset each snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ElementKey {
    role: String,
    name: Option<String>,
    depth: u64,
}

impl ElementKey {
    fn from(el: &SnapshotElement) -> Self {
        Self {
            role: el.role.clone(),
            name: el.name.clone(),
            depth: el.depth,
        }
    }
}

/// Compare two snapshot element lists and return added, removed, and changed entries.
///
/// Match elements by (role, name, depth). For duplicate keys, match by position order
/// within the group. Unmatched old → removed, unmatched new → added.
#[must_use]
pub fn compute_diff(old: &[SnapshotElement], new: &[SnapshotElement]) -> DiffResult {
    let mut old_groups: HashMap<ElementKey, Vec<usize>> = HashMap::new();
    for (i, el) in old.iter().enumerate() {
        old_groups.entry(ElementKey::from(el)).or_default().push(i);
    }

    let mut new_groups: HashMap<ElementKey, Vec<usize>> = HashMap::new();
    for (i, el) in new.iter().enumerate() {
        new_groups.entry(ElementKey::from(el)).or_default().push(i);
    }

    let mut matched_old: Vec<bool> = vec![false; old.len()];
    let mut matched_new: Vec<bool> = vec![false; new.len()];
    let mut changed: Vec<ChangedEntry> = Vec::new();

    // Match by key and position within group
    for (key, old_indices) in &old_groups {
        if let Some(new_indices) = new_groups.get(key) {
            let pair_count = old_indices.len().min(new_indices.len());
            for i in 0..pair_count {
                let old_idx = old_indices[i];
                let new_idx = new_indices[i];
                matched_old[old_idx] = true;
                matched_new[new_idx] = true;

                let old_el = &old[old_idx];
                let new_el = &new[new_idx];
                let mut field_changes: Vec<String> = Vec::new();

                if old_el.value != new_el.value {
                    field_changes.push("value".to_owned());
                }
                if old_el.checked != new_el.checked {
                    field_changes.push("checked".to_owned());
                }
                if old_el.disabled != new_el.disabled {
                    field_changes.push("disabled".to_owned());
                }

                if !field_changes.is_empty() {
                    changed.push(ChangedEntry {
                        old: old_el.clone(),
                        new: new_el.clone(),
                        changes: field_changes,
                    });
                }
            }
        }
    }

    let mut removed: Vec<SnapshotElement> = old
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_old[*i])
        .map(|(_, el)| el.clone())
        .collect();

    let mut added: Vec<SnapshotElement> = new
        .iter()
        .enumerate()
        .filter(|(i, _)| !matched_new[*i])
        .map(|(_, el)| el.clone())
        .collect();

    // Sort all result arrays for deterministic output (HashMap iteration is unordered)
    let sort_key = |a: &SnapshotElement, b: &SnapshotElement| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| a.role.cmp(&b.role))
            .then_with(|| a.name.cmp(&b.name))
    };
    added.sort_by(sort_key);
    removed.sort_by(sort_key);
    changed.sort_by(|a, b| sort_key(&a.new, &b.new));

    DiffResult {
        added,
        removed,
        changed,
    }
}

/// Options that decide which elements a snapshot contains.
///
/// Recorded next to `elements` because two snapshots captured with different
/// options differ in every element those options filter, not in page content.
/// `diff` compares them before diffing (#244).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaptureOptions {
    /// Only interactive elements were kept.
    pub interactive: bool,
    /// Root of the captured subtree; `depth` counts from it.
    pub selector: Option<String>,
    /// Maximum traversal depth, `None` for the bridge default.
    pub depth: Option<u64>,
}

impl CaptureOptions {
    /// Reads the options from `snapshot` or `diff` params.
    ///
    /// Normalizes the way the bridge applies them: a missing `interactive` is
    /// `false` and an empty `selector` is no selector.
    #[must_use]
    pub fn from_params(params: Option<&serde_json::Value>) -> Self {
        let get = |key: &str| params.and_then(|p| p.get(key));
        Self {
            interactive: get("interactive")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            selector: get("selector")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            depth: get("depth").and_then(serde_json::Value::as_u64),
        }
    }
}

/// Warning for a reference snapshot that records no capture options.
pub const UNRECORDED_OPTIONS_WARNING: &str = "Reference snapshot does not record its capture \
options (saved by tauri-pilot 0.7.3 or earlier), so it cannot be checked against this diff. \
If it was captured with other --interactive, --selector or --depth values, the elements they \
filter show up as added or removed. Re-save it to enable the check.";

/// Returns the options recorded in `snapshot`, or `None` when it has none.
#[must_use]
pub fn recorded_options(snapshot: &serde_json::Value) -> Option<CaptureOptions> {
    snapshot
        .get("options")
        .filter(|o| !o.is_null())
        .map(|o| CaptureOptions::from_params(Some(o)))
}

/// Describes how `reference` differs from `current`, or `None` if they match.
#[must_use]
pub fn options_mismatch(reference: &CaptureOptions, current: &CaptureOptions) -> Option<String> {
    let selector = |s: &Option<String>| s.as_ref().map_or("none".to_owned(), |s| format!("{s:?}"));
    let depth = |d: Option<u64>| d.map_or("none".to_owned(), |d| d.to_string());
    let mut fields = Vec::new();
    if reference.interactive != current.interactive {
        fields.push(format!(
            "interactive (reference: {}, current: {})",
            reference.interactive, current.interactive
        ));
    }
    if reference.selector != current.selector {
        fields.push(format!(
            "selector (reference: {}, current: {})",
            selector(&reference.selector),
            selector(&current.selector)
        ));
    }
    if reference.depth != current.depth {
        fields.push(format!(
            "depth (reference: {}, current: {})",
            depth(reference.depth),
            depth(current.depth)
        ));
    }
    (!fields.is_empty()).then(|| {
        format!(
            "Reference snapshot was captured with other options: {}. Diffing them would report \
             every element those options filter as added or removed. Re-run diff with the \
             reference's options, or take a new reference snapshot with the current ones.",
            fields.join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(ref_id: &str, role: &str, depth: u64) -> SnapshotElement {
        SnapshotElement {
            ref_id: ref_id.to_owned(),
            role: role.to_owned(),
            depth,
            name: None,
            value: None,
            checked: None,
            disabled: None,
        }
    }

    fn el_named(ref_id: &str, role: &str, depth: u64, name: &str) -> SnapshotElement {
        SnapshotElement {
            ref_id: ref_id.to_owned(),
            role: role.to_owned(),
            depth,
            name: Some(name.to_owned()),
            value: None,
            checked: None,
            disabled: None,
        }
    }

    #[test]
    fn test_snapshot_element_deserializes_string_value() {
        // Regression for #120: the bridge emits `value` as a string, and the
        // reference-snapshot parse in `diff` must accept it. Reference files
        // saved before #162 still carry "0" on every bare <li>.
        let json =
            r#"{"ref":"e31","role":"listitem","depth":3,"name":"single hyphen value","value":"0"}"#;
        let el: SnapshotElement =
            serde_json::from_str(json).expect("string value should deserialize");
        assert_eq!(el.value, Some("0".to_owned()));
    }

    #[test]
    fn test_diff_identical_snapshots() {
        let snapshot = vec![el("e1", "button", 1), el("e2", "input", 2)];
        let result = compute_diff(&snapshot, &snapshot);
        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
        assert!(result.changed.is_empty());
    }

    #[test]
    fn test_diff_added_elements() {
        let old = vec![el("e1", "button", 1)];
        let new = vec![el("e1", "button", 1), el("e2", "input", 2)];
        let result = compute_diff(&old, &new);
        assert_eq!(result.added.len(), 1);
        assert_eq!(result.added[0].role, "input");
        assert!(result.removed.is_empty());
        assert!(result.changed.is_empty());
    }

    #[test]
    fn test_diff_removed_elements() {
        let old = vec![el("e1", "button", 1), el("e2", "input", 2)];
        let new = vec![el("e1", "button", 1)];
        let result = compute_diff(&old, &new);
        assert!(result.added.is_empty());
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].role, "input");
        assert!(result.changed.is_empty());
    }

    #[test]
    fn test_diff_changed_value() {
        let old = vec![SnapshotElement {
            ref_id: "e1".to_owned(),
            role: "input".to_owned(),
            depth: 1,
            name: Some("username".to_owned()),
            value: Some("old".to_owned()),
            checked: None,
            disabled: None,
        }];
        let new = vec![SnapshotElement {
            ref_id: "e2".to_owned(),
            role: "input".to_owned(),
            depth: 1,
            name: Some("username".to_owned()),
            value: Some("new".to_owned()),
            checked: None,
            disabled: None,
        }];
        let result = compute_diff(&old, &new);
        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
        assert_eq!(result.changed.len(), 1);
        assert_eq!(result.changed[0].changes, vec!["value"]);
    }

    #[test]
    fn test_diff_changed_multiple_fields() {
        let old = vec![SnapshotElement {
            ref_id: "e1".to_owned(),
            role: "checkbox".to_owned(),
            depth: 2,
            name: Some("agree".to_owned()),
            value: Some("on".to_owned()),
            checked: Some(false),
            disabled: Some(false),
        }];
        let new = vec![SnapshotElement {
            ref_id: "e2".to_owned(),
            role: "checkbox".to_owned(),
            depth: 2,
            name: Some("agree".to_owned()),
            value: Some("off".to_owned()),
            checked: Some(true),
            disabled: Some(true),
        }];
        let result = compute_diff(&old, &new);
        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
        assert_eq!(result.changed.len(), 1);
        assert!(result.changed[0].changes.contains(&"value".to_owned()));
        assert!(result.changed[0].changes.contains(&"checked".to_owned()));
        assert!(result.changed[0].changes.contains(&"disabled".to_owned()));
    }

    #[test]
    fn test_diff_mixed_changes() {
        let old = vec![
            el_named("e1", "button", 1, "submit"),
            SnapshotElement {
                ref_id: "e2".to_owned(),
                role: "input".to_owned(),
                depth: 2,
                name: Some("email".to_owned()),
                value: Some("old@example.com".to_owned()),
                checked: None,
                disabled: None,
            },
            el_named("e3", "link", 3, "home"),
        ];
        let new = vec![
            el_named("e1", "button", 1, "submit"),
            SnapshotElement {
                ref_id: "e4".to_owned(),
                role: "input".to_owned(),
                depth: 2,
                name: Some("email".to_owned()),
                value: Some("new@example.com".to_owned()),
                checked: None,
                disabled: None,
            },
            el_named("e5", "paragraph", 4, "info"),
        ];
        let result = compute_diff(&old, &new);
        assert_eq!(result.added.len(), 1);
        assert_eq!(result.added[0].role, "paragraph");
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].role, "link");
        assert_eq!(result.changed.len(), 1);
        assert_eq!(result.changed[0].changes, vec!["value"]);
    }

    #[test]
    fn test_diff_empty_old() {
        let old: Vec<SnapshotElement> = vec![];
        let new = vec![el("e1", "button", 1), el("e2", "input", 2)];
        let result = compute_diff(&old, &new);
        assert_eq!(result.added.len(), 2);
        assert!(result.removed.is_empty());
        assert!(result.changed.is_empty());
    }

    #[test]
    fn test_diff_duplicate_roles() {
        // Two buttons at same depth — matched by position order within group
        let old = vec![
            SnapshotElement {
                ref_id: "e1".to_owned(),
                role: "button".to_owned(),
                depth: 1,
                name: None,
                value: Some("save".to_owned()),
                checked: None,
                disabled: None,
            },
            SnapshotElement {
                ref_id: "e2".to_owned(),
                role: "button".to_owned(),
                depth: 1,
                name: None,
                value: Some("cancel".to_owned()),
                checked: None,
                disabled: None,
            },
        ];
        let new = vec![
            SnapshotElement {
                ref_id: "e3".to_owned(),
                role: "button".to_owned(),
                depth: 1,
                name: None,
                value: Some("save".to_owned()),
                checked: None,
                disabled: None,
            },
            SnapshotElement {
                ref_id: "e4".to_owned(),
                role: "button".to_owned(),
                depth: 1,
                name: None,
                value: Some("cancel".to_owned()),
                checked: None,
                disabled: None,
            },
        ];
        let result = compute_diff(&old, &new);
        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
        assert!(result.changed.is_empty());
    }

    #[test]
    fn test_capture_options_normalize_like_the_bridge() {
        // The bridge reads `interactive || false` and `selector || null`, so a
        // missing flag and an empty selector capture the same tree as the
        // defaults and must not count as a mismatch.
        let defaults = CaptureOptions::from_params(None);
        let explicit = CaptureOptions::from_params(Some(
            &serde_json::json!({"interactive": false, "selector": "", "depth": null}),
        ));
        assert_eq!(defaults, explicit);
        assert_eq!(
            serde_json::to_value(&defaults).expect("serialize"),
            serde_json::json!({"interactive": false, "selector": null, "depth": null})
        );
    }

    #[test]
    fn test_recorded_options_absent_in_pre_244_snapshot() {
        let legacy = serde_json::json!({"elements": []});
        assert_eq!(recorded_options(&legacy), None);
    }

    #[test]
    fn test_recorded_options_read_back() {
        let snapshot = serde_json::json!({
            "elements": [],
            "options": {"interactive": true, "selector": "#app", "depth": 3}
        });
        let options = recorded_options(&snapshot).expect("options recorded");
        assert!(options.interactive);
        assert_eq!(options.selector.as_deref(), Some("#app"));
        assert_eq!(options.depth, Some(3));
    }

    #[test]
    fn test_options_mismatch_none_when_equal() {
        let options = CaptureOptions::from_params(None);
        assert_eq!(options_mismatch(&options, &options), None);
    }

    #[test]
    fn test_options_mismatch_names_every_differing_option() {
        let reference = CaptureOptions::from_params(None);
        let current = CaptureOptions::from_params(Some(
            &serde_json::json!({"interactive": true, "selector": "#app", "depth": 2}),
        ));
        let message = options_mismatch(&reference, &current).expect("mismatch");
        assert!(
            message.contains("interactive (reference: false, current: true)"),
            "{message}"
        );
        assert!(
            message.contains("selector (reference: none, current: \"#app\")"),
            "{message}"
        );
        assert!(
            message.contains("depth (reference: none, current: 2)"),
            "{message}"
        );
    }
}
