//! HTML form dump formatter.

use std::fmt::Write;

use super::strip_ansi;

/// Format a single form field for display.
fn format_form_field(field: &serde_json::Value) {
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
    let checked = field.get("checked").and_then(serde_json::Value::as_bool);

    let mut line = format!("  {display_name}");
    if !field_type.is_empty() {
        let _ = write!(
            line,
            " {}",
            crate::style::dim(format!("[type={field_type}]"))
        );
    }
    if field_type == "password" {
        if field_value.is_empty() {
            let _ = write!(line, " = \"\"");
        } else {
            let _ = write!(line, " = {}", crate::style::dim("[redacted]"));
        }
    } else if let Some(is_checked) = checked {
        let _ = write!(line, " = \"{field_value}\"");
        if is_checked {
            let _ = write!(line, " {}", crate::style::success("checked"));
        } else {
            let _ = write!(line, " {}", crate::style::dim("unchecked"));
        }
    } else {
        let _ = write!(line, " = \"{field_value}\"");
    }
    println!("{line}");
}

/// Format form fields dumped from the page.
///
/// Expects `{forms: [{id, name, action, method, fields: [{tag, type, name, value, checked}]}]}`
pub fn format_forms(value: &serde_json::Value) {
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
                format_form_field(field);
            }
            if form
                .get("fieldsTruncated")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                println!(
                    "  {}",
                    crate::style::warn("(fields truncated — more fields exist)")
                );
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
