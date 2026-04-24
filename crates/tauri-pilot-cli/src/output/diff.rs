//! Snapshot-diff formatter.

use std::fmt::Write;

use super::strip_ansi;

/// Format a diff result showing added, removed, and changed elements.
pub fn format_diff(value: &serde_json::Value) {
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
                        format!("{} {} ", crate::style::warn("~"), crate::style::info(&role));
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
                    format!("{} {} ", crate::style::warn("~"), crate::style::info(&role));
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

    let mut line = format!("{} {} ", prefix_style(prefix), crate::style::info(role));
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
