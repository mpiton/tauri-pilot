use std::fmt::Write;

use anyhow::Result;

/// Print a value as pretty JSON.
pub(crate) fn format_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Print an assertion failure message to stderr in red.
pub(crate) fn format_assert_fail(message: &str) {
    use owo_colors::{OwoColorize, Stream::Stderr};
    let text = format!("FAIL: {}", strip_ansi(message));
    eprintln!("{}", text.if_supports_color(Stderr, |t| t.red()));
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

/// Format a single storage value (from `storage get`).
pub(crate) fn format_storage_value(value: &serde_json::Value) {
    let found = value
        .get("found")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !found {
        println!("{}", crate::style::dim("(not found)"));
        return;
    }
    if let Some(val) = value.get("value").and_then(serde_json::Value::as_str) {
        println!("{}", strip_ansi(val));
    } else {
        println!("{}", crate::style::dim("(not found)"));
    }
}

/// Format storage entries as key = value pairs.
///
/// Expects `{entries: [{key, value}, ...], truncated: bool}` from `storageList`.
pub(crate) fn format_storage(value: &serde_json::Value) {
    let Some(entries) = value.get("entries").and_then(|e| e.as_array()) else {
        println!("{}", crate::style::dim("(empty storage)"));
        return;
    };
    if entries.is_empty() {
        println!("{}", crate::style::dim("(empty storage)"));
        return;
    }
    for entry in entries {
        let key = entry.get("key").and_then(|k| k.as_str()).unwrap_or("?");
        let val = entry
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("null");
        let key_safe = strip_ansi(key);
        let val_safe = strip_ansi(val);
        println!(
            "{} {} {}",
            crate::style::bold(&key_safe),
            crate::style::dim("="),
            val_safe
        );
    }
    if value.get("truncated").and_then(serde_json::Value::as_bool) == Some(true) {
        println!(
            "{}",
            crate::style::warn("(output truncated — more entries exist)")
        );
    }
}

/// Format form fields dumped from the page.
///
/// Expects `{forms: [{id, name, action, method, fields: [{tag, type, name, value, checked}]}]}`
pub(crate) fn format_forms(value: &serde_json::Value) {
    let Some(forms) = value.get("forms").and_then(|f| f.as_array()) else {
        println!("{}", crate::style::dim("(no forms found)"));
        return;
    };
    if forms.is_empty() {
        println!("{}", crate::style::dim("(no forms found)"));
        return;
    }
    for form in forms {
        let id = form
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let name = form
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let mut header = String::from("form");
        if !id.is_empty() {
            let _ = write!(header, "#{}", strip_ansi(id));
        } else if !name.is_empty() {
            let _ = write!(header, "[name=\"{}\"]", strip_ansi(name));
        }
        header.push(':');
        println!("{}", crate::style::bold(&header));

        if let Some(fields) = form.get("fields").and_then(|f| f.as_array()) {
            for field in fields {
                let field_name = strip_ansi(
                    field
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(""),
                );
                let display_name = if field_name.is_empty() {
                    "(unnamed)".to_owned()
                } else {
                    field_name
                };
                let field_type = strip_ansi(
                    field
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(""),
                );
                let field_value_raw = field.get("value");
                let field_value = match field_value_raw {
                    Some(serde_json::Value::Array(arr)) => arr
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(strip_ansi)
                        .collect::<Vec<_>>()
                        .join(", "),
                    _ => strip_ansi(
                        field_value_raw
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(""),
                    ),
                };
                let checked = field
                    .get("checked")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);

                let mut line = format!("  {display_name}");
                if !field_type.is_empty() {
                    let _ = write!(
                        line,
                        " {}",
                        crate::style::dim(format!("[type={field_type}]"))
                    );
                }
                if field_type == "checkbox" || field_type == "radio" {
                    if checked {
                        let _ = write!(line, " = checked");
                    } else {
                        let _ = write!(line, " = {}", crate::style::dim("unchecked"));
                    }
                } else if field_type == "password" {
                    if field_value.is_empty() {
                        let _ = write!(line, " = \"\"");
                    } else {
                        let _ = write!(line, " = {}", crate::style::dim("[redacted]"));
                    }
                } else {
                    let _ = write!(line, " = \"{field_value}\"");
                }
                println!("{line}");
            }
        }
    }
    if value.get("truncated").and_then(serde_json::Value::as_bool) == Some(true) {
        println!(
            "{}",
            crate::style::warn("(output truncated — more forms exist)")
        );
    }
}

