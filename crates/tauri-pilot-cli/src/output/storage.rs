//! Storage (localStorage / sessionStorage) formatters.

use super::strip_ansi;

/// Format a single storage value (from `storage get`).
pub fn format_storage_value(value: &serde_json::Value) {
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
pub fn format_storage(value: &serde_json::Value) {
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
