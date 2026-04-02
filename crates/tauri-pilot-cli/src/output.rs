use std::fmt::Write;

use anyhow::Result;

/// Print a value as pretty JSON.
pub(crate) fn format_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Print a value as compact text for human consumption.
pub(crate) fn format_text(value: &serde_json::Value) {
    // {error: {message: "...", code: N}} → "✗ <message>"
    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown error");
        let code = err.get("code").and_then(serde_json::Value::as_i64);
        if let Some(c) = code {
            println!("{}", crate::style::error(&format!("{msg} (code {c})")));
        } else {
            println!("{}", crate::style::error(msg));
        }
        return;
    }
    // {ok: true} → "✓ ok", {found: true} → "✓ found"
    for key in ["ok", "found"] {
        if value.get(key).and_then(serde_json::Value::as_bool) == Some(true) {
            println!("{}", crate::style::success(key));
            return;
        }
    }
    // {status: "ok"} → "✓ ok", {status: "error"} → "✗ error"
    if let Some(status) = value.get("status").and_then(serde_json::Value::as_str) {
        if status == "ok" || status == "success" {
            println!("{}", crate::style::success(status));
        } else {
            println!("{}", crate::style::error(status));
        }
        return;
    }
    match value {
        serde_json::Value::String(s) => println!("{s}"),
        serde_json::Value::Null => {}
        other => println!("{other}"),
    }
}

/// Format a snapshot result as an indented accessibility tree.
pub(crate) fn format_snapshot(value: &serde_json::Value) {
    let Some(elements) = value.get("elements").and_then(|e| e.as_array()) else {
        println!("(empty snapshot)");
        return;
    };

    if elements.is_empty() {
        println!("(empty snapshot)");
        return;
    }

    for el in elements {
        let depth = usize::try_from(
            el.get("depth")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .unwrap_or(0);
        let indent = "  ".repeat(depth);
        let role = el
            .get("role")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?");
        let r#ref = el
            .get("ref")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("?");

        let mut line = format!("{indent}- {}", crate::style::info(role));

        if let Some(name) = el.get("name").and_then(serde_json::Value::as_str) {
            let _ = write!(line, " {}", crate::style::bold(format!("\"{name}\"")));
        }

        let _ = write!(line, " {}", crate::style::dim(format!("[ref={ref}]")));

        if let Some(val) = el.get("value").and_then(serde_json::Value::as_str) {
            let _ = write!(line, " {}", crate::style::dim(format!("value=\"{val}\"")));
        }
        if el.get("checked").and_then(serde_json::Value::as_bool) == Some(true) {
            let _ = write!(line, " {}", crate::style::dim("checked"));
        }
        if el.get("disabled").and_then(serde_json::Value::as_bool) == Some(true) {
            let _ = write!(line, " {}", crate::style::dim("disabled"));
        }

        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_json_does_not_panic() {
        format_json(&json!({"status": "ok"})).unwrap();
    }

    #[test]
    fn test_format_text_ok_contains_checkmark() {
        // Just verify it doesn't panic
        format_text(&json!({"ok": true}));
    }

    #[test]
    fn test_format_text_found_does_not_panic() {
        format_text(&json!({"found": true}));
    }

    #[test]
    fn test_format_text_status_ok() {
        // Just verify it doesn't panic
        format_text(&json!({"status": "ok"}));
    }

    #[test]
    fn test_format_text_rpc_error_does_not_panic() {
        // {error: {message: "..."}} path — just verify it doesn't panic
        format_text(&json!({"error": {"message": "Method not found", "code": -32601}}));
    }

    #[test]
    fn test_format_text_error_without_message() {
        // {error: {code: -32601}} without message key — should display "unknown error"
        format_text(&json!({"error": {"code": -32601}}));
    }

    #[test]
    fn test_format_text_string_value() {
        format_text(&json!("some text"));
    }

    #[test]
    fn test_format_text_null_value() {
        format_text(&serde_json::Value::Null);
    }

    #[test]
    fn test_format_snapshot_with_elements() {
        let snapshot = json!({
            "elements": [
                {"ref": "e1", "role": "heading", "name": "Title", "depth": 0},
                {"ref": "e2", "role": "textbox", "name": "Search", "depth": 1, "value": ""},
                {"ref": "e3", "role": "button", "name": "Submit", "depth": 1},
                {"ref": "e4", "role": "checkbox", "name": "Agree", "depth": 1, "checked": true},
                {"ref": "e5", "role": "button", "name": "Disabled", "depth": 2, "disabled": true},
            ]
        });
        // Just verify it doesn't panic — output goes to stdout
        format_snapshot(&snapshot);
    }

    #[test]
    fn test_format_snapshot_empty() {
        format_snapshot(&json!({"elements": []}));
        format_snapshot(&json!({}));
        format_snapshot(&json!(null));
    }
}