/// Format watch result showing DOM mutations grouped by type.
pub(crate) fn format_watch(value: &serde_json::Value) {
    let added = value.get("added").and_then(|v| v.as_array());
    let removed = value.get("removed").and_then(|v| v.as_array());
    let modified = value.get("modified").and_then(|v| v.as_array());

    let is_empty = added.is_none_or(Vec::is_empty)
        && removed.is_none_or(Vec::is_empty)
        && modified.is_none_or(Vec::is_empty);

    if is_empty {
        println!("{}", crate::style::dim("No DOM changes detected."));
        return;
    }

    if let Some(entries) = added
        && !entries.is_empty()
    {
        println!("{}", crate::style::success("Added:"));
        for el in entries {
            println!("  {}", format_mutation_entry(el));
        }
    }

    if let Some(entries) = removed
        && !entries.is_empty()
    {
        println!("{}", crate::style::error("Removed:"));
        for el in entries {
            println!("  {}", format_mutation_entry(el));
        }
    }

    if let Some(entries) = modified
        && !entries.is_empty()
    {
        println!("{}", crate::style::warn("Modified:"));
        for el in entries {
            let tag = strip_ansi(el.get("tag").and_then(|t| t.as_str()).unwrap_or("?"));
            if let Some(attr) = el.get("attribute").and_then(|a| a.as_str()) {
                let attr_safe = strip_ansi(attr);
                let is_removed = el
                    .get("removed")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if is_removed {
                    println!(
                        "  {} {} {}",
                        crate::style::info(&tag),
                        attr_safe,
                        crate::style::dim("<removed>")
                    );
                } else {
                    let val = strip_ansi(el.get("value").and_then(|v| v.as_str()).unwrap_or(""));
                    println!(
                        "  {} {} = {}",
                        crate::style::info(&tag),
                        attr_safe,
                        crate::style::dim(format!("\"{val}\""))
                    );
                }
            } else if let Some(text) = el.get("text").and_then(|t| t.as_str()) {
                let text_safe = strip_ansi(text);
                println!(
                    "  {} {}",
                    crate::style::info(&tag),
                    crate::style::dim(format!("\"{text_safe}\""))
                );
            } else {
                println!(
                    "  {} {}",
                    crate::style::info(&tag),
                    crate::style::dim("(unknown change)")
                );
            }
        }
    }

    if value
        .get("truncated")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        println!(
            "{}",
            crate::style::warn("(some entries truncated — mutation buffer limit reached)")
        );
    }
}

fn format_mutation_entry(el: &serde_json::Value) -> String {
    let tag = strip_ansi(el.get("tag").and_then(|t| t.as_str()).unwrap_or("?"));
    let mut desc = crate::style::info(&tag);
    if let Some(id) = el.get("id").and_then(|i| i.as_str()) {
        let id_safe = strip_ansi(id);
        let _ = write!(desc, "#{id_safe}");
    }
    if let Some(class) = el.get("class").and_then(|c| c.as_str()) {
        let class_safe = strip_ansi(class);
        let first = class_safe.split_whitespace().next().unwrap_or("");
        let _ = write!(desc, ".{first}");
    }
    if let Some(text) = el.get("text").and_then(|t| t.as_str()) {
        let text_safe = strip_ansi(text);
        let _ = write!(desc, " {}", crate::style::dim(format!("\"{text_safe}\"")));
    }
    desc
}

