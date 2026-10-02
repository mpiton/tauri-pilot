//! Reads the locator fields of a recorded step (#276).
//!
//! `record` stores, for each step that targets a snapshot ref, a stable
//! `selector` and an `expect` fingerprint next to the `ref`. `drag` stores
//! them on its `source` and `target` objects. A step with a ref and no
//! selector (a recording from before #276, or an element no selector could
//! single out) only replays against the snapshot that numbered the ref.

// Rust guideline compliant 2026-02-21
use serde_json::Value;

/// The objects of a step that may hold a target: the step itself, then the
/// `source` and `target` of a `drag`.
fn targets(entry: &Value) -> impl Iterator<Item = &Value> {
    [Some(entry), entry.get("source"), entry.get("target")]
        .into_iter()
        .flatten()
}

/// Refs the step relies on because no selector was recorded next to them.
pub(crate) fn ephemeral_refs(entry: &Value) -> Vec<&str> {
    targets(entry)
        .filter(|target| target.get("selector").is_none())
        .filter_map(|target| target.get("ref").and_then(Value::as_str))
        .collect()
}

/// Replay warning for a step that relies on `refs`, or `None` when it
/// relies on none.
pub(crate) fn ephemeral_ref_warning(refs: &[&str]) -> Option<String> {
    let (first, rest) = refs.split_first()?;
    let (noun, exist, it) = if rest.is_empty() {
        ("ref", "exists", "it")
    } else {
        ("refs", "exist", "them")
    };
    let mut list = (*first).to_owned();
    for r in rest {
        list.push_str(", ");
        list.push_str(r);
    }
    Some(format!(
        "relies on snapshot {noun} {list}, which only {exist} in the snapshot \
         that numbered {it}; re-record for a stable replay"
    ))
}

/// Whether any step of the recording carries an `expect` fingerprint.
pub(crate) fn has_fingerprint(entries: &[Value]) -> bool {
    entries
        .iter()
        .any(|entry| targets(entry).any(|target| target.get("expect").is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_ephemeral_refs_skips_refs_with_a_selector() {
        let entry = json!({
            "action": "drag",
            "source": {"ref": "e1", "selector": "#card"},
            "target": {"ref": "e2"},
        });
        assert_eq!(ephemeral_refs(&entry), ["e2"]);
        assert!(ephemeral_refs(&json!({"ref": "e9", "selector": "#pro"})).is_empty());
        assert_eq!(ephemeral_refs(&json!({"ref": "e9"})), ["e9"]);
    }

    #[test]
    fn test_ephemeral_ref_warning_names_every_ref() {
        assert_eq!(ephemeral_ref_warning(&[]), None);
        assert_eq!(
            ephemeral_ref_warning(&["e1", "e2"]).as_deref(),
            Some(
                "relies on snapshot refs e1, e2, which only exist in the snapshot \
                 that numbered them; re-record for a stable replay"
            )
        );
    }
}
