//! Tests for the MCP tools registry and schema correctness.

use super::*;

/// Every tool registered in `tools()` has a unique name.
#[test]
fn test_tools_registry_has_unique_names() {
    let tools = tools();
    let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(
        total,
        names.len(),
        "duplicate tool names: total={total}, unique={}",
        names.len()
    );
}

/// All known tool names (canonical list at PR1 baseline). Must stay stable
/// across the upcoming Task 4–8 module split — additions/removals require
/// updating BOTH this list and the `tools()` registry in lock-step.
#[test]
fn test_tools_registry_contains_baseline_tools() {
    let tools = tools();
    let names: std::collections::HashSet<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

    let expected = [
        // core
        "ping",
        "windows",
        "state",
        "snapshot",
        "diff",
        "screenshot",
        "navigate",
        "url",
        "title",
        "wait",
        // interact
        "click",
        "fill",
        "type",
        "press",
        "select",
        "check",
        "scroll",
        "drag",
        "drop",
        // inspect
        "text",
        "html",
        "value",
        "attrs",
        // eval
        "eval",
        "ipc",
        // observe
        "watch",
        "logs",
        "network",
        // storage
        "storage_get",
        "storage_set",
        "storage_list",
        "storage_clear",
        "forms",
        // assert
        "assert_text",
        "assert_contains",
        "assert_visible",
        "assert_hidden",
        "assert_value",
        "assert_count",
        "assert_checked",
        "assert_url",
        // record
        "record_start",
        "record_stop",
        "record_status",
        "replay",
    ];

    for name in expected {
        assert!(
            names.contains(name),
            "expected tool '{name}' missing from registry"
        );
    }
}

/// Tool count matches baseline. If you add a tool, bump this number AND
/// add it to `test_tools_registry_contains_baseline_tools`.
#[test]
fn test_tools_registry_count_baseline() {
    let count = tools().len();
    // Snapshot of pre-split registry. If you intentionally add a tool, update.
    assert_eq!(count, 45, "tool count drifted from baseline");
}
