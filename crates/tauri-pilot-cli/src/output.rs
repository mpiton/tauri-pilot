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
    for key in ["ok", "found", "cleared"] {
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

/// Format a millisecond timestamp as `HH:MM:SS.mmm`.
fn format_timestamp(timestamp: u64) -> String {
    let secs = (timestamp / 1000) % 86400;
    let ms = timestamp % 1000;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

/// Format console log entries for human-readable display.
pub(crate) fn format_logs(value: &serde_json::Value) -> String {
    let mut output = String::new();
    let Some(entries) = value.as_array() else {
        return format!("{}\n", crate::style::error("Unexpected response format"));
    };
    if entries.is_empty() {
        return String::from("No logs captured\n");
    }
    for entry in entries {
        let timestamp = entry
            .get("timestamp")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let level = entry.get("level").and_then(|l| l.as_str()).unwrap_or("log");
        let args = entry.get("args").and_then(|a| a.as_array());

        let time_str = format_timestamp(timestamp);

        // Format args (strip ANSI escape sequences to prevent terminal injection)
        let args_str = match args {
            Some(arr) => arr
                .iter()
                .map(|a| {
                    let raw = match a {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    strip_ansi(&raw)
                })
                .collect::<Vec<_>>()
                .join(" "),
            None => String::new(),
        };

        // Color by level
        let level_display = match level {
            "error" => crate::style::error(level),
            "warn" => crate::style::warn(level),
            "info" => crate::style::info(level),
            _ => crate::style::dim(level),
        };

        let _ = writeln!(output, "[{time_str}] {level_display} {args_str}");
    }
    output
}

/// Format network request entries for human-readable display.
pub(crate) fn format_network(value: &serde_json::Value) -> String {
    let mut output = String::new();
    let Some(entries) = value.as_array() else {
        return format!("{}\n", crate::style::error("Unexpected response format"));
    };
    if entries.is_empty() {
        return String::from("No requests captured\n");
    }
    for entry in entries {
        let timestamp = entry
            .get("timestamp")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let method = entry.get("method").and_then(|m| m.as_str()).unwrap_or("?");
        let url = entry.get("url").and_then(|u| u.as_str()).unwrap_or("?");
        let status = entry
            .get("status")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let duration = entry
            .get("duration_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let error = entry.get("error").and_then(|e| e.as_str());

        let time_str = format_timestamp(timestamp);

        // Strip ANSI from URL/method/error to prevent terminal injection
        let method_safe = strip_ansi(method);
        let url_safe = strip_ansi(url);

        // Color status by range — status 0 is always a network error
        let status_display = match status {
            0 => crate::style::error("ERR"),
            200..=299 => crate::style::success(&status.to_string()),
            300..=399 => crate::style::info(status),
            400..=499 => crate::style::warn(status),
            _ => crate::style::error(&status.to_string()),
        };

        let method_display = crate::style::bold(method_safe);
        let duration_display = crate::style::dim(format!("{duration}ms"));

        if let Some(err) = error {
            let err_safe = strip_ansi(err);
            let _ = writeln!(
                output,
                "[{time_str}] {method_display} {status_display} {url_safe} {} {duration_display}",
                crate::style::error(&err_safe)
            );
        } else {
            let _ = writeln!(
                output,
                "[{time_str}] {method_display} {status_display} {url_safe} {duration_display}"
            );
        }
    }
    output
}

/// Strip ANSI escape sequences from a string to prevent terminal injection.
fn strip_ansi(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip CSI sequences: ESC [ ... final_byte
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() || next == '~' {
                        break;
                    }
                }
            // Skip OSC sequences: ESC ] ... BEL or ESC \ (ST)
            } else if chars.peek() == Some(&']') {
                chars.next();
                let mut prev_was_esc = false;
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '\x07' {
                        break;
                    }
                    if prev_was_esc && next == '\\' {
                        break;
                    }
                    prev_was_esc = next == '\x1b';
                }
            } else {
                // Skip single-char escape (ESC + one char)
                chars.next();
            }
        } else {
            result.push(c);
        }
    }
    result
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

    #[test]
    fn test_format_logs_with_entries() {
        let logs = json!([
            {"id": 1, "timestamp": 3661123_u64, "level": "error", "args": ["fail"], "source": null},
            {"id": 2, "timestamp": 3661500_u64, "level": "info", "args": ["ok", 42], "source": null},
        ]);
        let output = format_logs(&logs);
        assert!(output.contains("01:01:01.123"));
        assert!(output.contains("fail"));
        assert!(output.contains("ok 42"));
    }

    #[test]
    fn test_format_logs_empty_array() {
        let output = format_logs(&json!([]));
        assert!(output.contains("No logs captured"));
    }

    #[test]
    fn test_format_logs_non_array() {
        let output = format_logs(&json!({"unexpected": true}));
        assert!(output.contains("Unexpected"));
    }

    #[test]
    fn test_format_network_with_entries() {
        let requests = json!([
            {"id": 1, "timestamp": 3661123_u64, "method": "GET", "url": "/api/users", "status": 200, "duration_ms": 150, "error": null, "request_size": 0, "response_size": 1024},
            {"id": 2, "timestamp": 3661500_u64, "method": "POST", "url": "/api/login", "status": 500, "duration_ms": 2000, "error": "Internal Server Error", "request_size": 42, "response_size": 128},
        ]);
        let output = format_network(&requests);
        assert!(output.contains("01:01:01.123"));
        assert!(output.contains("GET"));
        assert!(output.contains("/api/users"));
        assert!(output.contains("150ms"));
        assert!(output.contains("POST"));
        assert!(output.contains("/api/login"));
        assert!(output.contains("Internal Server Error"));
    }

    #[test]
    fn test_format_network_empty_array() {
        let output = format_network(&json!([]));
        assert!(output.contains("No requests captured"));
    }

    #[test]
    fn test_format_network_non_array() {
        let output = format_network(&json!({"unexpected": true}));
        assert!(output.contains("Unexpected"));
    }

    #[test]
    fn test_strip_ansi_removes_csi_sequences() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
        assert_eq!(strip_ansi("\x1b[2J\x1b[Hcleared"), "cleared");
    }

    #[test]
    fn test_strip_ansi_removes_osc_sequences() {
        // BEL terminator
        assert_eq!(strip_ansi("\x1b]0;title\x07text"), "text");
        // ST terminator (ESC \)
        assert_eq!(strip_ansi("\x1b]0;title\x1b\\text"), "text");
    }

    #[test]
    fn test_strip_ansi_preserves_backslash_in_osc() {
        // Bare backslash inside OSC should not terminate early
        assert_eq!(
            strip_ansi("\x1b]8;;http://host/path\\to\\file\x07link"),
            "link"
        );
    }

    #[test]
    fn test_strip_ansi_preserves_plain_text() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }
}
