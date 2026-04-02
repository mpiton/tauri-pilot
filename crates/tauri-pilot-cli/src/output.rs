use anyhow::Result;

/// Print a value as pretty JSON.
pub(crate) fn format_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Print a value as compact text for human consumption.
pub(crate) fn format_text(value: &serde_json::Value) {
    match value {
        serde_json::Value::String(s) => println!("{s}"),
        serde_json::Value::Null => {}
        other => println!("{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_json_does_not_panic() {
        format_json(&json!({"status": "ok"})).unwrap();
    }

    #[test]
    fn test_format_text_does_not_panic() {
        format_text(&json!("hello"));
        format_text(&json!(42));
        format_text(&json!(null));
    }
}
