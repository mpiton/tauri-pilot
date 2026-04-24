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
