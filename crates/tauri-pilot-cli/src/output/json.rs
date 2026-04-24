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
