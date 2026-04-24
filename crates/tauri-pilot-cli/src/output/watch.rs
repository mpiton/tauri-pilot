//! DOM-mutation watch formatter.

use std::fmt::Write;

use super::strip_ansi;

/// Format watch result showing DOM mutations grouped by type.
pub fn format_watch(value: &serde_json::Value) {
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
