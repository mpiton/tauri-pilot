//! Unit tests for the DOM-mutation watch formatter.

use super::watch::format_watch;
use serde_json::json;

#[test]
fn test_format_watch_no_changes() {
    format_watch(&json!({"added": [], "removed": [], "modified": []}));
    format_watch(&json!({}));
}

#[test]
fn test_format_watch_added() {
    let result = json!({
        "added": [{"tag": "div", "class": "result", "text": "Hello"}],
        "removed": [],
        "modified": []
    });
    format_watch(&result);
}

#[test]
fn test_format_watch_mixed() {
    let result = json!({
        "added": [{"tag": "div", "id": "new", "text": "New item"}],
        "removed": [{"tag": "span", "class": "old"}],
        "modified": [
            {"tag": "div", "attribute": "class", "value": "active"},
            {"tag": "div", "attribute": "data-old", "removed": true},
            {"tag": "p", "text": "updated text"},
            {"tag": "section"}
        ],
        "truncated": true
    });
    format_watch(&result);
}
