//! Accessibility-tree snapshot formatter.

use std::fmt::Write;

/// Format a snapshot result as an indented accessibility tree.
pub fn format_snapshot(value: &serde_json::Value) {
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
