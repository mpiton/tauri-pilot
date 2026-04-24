//! Network-request formatter.

use std::fmt::Write;

use super::{format_timestamp, strip_ansi};

/// Format network request entries for human-readable display.
#[must_use]
pub fn format_network(value: &serde_json::Value) -> String {
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
