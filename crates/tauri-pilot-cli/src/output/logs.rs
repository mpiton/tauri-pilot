//! Console-log formatter.

use std::fmt::Write;

use super::{format_timestamp, strip_ansi};

/// Format console log entries for human-readable display.
#[must_use]
pub fn format_logs(value: &serde_json::Value) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_logs_with_entries() {
        let logs = json!([
            {"id": 1, "timestamp": 3_661_123_u64, "level": "error", "args": ["fail"], "source": null},
            {"id": 2, "timestamp": 3_661_500_u64, "level": "info", "args": ["ok", 42], "source": null},
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
}
