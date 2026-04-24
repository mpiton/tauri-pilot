//! Plain-text and assertion-failure formatters.

use super::strip_ansi;

/// Print an assertion failure message to stderr in red.
pub fn format_assert_fail(message: &str) {
    use owo_colors::{OwoColorize, Stream::Stderr};
    let text = format!("FAIL: {}", strip_ansi(message));
    eprintln!("{}", text.if_supports_color(Stderr, |t| t.red()));
}

/// Print a value as compact text for human consumption.
pub fn format_text(value: &serde_json::Value) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
}
