//! Unit tests for the snapshot-diff formatter.

use super::diff::format_diff;
use serde_json::json;

#[test]
fn test_format_diff_no_changes() {
    // Should not panic and print "No changes detected."
    format_diff(&json!({"added": [], "removed": [], "changed": []}));
    format_diff(&json!({}));
}

#[test]
fn test_format_diff_added() {
    let diff = json!({
        "added": [{"ref": "e8", "role": "button", "depth": 0, "name": "Submit"}],
        "removed": [],
        "changed": []
    });
    // Just verify it doesn't panic — output goes to stdout
    format_diff(&diff);
}

#[test]
fn test_format_diff_removed() {
    let diff = json!({
        "added": [],
        "removed": [{"ref": "e3", "role": "button", "depth": 0, "name": "Loading..."}],
        "changed": []
    });
    format_diff(&diff);
}

#[test]
fn test_format_diff_changed() {
    let diff = json!({
        "added": [],
        "removed": [],
        "changed": [{
            "old": {"ref": "e2", "role": "textbox", "name": "Search", "value": ""},
            "new": {"ref": "e2", "role": "textbox", "name": "Search", "value": "workspace"},
            "changes": ["value"]
        }]
    });
    format_diff(&diff);
}

#[test]
fn test_format_diff_mixed() {
    let diff = json!({
        "added": [{"ref": "e9", "role": "link", "name": "Home"}],
        "removed": [{"ref": "e1", "role": "button", "name": "Old"}],
        "changed": [{
            "old": {"ref": "e5", "role": "checkbox", "name": "Agree", "checked": false},
            "new": {"ref": "e5", "role": "checkbox", "name": "Agree", "checked": true},
            "changes": ["checked"]
        }]
    });
    format_diff(&diff);
}
