//! Recording and replay-step formatters.

use std::time::Duration;

use super::strip_ansi;
use super::text::format_text;

/// Format a record command result.
///
/// Handles three shapes:
/// - `{"status": "recording"}` → "✓ Recording started"
/// - `{"status": "saved", "path": "...", "count": N}` → "✓ Recording saved — N actions → path"
/// - `{"active": true/false, "count": N, "elapsed_ms": M}` → status line
#[must_use]
pub fn format_record(value: &serde_json::Value) -> String {
    if let Some(status) = value.get("status").and_then(serde_json::Value::as_str) {
        match status {
            "recording" => return crate::style::success("Recording started"),
            "saved" => {
                let path = value
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                let count = value
                    .get("count")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                return crate::style::success(&format!(
                    "Recording saved \u{2014} {count} actions \u{2192} {}",
                    strip_ansi(path)
                ));
            }
            _ => {}
        }
    }
    if let Some(active) = value.get("active").and_then(serde_json::Value::as_bool) {
        let count = value
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if active {
            let elapsed_ms = value
                .get("elapsed_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            // Precision below ~285 million years of ms is exact in f64; safe for display.
            let secs = Duration::from_millis(elapsed_ms).as_secs_f64();
            return format!(
                "{} Recording: {count} actions ({secs:.1}s)",
                crate::style::info("\u{25cf}")
            );
        }
        return format!("{} Not recording", crate::style::dim("\u{25cb}"));
    }
    // Fallback: format_text prints directly to stdout; return empty so the
    // caller (which only prints non-empty strings) does not double-print.
    format_text(value);
    String::new()
}

/// Format a single replay step.
///
/// Returns a string like "[3/10] click @e3 → ok" with color based on result.
#[must_use]
pub fn format_replay_step(step: usize, total: usize, action: &str, result: &str) -> String {
    let action_safe = strip_ansi(action);
    let result_display = if result == "ok" {
        crate::style::success(result)
    } else {
        crate::style::error(result)
    };
    format!(
        "{} {} \u{2192} {result_display}",
        crate::style::dim(format!("[{step}/{total}]")),
        crate::style::info(action_safe)
    )
}
