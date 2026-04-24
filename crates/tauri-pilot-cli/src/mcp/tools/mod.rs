//! MCP tool registry. Each per-domain file returns its own `Vec<ToolSpec>`,
//! and `tools()` concatenates them, sorts by name, and converts to `rmcp::model::Tool`.

mod assert;
pub(super) mod core;
mod eval;
mod inspect;
mod interact;
mod observe;
mod record;
mod schemas;
mod storage;

use std::sync::{Arc, OnceLock};

use rmcp::model::{JsonObject, Tool, ToolAnnotations};

#[derive(Copy, Clone)]
pub(super) struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Arc<JsonObject>,
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
}

pub(super) fn tools() -> Vec<Tool> {
    cached_tools().clone()
}

pub(super) fn cached_tools() -> &'static Vec<Tool> {
    static TOOLS: OnceLock<Vec<Tool>> = OnceLock::new();
    TOOLS.get_or_init(build_tools)
}

fn build_tools() -> Vec<Tool> {
    let mut specs = Vec::new();
    specs.extend(assert::specs());
    specs.extend(core::specs());
    specs.extend(eval::specs());
    specs.extend(inspect::specs());
    specs.extend(interact::specs());
    specs.extend(observe::specs());
    specs.extend(record::specs());
    specs.extend(storage::specs());

    specs.sort_by_key(|spec| spec.name);
    specs.into_iter().map(spec_to_tool).collect()
}

fn spec_to_tool(spec: ToolSpec) -> Tool {
    Tool::new(spec.name, spec.description, (spec.schema)()).with_annotations(
        ToolAnnotations::new()
            .read_only(spec.read_only)
            .destructive(spec.destructive)
            .idempotent(spec.idempotent)
            .open_world(false),
    )
}

#[cfg(test)]
mod tests {
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
        let names: std::collections::HashSet<&str> =
            tools.iter().map(|t| t.name.as_ref()).collect();

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

    #[test]
    fn tool_list_is_sorted_alphabetically() {
        let tools = tools();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        let expected = vec![
            "assert_checked",
            "assert_contains",
            "assert_count",
            "assert_hidden",
            "assert_text",
            "assert_url",
            "assert_value",
            "assert_visible",
            "attrs",
            "check",
            "click",
            "diff",
            "drag",
            "drop",
            "eval",
            "fill",
            "forms",
            "html",
            "ipc",
            "logs",
            "navigate",
            "network",
            "ping",
            "press",
            "record_start",
            "record_status",
            "record_stop",
            "replay",
            "screenshot",
            "scroll",
            "select",
            "snapshot",
            "state",
            "storage_clear",
            "storage_get",
            "storage_list",
            "storage_set",
            "text",
            "title",
            "type",
            "url",
            "value",
            "wait",
            "watch",
            "windows",
        ];
        assert_eq!(names, expected);
    }

    #[test]
    fn schemas_include_window_override() {
        use super::super::schemas::target_schema;
        use serde_json::Value;

        let schema = target_schema();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema has properties");
        assert!(properties.contains_key("target"));
        assert!(properties.contains_key("window"));
    }

    #[test]
    fn windows_schema_omits_window_override() {
        use serde_json::Value;

        let schema = core::global_empty_schema();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema has properties");
        assert!(!properties.contains_key("window"));
    }
}
