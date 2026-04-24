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
