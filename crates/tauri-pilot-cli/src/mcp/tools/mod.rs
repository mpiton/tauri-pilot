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
mod schema_tests;
#[cfg(test)]
mod tests;
