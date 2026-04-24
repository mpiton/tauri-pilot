//! Window-list formatter.

use super::strip_ansi;

/// Format the list of open windows.
///
/// Expects `{"windows": [{"label": "main", "url": "http://...", "title": "My App"}]}`
pub fn format_windows(value: &serde_json::Value) {
    let Some(windows) = value.get("windows").and_then(|w| w.as_array()) else {
        println!("{}", crate::style::dim("No windows found"));
        return;
    };
    if windows.is_empty() {
        println!("{}", crate::style::dim("No windows found"));
        return;
    }

    let max_label = windows
        .iter()
        .map(|w| {
            strip_ansi(
                w.get("label")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            )
            .len()
        })
        .max()
        .unwrap_or(0);

    let max_url = windows
        .iter()
        .map(|w| {
            strip_ansi(
                w.get("url")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            )
            .len()
        })
        .max()
        .unwrap_or(0);

    for window in windows {
        let label = strip_ansi(
            window
                .get("label")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
        );
        let url = strip_ansi(
            window
                .get("url")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
        );
        let title = strip_ansi(
            window
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
        );
        let padded_label = format!("{label:<max_label$}");
        let padded_url = format!("{url:<max_url$}");
        println!(
            "{}   {}   {}",
            crate::style::bold(&padded_label),
            crate::style::dim(&padded_url),
            title,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_windows_empty() {
        format_windows(&json!({"windows": []}));
    }

    #[test]
    fn test_format_windows_single() {
        let value = json!({
            "windows": [
                {"label": "main", "url": "http://localhost:1420/", "title": "My Tauri App"}
            ]
        });
        format_windows(&value);
    }

    #[test]
    fn test_format_windows_multiple() {
        let value = json!({
            "windows": [
                {"label": "main", "url": "http://localhost:1420/", "title": "My Tauri App"},
                {"label": "settings", "url": "http://localhost:1420/settings", "title": "Settings"},
            ]
        });
        format_windows(&value);
    }

    #[test]
    fn test_format_windows_missing_fields() {
        let value = json!({
            "windows": [
                {"label": "main"},
                {"url": "http://localhost:1420/"},
                {},
            ]
        });
        format_windows(&value);
    }

    #[test]
    fn test_format_windows_no_windows_key() {
        format_windows(&json!({}));
    }
}
