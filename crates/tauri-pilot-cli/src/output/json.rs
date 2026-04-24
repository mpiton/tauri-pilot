//! JSON pretty-printer.

use anyhow::Result;

/// Print a value as pretty JSON.
///
/// # Errors
///
/// Returns an error if the value cannot be serialized to JSON (which in
/// practice never occurs for `serde_json::Value`, but is required by the
/// return type).
pub fn format_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_format_json_does_not_panic() {
        format_json(&json!({"status": "ok"})).expect("format_json succeeds");
    }
}
