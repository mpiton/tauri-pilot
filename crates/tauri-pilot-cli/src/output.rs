use std::fmt::Write;

use anyhow::Result;

/// Print a value as pretty JSON.
pub(crate) fn format_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Print a value as compact text for human consumption.
pub(crate) fn format_text(value: &serde_json::Value) {
    // {ok: true} → "ok", {found: true} → "found"
    for key in ["ok", "found"] {
        if value.get(key).and_then(serde_json::Value::as_bool) == Some(true) {
            println!("{key}");
            return;
        }
    }
    // {status: "ok"} → "ok"
    if let Some(status) = value.get("status").and_then(serde_json::Value::as_str) {
        println!("{status}");
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

        let mut line = format!("{indent}- {role}");

        if let Some(name) = el.get("name").and_then(serde_json::Value::as_str) {
            let _ = write!(line, " \"{name}\"");
        }

        let _ = write!(line, " [ref={ref}]");

        if let Some(val) = el.get("value").and_then(serde_json::Value::as_str) {
            let _ = write!(line, " value=\"{val}\"");
        }
        if el.get("checked").and_then(serde_json::Value::as_bool) == Some(true) {
            line.push_str(" checked");
        }
        if el.get("disabled").and_then(serde_json::Value::as_bool) == Some(true) {
            line.push_str(" disabled");
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