/// Format a diff result showing added, removed, and changed elements.
pub(crate) fn format_diff(value: &serde_json::Value) {
    let added = value.get("added").and_then(|v| v.as_array());
    let removed = value.get("removed").and_then(|v| v.as_array());
    let changed_entries = value.get("changed").and_then(|v| v.as_array());

    let is_empty = added.is_none_or(Vec::is_empty)
        && removed.is_none_or(Vec::is_empty)
        && changed_entries.is_none_or(Vec::is_empty);

    if is_empty {
        println!("{}", crate::style::dim("No changes detected."));
        return;
    }

    if let Some(entries) = removed {
        for el in entries {
            println!("{}", format_diff_entry("-", el, &crate::style::error));
        }
    }

    if let Some(entries) = added {
        for el in entries {
            println!("{}", format_diff_entry("+", el, &crate::style::success));
        }
    }

    if let Some(entries) = changed_entries {
        for entry in entries {
            let el = entry.get("new").unwrap_or(entry);
            let role = strip_ansi(
                el.get("role")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?"),
            );
            let r#ref = strip_ansi(
                el.get("ref")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?"),
            );
            let name = el
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(strip_ansi);

            let old = entry.get("old");
            let field_changes = entry.get("changes").and_then(|c| c.as_array());

            if let Some(fields) = field_changes {
                for field in fields {
                    let field_name = field.as_str().unwrap_or("?");
                    let old_val = strip_ansi(
                        &old.and_then(|o| o.get(field_name))
                            .map(|v| match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            })
                            .unwrap_or_default(),
                    );
                    let new_val = strip_ansi(
                        &el.get(field_name)
                            .map(|v| match v {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            })
                            .unwrap_or_default(),
                    );

                    let mut line =
                        format!("{} {} ", crate::style::warn("~"), crate::style::info(&role),);
                    if let Some(ref n) = name {
                        let _ = write!(line, "{} ", crate::style::bold(format!("\"{n}\"")));
                    }
                    let _ = write!(
                        line,
                        "{} {}: {} \u{2192} {}",
                        crate::style::dim(format!("[ref={ref}]")),
                        field_name,
                        crate::style::dim(format!("\"{old_val}\"")),
                        crate::style::dim(format!("\"{new_val}\"")),
                    );
                    println!("{line}");
                }
            } else {
                let mut line =
                    format!("{} {} ", crate::style::warn("~"), crate::style::info(&role),);
                if let Some(ref n) = name {
                    let _ = write!(line, "{} ", crate::style::bold(format!("\"{n}\"")));
                }
                let _ = write!(line, "{}", crate::style::dim(format!("[ref={ref}]")));
                println!("{line}");
            }
        }
    }
}

fn format_diff_entry(
    prefix: &str,
    el: &serde_json::Value,
    prefix_style: &dyn Fn(&str) -> String,
) -> String {
    let role = el
        .get("role")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?");
    let r#ref = el
        .get("ref")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?");
    let name = el.get("name").and_then(serde_json::Value::as_str);

    let mut line = format!("{} {} ", prefix_style(prefix), crate::style::info(role),);
    if let Some(n) = name {
        let _ = write!(line, "{} ", crate::style::bold(format!("\"{n}\"")));
    }
    let _ = write!(line, "{}", crate::style::dim(format!("[ref={ref}]")));

    if let Some(val) = el.get("value").and_then(serde_json::Value::as_str) {
        let _ = write!(line, " {}", crate::style::dim(format!("value=\"{val}\"")));
    }
    if el.get("checked").and_then(serde_json::Value::as_bool) == Some(true) {
        let _ = write!(line, " {}", crate::style::dim("checked"));
    }
    if el.get("disabled").and_then(serde_json::Value::as_bool) == Some(true) {
        let _ = write!(line, " {}", crate::style::dim("disabled"));
    }

    line
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

    #[test]
    fn test_format_storage_empty_array() {
        // Should not panic and display "(empty storage)"
        format_storage(&json!({"entries": [], "truncated": false}));
    }

    #[test]
    fn test_format_storage_with_entries() {
        // Should not panic and display key = value pairs
        format_storage(&json!({"entries": [
            {"key": "auth_token", "value": "abc123"},
            {"key": "theme", "value": "dark"},
        ], "truncated": false}));
    }

    #[test]
    fn test_format_storage_non_object() {
        // Non-object input should not panic
        format_storage(&json!(null));
    }

    #[test]
    fn test_format_storage_value_found() {
        format_storage_value(&json!({"found": true, "value": "abc123"}));
    }

    #[test]
    fn test_format_storage_value_not_found() {
        format_storage_value(&json!({"found": false}));
    }

    #[test]
    fn test_format_storage_strips_ansi() {
        // Key and value with ANSI escape sequences should be stripped
        format_storage(&json!({"entries": [
            {"key": "\x1b[31mmalicious\x1b[0m", "value": "\x1b[2J\x1b[Hinjected"},
        ], "truncated": false}));
    }

    #[test]
    fn test_format_storage_truncated() {
        // Should not panic and display truncation warning
        format_storage(&json!({"entries": [
            {"key": "a", "value": "1"},
        ], "truncated": true}));
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
}
