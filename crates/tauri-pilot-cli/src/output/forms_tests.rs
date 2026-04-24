//! Unit tests for the form dump formatter.

use super::forms::format_forms;
use serde_json::json;

#[test]
fn test_format_forms_basic() {
    let value = json!({
        "forms": [{
            "id": "login-form",
            "name": "",
            "action": "/login",
            "method": "post",
            "fields": [
                {"tag": "input", "type": "email", "name": "email", "value": "user@example.com", "checked": false},
                {"tag": "input", "type": "password", "name": "password", "value": "", "checked": false},
                {"tag": "input", "type": "checkbox", "name": "remember", "value": "", "checked": true},
            ]
        }]
    });
    format_forms(&value);
}

#[test]
fn test_format_forms_empty() {
    format_forms(&json!({"forms": []}));
}

#[test]
fn test_format_forms_no_forms_key() {
    format_forms(&json!({}));
}
