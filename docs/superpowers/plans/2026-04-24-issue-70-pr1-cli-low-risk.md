# Phase C — PR1 (CLI low-risk) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first stacked PR of issue #70: rename scenario types (kill 4 allows = 5 lint instances), split `output.rs` (1172 lines) and `mcp.rs` (1815 lines) into per-domain modules under the 150-line cap, kill 2 more allows in mcp.

**Architecture:** Pure refactor on branch `chore/phase-c-1-cli-low-risk`. No behavior change. TDD discipline: char tests RED first wherever logic branches (`mcp` dispatch, `output` formatters), pure mechanical splits rely on existing 241-test suite + `cargo build`. Each task ends with `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check` green.

**Tech Stack:** Rust 2024, tokio, clap 4, anyhow, serde, owo_colors, rmcp.

**Spec:** `docs/superpowers/specs/2026-04-24-issue-70-phase-c-design.md`

**Branch state:** Already on `chore/phase-c-1-cli-low-risk` (off `main`), spec doc committed at `c2b07d7`.

**Note on file count vs spec:** Spec sketches `output/{json,text,table}.rs` (3 files) and `mcp/{tools,handlers}/{assert,record,dom,capture}.rs` (4+4 files). Real splits in this plan use **more granular** files because the 150-line cap applied to `output.rs` (1172 lines) forces ~10 files, and `mcp.rs` (1815 lines) requires finer splits than 4 prefix groups (some prefixes exceed 150 lines on their own). This is a refinement of the spec, not a deviation — the cap is the binding constraint.

---

## File Structure (PR1 target)

**Created:**
- `crates/tauri-pilot-cli/src/output/mod.rs` — re-exports + module decls
- `crates/tauri-pilot-cli/src/output/json.rs` — `format_json`
- `crates/tauri-pilot-cli/src/output/text.rs` — `format_text`, `format_assert_fail`
- `crates/tauri-pilot-cli/src/output/snapshot.rs` — `format_snapshot`
- `crates/tauri-pilot-cli/src/output/logs.rs` — `format_logs`, `format_timestamp`, `strip_ansi`
- `crates/tauri-pilot-cli/src/output/network.rs` — `format_network`
- `crates/tauri-pilot-cli/src/output/storage.rs` — `format_storage`, `format_storage_value`
- `crates/tauri-pilot-cli/src/output/forms.rs` — `format_forms`, `format_form_field`
- `crates/tauri-pilot-cli/src/output/watch.rs` — `format_watch`, `format_mutation_entry`
- `crates/tauri-pilot-cli/src/output/diff.rs` — `format_diff`, `format_diff_entry`
- `crates/tauri-pilot-cli/src/output/windows.rs` — `format_windows`
- `crates/tauri-pilot-cli/src/output/record.rs` — `format_record`, `format_replay_step`
- `crates/tauri-pilot-cli/src/mcp/mod.rs` — re-exports + `run_mcp_server`
- `crates/tauri-pilot-cli/src/mcp/server.rs` — `PilotMcpServer` struct + ctor + `connect_client` + `call_app` + `call_app_tool` + `ServerHandler` impl
- `crates/tauri-pilot-cli/src/mcp/banner.rs` — `print_startup_banner`, `startup_banner`
- `crates/tauri-pilot-cli/src/mcp/args.rs` — `required_string`, `optional_string`, `required_u64`, `optional_u64`, `optional_usize`, `optional_i32`, `optional_u8`, `optional_bool`, helper extractors
- `crates/tauri-pilot-cli/src/mcp/responses.rs` — `tool_success`, `tool_error`, `tool_error_str`, `mcp_error`
- `crates/tauri-pilot-cli/src/mcp/schemas.rs` — schema builders (`schema_object`, `properties`, `string_prop`, `number_prop`, `bool_prop`, `array_prop`, `enum_prop`, etc.)
- `crates/tauri-pilot-cli/src/mcp/tools/mod.rs` — `tools()` registry (combines per-domain lists)
- `crates/tauri-pilot-cli/src/mcp/tools/core.rs` — ping, windows, state, snapshot, diff, screenshot, navigate, url, title, wait
- `crates/tauri-pilot-cli/src/mcp/tools/interact.rs` — click, fill, type, press, select, check, scroll, drag, drop
- `crates/tauri-pilot-cli/src/mcp/tools/inspect.rs` — text, html, value, attrs
- `crates/tauri-pilot-cli/src/mcp/tools/eval.rs` — eval, ipc
- `crates/tauri-pilot-cli/src/mcp/tools/observe.rs` — watch, logs, network
- `crates/tauri-pilot-cli/src/mcp/tools/storage.rs` — storage_get, storage_set, storage_list, storage_clear, forms
- `crates/tauri-pilot-cli/src/mcp/tools/assert.rs` — assert_*
- `crates/tauri-pilot-cli/src/mcp/tools/record.rs` — record_start, record_stop, record_status, replay
- `crates/tauri-pilot-cli/src/mcp/handlers/mod.rs` — `call_tool_by_name` prefix dispatcher
- `crates/tauri-pilot-cli/src/mcp/handlers/core.rs` — handlers for core tools
- `crates/tauri-pilot-cli/src/mcp/handlers/interact.rs` — handlers for interact tools
- `crates/tauri-pilot-cli/src/mcp/handlers/inspect.rs` — handlers for inspect tools
- `crates/tauri-pilot-cli/src/mcp/handlers/eval.rs` — handlers for eval/ipc
- `crates/tauri-pilot-cli/src/mcp/handlers/observe.rs` — handlers for watch/logs/network
- `crates/tauri-pilot-cli/src/mcp/handlers/storage.rs` — handlers for storage/forms
- `crates/tauri-pilot-cli/src/mcp/handlers/assert.rs` — `assert_text`, `assert_bool`, `assert_value`, `assert_count`, `assert_url` impls
- `crates/tauri-pilot-cli/src/mcp/handlers/record.rs` — record/replay handlers + `call_drop_tool`, `call_replay_tool`
- `crates/tauri-pilot-cli/tests/output_smoke.rs` — char tests for output formatters
- `crates/tauri-pilot-cli/tests/mcp_dispatch.rs` — char tests for MCP dispatch

**Deleted:**
- `crates/tauri-pilot-cli/src/output.rs`
- `crates/tauri-pilot-cli/src/mcp.rs`

**Modified:**
- `crates/tauri-pilot-cli/src/scenario.rs` — type renames + field rename (no file split in PR1)
- `crates/tauri-pilot-cli/src/main.rs` — update `mod output;` + `mod mcp;` declarations remain unchanged (modules now folders); `scenario` type imports if any
- `CHANGELOG.md` — add entry under `[Unreleased]` → `Changed`

---

## Task 0: Verify branch state

- [ ] **Step 0.1: Confirm branch + clean working tree**

Run: `git rev-parse --abbrev-ref HEAD && git status --short`
Expected: `chore/phase-c-1-cli-low-risk` and empty status (only spec doc committed).

- [ ] **Step 0.2: Confirm baseline green**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: tests pass (~241), clippy clean.

- [ ] **Step 0.3: Snapshot baseline allow count + oversized files**

Run:
```bash
echo "=== allows ===" && grep -rn "#\[allow" crates/ | wc -l
echo "=== oversized ===" && find crates -name "*.rs" -not -path "*/target/*" | xargs wc -l | awk '$1 > 150 { print }'
```
Expected: 12 allows, 15 oversized files. Record numbers for end-of-PR comparison.

---

## Task 1: Char tests for output formatters (RED)

These tests pin current output behavior so the split cannot regress.

**Files:**
- Create: `crates/tauri-pilot-cli/tests/output_smoke.rs`

- [ ] **Step 1.1: Write the failing tests**

Create `crates/tauri-pilot-cli/tests/output_smoke.rs`:

```rust
//! Characterization tests for `tauri_pilot_cli::output` — pin current behavior
//! before splitting `output.rs` into per-domain modules (issue #70 PR1).
//!
//! Only String-returning formatters are tested. Void formatters
//! (`format_text`, `format_snapshot`, `format_diff`, etc.) print directly
//! to stdout — capturing real stdout from a test requires unsafe FD
//! manipulation or external crates, which is out of scope for PR1. They
//! are exercised indirectly by integration smoke tests against a real
//! sample app.

use serde_json::json;
use tauri_pilot_cli::output;

#[test]
fn test_format_logs_with_one_entry_contains_message() {
    let payload = json!({
        "logs": [
            { "timestamp": 1_700_000_000_000_u64, "level": "info", "message": "hello" }
        ]
    });
    let rendered = output::format_logs(&payload);
    assert!(
        rendered.contains("hello"),
        "expected 'hello' in rendered logs, got: {rendered}"
    );
}

#[test]
fn test_format_logs_with_empty_logs_returns_string() {
    let payload = json!({ "logs": [] });
    // Just verify it does not panic and returns SOMETHING (may be empty).
    let _rendered = output::format_logs(&payload);
}

#[test]
fn test_format_record_with_one_event_returns_non_empty() {
    let payload = json!({
        "events": [
            { "type": "click", "selector": "#btn" }
        ]
    });
    let rendered = output::format_record(&payload);
    assert!(!rendered.is_empty(), "expected non-empty record output");
}

#[test]
fn test_format_replay_step_returns_non_empty() {
    let rendered = output::format_replay_step(1, 3, "click", "ok");
    assert!(rendered.contains("click"), "expected 'click' in: {rendered}");
    assert!(rendered.contains("ok"), "expected 'ok' in: {rendered}");
}

#[test]
fn test_format_network_with_empty_array_does_not_panic() {
    let payload = json!({ "requests": [] });
    let _rendered = output::format_network(&payload);
}
```

No new dev-dependencies required.

Also expose the `output` module so the integration test can reach it:

Modify `crates/tauri-pilot-cli/src/main.rs` (top of file, near `mod output;`):
```rust
// Already declared in main.rs — must also be reachable from integration tests.
// Add a thin lib.rs alongside main.rs that re-exports the modules under test.
```

Create `crates/tauri-pilot-cli/src/lib.rs`:
```rust
//! Library facade for integration tests.
//!
//! Binary code lives in `main.rs`; this file re-exports the modules used by
//! the integration test suite under `tests/`. Keeping the binary entry point
//! and the test surface separate avoids having to duplicate code.

pub mod output;
pub mod mcp;
pub mod scenario;
pub mod protocol;
pub mod style;
pub mod client;
pub mod error;
```

Modify `crates/tauri-pilot-cli/Cargo.toml` to add a `[lib]` section if not present:
```toml
[lib]
name = "tauri_pilot_cli"
path = "src/lib.rs"
```

> **Note:** if a `[lib]` already exists, leave it; this step only adds it if missing. Run `grep -n "^\[lib\]" crates/tauri-pilot-cli/Cargo.toml` before editing.

- [ ] **Step 1.2: Run test, verify it fails (compiles or fails on assertion)**

Run: `cargo test --package tauri-pilot-cli --test output_smoke -v`
Expected outcomes:
- If `lib.rs` was newly created: tests compile and pass (current behavior pinned).
- If something is wrong with `format_*` exposure: compile error pointing to missing `pub`. Fix by changing `pub(crate) fn` → `pub fn` for the formatters tested (`format_text`, `format_snapshot`, `format_logs`, `format_record`, `format_diff`) **only inside `output.rs`**.

If tests pass: that's the GREEN baseline — proceed.
If tests fail with assertion errors: adjust assertions to match actual current output (do NOT modify `output.rs` behavior).

- [ ] **Step 1.3: Commit RED → GREEN baseline**

Run:
```bash
git add crates/tauri-pilot-cli/Cargo.toml crates/tauri-pilot-cli/src/lib.rs crates/tauri-pilot-cli/src/output.rs crates/tauri-pilot-cli/tests/output_smoke.rs
git commit -m "test(cli): pin output formatter behavior before split (issue #70)"
```

---

## Task 2: Split `output.rs` into per-domain modules

**Files:**
- Delete: `crates/tauri-pilot-cli/src/output.rs`
- Create: 12 files under `crates/tauri-pilot-cli/src/output/`

- [ ] **Step 2.1: Read current `output.rs` to know what to move**

Run: `wc -l crates/tauri-pilot-cli/src/output.rs && head -20 crates/tauri-pilot-cli/src/output.rs`
Expected: 1172 lines, file starts with imports + `format_json`.

- [ ] **Step 2.2: Create the directory and `mod.rs`**

Create `crates/tauri-pilot-cli/src/output/mod.rs`:
```rust
//! Per-format / per-domain output renderers for the `tauri-pilot` CLI.
//!
//! Split out of the original `output.rs` (1172 lines, issue #70) by domain
//! to keep each file under the 150-line cap defined in `CLAUDE.md`.

mod diff;
mod forms;
mod json;
mod logs;
mod network;
mod record;
mod snapshot;
mod storage;
mod text;
mod watch;
mod windows;

pub use diff::format_diff;
pub use forms::format_forms;
pub use json::format_json;
pub use logs::format_logs;
pub use network::format_network;
pub use record::{format_record, format_replay_step};
pub use snapshot::format_snapshot;
pub use storage::format_storage;
pub use text::{format_assert_fail, format_text};
pub use watch::format_watch;
pub use windows::format_windows;
```

- [ ] **Step 2.3: Create `output/json.rs`**

Move `format_json` from `output.rs` (lines 6–10) into `crates/tauri-pilot-cli/src/output/json.rs`:
```rust
use anyhow::Result;

/// Print a value as pretty JSON.
pub fn format_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
```

- [ ] **Step 2.4: Create `output/text.rs`**

Move `format_text` (lines 20–56) and `format_assert_fail` (lines 13–17) from `output.rs` into `crates/tauri-pilot-cli/src/output/text.rs`. Keep imports the file needs (`owo_colors`, `crate::style`). Visibility: `pub fn` (was `pub(crate) fn`).

- [ ] **Step 2.5: Create `output/snapshot.rs`**

Move `format_snapshot` (lines 59–~108) into `crates/tauri-pilot-cli/src/output/snapshot.rs`. Visibility: `pub fn`.

- [ ] **Step 2.6: Create `output/logs.rs`**

Move `format_logs` (line 120), `format_timestamp` (line 110, private), and `strip_ansi` (line 775, private — used by `format_assert_fail` AND `format_logs`?). Verify usage:
Run: `rtk proxy grep -n "strip_ansi\|format_timestamp" crates/tauri-pilot-cli/src/output.rs`

If `strip_ansi` is used by both `text.rs` and `logs.rs`, place it in `output/mod.rs` as `pub(super) fn strip_ansi(...)` so both can `use super::strip_ansi`. Same for `format_timestamp` if cross-used.

If `strip_ansi` is only used by `text.rs` (line 16), keep it `fn strip_ansi` private inside `text.rs`.

Either way, `format_logs` goes into `output/logs.rs` as `pub fn format_logs(...)`.

- [ ] **Step 2.7: Create `output/network.rs`**

Move `format_network` (line 168) into `crates/tauri-pilot-cli/src/output/network.rs`. Visibility: `pub fn`.

- [ ] **Step 2.8: Create `output/storage.rs`**

Move `format_storage_value` (line 229, private) and `format_storage` (line 248) into `crates/tauri-pilot-cli/src/output/storage.rs`. Keep `format_storage_value` as `fn` (private). Visibility of `format_storage`: `pub fn`.

- [ ] **Step 2.9: Create `output/forms.rs`**

Move `format_form_field` (line 281, private) and `format_forms` (line 345) into `crates/tauri-pilot-cli/src/output/forms.rs`. Visibility of `format_forms`: `pub fn`.

- [ ] **Step 2.10: Create `output/watch.rs`**

Move `format_watch` (line 397) and `format_mutation_entry` (line 486, private) into `crates/tauri-pilot-cli/src/output/watch.rs`. Visibility: `pub fn` for `format_watch`.

- [ ] **Step 2.11: Create `output/diff.rs`**

Move `format_diff` (line 506) and `format_diff_entry` (line 601, private) into `crates/tauri-pilot-cli/src/output/diff.rs`. Visibility: `pub fn` for `format_diff`.

- [ ] **Step 2.12: Create `output/windows.rs`**

Move `format_windows` (line 638) into `crates/tauri-pilot-cli/src/output/windows.rs`. Visibility: `pub fn`.

- [ ] **Step 2.13: Create `output/record.rs`**

Move `format_record` (line 710) and `format_replay_step` (line 759) into `crates/tauri-pilot-cli/src/output/record.rs`. Visibility: `pub fn` for both.

- [ ] **Step 2.14: Delete `output.rs`**

Run: `git rm crates/tauri-pilot-cli/src/output.rs`

- [ ] **Step 2.15: Update `lib.rs` if needed**

`pub mod output;` already declared in Step 1.1; no change needed (Cargo treats `output.rs` and `output/mod.rs` interchangeably).

- [ ] **Step 2.16: Compile + verify cap**

Run:
```bash
cargo build --workspace 2>&1 | tail -40
find crates/tauri-pilot-cli/src/output -name "*.rs" | xargs wc -l | sort -rn
```
Expected: build succeeds, every file in `output/` ≤ 150 lines.

If any file exceeds 150 lines, split further (e.g., `output/diff.rs` may need `output/diff/{mod,renderer,formatter}.rs`).

If compile errors: missing imports — copy the relevant `use` statements from old `output.rs` to each new file. Each new file owns only the imports it actually needs.

- [ ] **Step 2.17: Run output char tests + full clippy**

Run:
```bash
cargo test --package tauri-pilot-cli --test output_smoke
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```
Expected: all green.

- [ ] **Step 2.18: Commit**

```bash
git add -A crates/tauri-pilot-cli/src/output crates/tauri-pilot-cli/src/lib.rs
git rm --cached crates/tauri-pilot-cli/src/output.rs 2>/dev/null || true
git commit -m "refactor(cli): split output.rs (1172 lines) into output/ modules

Split per-domain (json, text, snapshot, logs, network, storage, forms,
watch, diff, windows, record) to keep each file under the 150-line cap
defined in CLAUDE.md. Behavior preserved — all existing tests pass plus
new char tests in tests/output_smoke.rs (issue #70)."
```

---

## Task 3: Char tests for MCP dispatch (RED)

Pin the current dispatch contract: every tool name routes to its expected backend method.

**Files:**
- Create: `crates/tauri-pilot-cli/tests/mcp_dispatch.rs`

- [ ] **Step 3.1: Write the failing tests**

Create `crates/tauri-pilot-cli/tests/mcp_dispatch.rs`:

```rust
//! Characterization tests for the MCP `call_tool_by_name` dispatcher.
//!
//! These pin the tool-name → backend-method mapping so the upcoming module
//! split (issue #70) cannot silently drop or rename a tool.

use tauri_pilot_cli::mcp;

/// Every tool registered in `mcp::tools()` has a unique name.
#[test]
fn test_tools_registry_has_unique_names() {
    let tools = mcp::tools();
    let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort();
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
/// across the split — additions/removals require updating BOTH this list and
/// the `tools()` registry in lock-step.
#[test]
fn test_tools_registry_contains_baseline_tools() {
    let tools = mcp::tools();
    let names: std::collections::HashSet<&str> =
        tools.iter().map(|t| t.name.as_ref()).collect();

    let expected = [
        // core
        "ping", "windows", "state", "snapshot", "diff", "screenshot",
        "navigate", "url", "title", "wait",
        // interact
        "click", "fill", "type", "press", "select", "check", "scroll",
        "drag", "drop",
        // inspect
        "text", "html", "value", "attrs",
        // eval
        "eval", "ipc",
        // observe
        "watch", "logs", "network",
        // storage
        "storage_get", "storage_set", "storage_list", "storage_clear", "forms",
        // assert
        "assert_text", "assert_contains", "assert_visible", "assert_hidden",
        "assert_value", "assert_count", "assert_checked", "assert_url",
        // record
        "record_start", "record_stop", "record_status", "replay",
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
    let count = mcp::tools().len();
    // Snapshot of pre-split registry. If you intentionally add a tool, update.
    assert_eq!(count, 44, "tool count drifted from baseline");
}
```

- [ ] **Step 3.2: Verify `mcp::tools()` is callable from tests**

Currently `tools()` is `fn tools() -> Vec<Tool>` inside `mcp.rs` (line ~648), declared without a `pub`. Make it `pub fn tools()` in `mcp.rs` so the test crate can reach it. Same for any helper the test needs.

Run: `rtk proxy grep -n "fn tools" crates/tauri-pilot-cli/src/mcp.rs`
Edit `mcp.rs` line 648: `fn tools()` → `pub fn tools()`.

- [ ] **Step 3.3: Run, fix the count assertion**

Run: `cargo test --package tauri-pilot-cli --test mcp_dispatch -v`

Expected first run: `test_tools_registry_count_baseline` may fail with the actual count. Update the `44` literal to the actual count printed in the failure. Re-run; all 3 tests should pass.

- [ ] **Step 3.4: Commit**

```bash
git add crates/tauri-pilot-cli/src/mcp.rs crates/tauri-pilot-cli/tests/mcp_dispatch.rs
git commit -m "test(cli): pin MCP tools registry baseline before split (issue #70)"
```

---

## Task 4: Extract MCP helper modules (no behavior change)

Pull out the obvious leaf utilities first — these have zero dispatch logic and shrink `mcp.rs` quickly.

**Files:**
- Create: `crates/tauri-pilot-cli/src/mcp/mod.rs`
- Create: `crates/tauri-pilot-cli/src/mcp/banner.rs`
- Create: `crates/tauri-pilot-cli/src/mcp/args.rs`
- Create: `crates/tauri-pilot-cli/src/mcp/responses.rs`
- Create: `crates/tauri-pilot-cli/src/mcp/schemas.rs`

> **Critical ordering note:** Cargo errors with "file found for module `mcp` at both …/mcp.rs and …/mcp/mod.rs" if both coexist. The migration must rename `mcp.rs` to `mcp/_legacy.rs` BEFORE creating `mcp/mod.rs`. The plan below does this in Step 4.1. After each subsequent step, the build must compile (helpers are drained progressively from `_legacy.rs`).

- [ ] **Step 4.1: Rename `mcp.rs` → `mcp/_legacy.rs` (creates dir, moves file in one go)**

Run:
```bash
mkdir -p crates/tauri-pilot-cli/src/mcp
git mv crates/tauri-pilot-cli/src/mcp.rs crates/tauri-pilot-cli/src/mcp/_legacy.rs
```

- [ ] **Step 4.2: Create `mcp/mod.rs` with the `_legacy` shim**

Create `crates/tauri-pilot-cli/src/mcp/mod.rs`:
```rust
//! MCP server for `tauri-pilot`. Split out of the original `mcp.rs`
//! (1815 lines, issue #70) by responsibility: server lifecycle, banner,
//! argument extractors, JSON-RPC response helpers, JSON schema builders,
//! per-domain tool registries, per-domain handlers.
//!
//! `_legacy` is a TEMPORARY container that shrinks step-by-step as
//! helpers are extracted into sibling modules. It will be deleted when
//! empty (Task 8).

mod _legacy;

pub use _legacy::{run_mcp_server, tools};
```

Run: `cargo build --workspace 2>&1 | tail -10`
Expected: build succeeds (functionally identical to before — only the file moved).

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all green.

- [ ] **Step 4.3: Extract `mcp/banner.rs`**

Create `crates/tauri-pilot-cli/src/mcp/banner.rs` containing `print_startup_banner` and `startup_banner` (lines 46–71 of `_legacy.rs`). Visibility: `pub(super) fn`. Imports needed: `use std::{io::IsTerminal, path::Path};`.

In `mcp/mod.rs`, add `mod banner;` (no re-export — internal use only).

In `_legacy.rs`, delete the two function definitions and add at the top: `use super::banner::print_startup_banner;`. Drop `startup_banner` references from `_legacy.rs` (it's only called by `print_startup_banner`).

Run: `cargo build --workspace && cargo test --workspace`
Expected: green.

- [ ] **Step 4.4: Extract `mcp/responses.rs`**

Create `crates/tauri-pilot-cli/src/mcp/responses.rs` containing `tool_success`, `tool_error`, `tool_error_str`, `mcp_error` (lines 1040–1057 of `_legacy.rs`). Visibility: `pub(super) fn`. Imports: `use rmcp::{ErrorData as McpError, model::{CallToolResult, ErrorCode}}; use serde_json::Value;`.

In `mcp/mod.rs`, add `mod responses;`.

In `_legacy.rs`, delete the four function definitions and add: `use super::responses::{tool_success, tool_error, tool_error_str, mcp_error};`.

Run: `cargo build --workspace && cargo test --workspace`
Expected: green.

- [ ] **Step 4.5: Extract `mcp/args.rs`**

Create `crates/tauri-pilot-cli/src/mcp/args.rs` containing `required_string`, `optional_string`, `required_u64`, `optional_u64`, `optional_usize`, `optional_i32`, `optional_u8`, `optional_bool` (lines 1058–1145 of `_legacy.rs`). Visibility: `pub(super) fn`. Imports: `use rmcp::{ErrorData as McpError, model::{ErrorCode, JsonObject}}; use super::responses::mcp_error;`.

> **Note:** if `window_arg` is a method on `PilotMcpServer` (it likely is — search with `rtk proxy grep -n "fn window_arg" crates/tauri-pilot-cli/src/mcp/_legacy.rs`), leave it on the impl for now. It moves later in Task 5/8.

In `mcp/mod.rs`, add `mod args;`.

In `_legacy.rs`, delete the eight function definitions and add: `use super::args::{required_string, optional_string, required_u64, optional_u64, optional_usize, optional_i32, optional_u8, optional_bool};`.

Run: `cargo build --workspace && cargo test --workspace`
Expected: green.

- [ ] **Step 4.6: Extract `mcp/schemas.rs`**

Create `crates/tauri-pilot-cli/src/mcp/schemas.rs` containing `schema_object`, `properties`, `string_prop`, `number_prop`, `bool_prop`, `array_prop`, `enum_prop` (lines 1514–1560 of `_legacy.rs`). Visibility: `pub(super) fn`. Imports: `use std::sync::Arc; use rmcp::model::JsonObject; use serde_json::{Map, Value, json};`.

In `mcp/mod.rs`, add `mod schemas;`.

In `_legacy.rs`, delete the seven function definitions and add: `use super::schemas::{schema_object, properties, string_prop, number_prop, bool_prop, array_prop, enum_prop};`.

Run: `cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: all green.

- [ ] **Step 4.7: Commit**

```bash
git add -A crates/tauri-pilot-cli/src/mcp
git commit -m "refactor(cli): extract mcp banner/args/responses/schemas helpers (issue #70)

Renamed mcp.rs to mcp/_legacy.rs as a temporary container; helpers drained
into sibling modules. _legacy.rs will be deleted once empty (PR1 Task 8)."
```

---

## Task 5: Extract `PilotMcpServer` + `ServerHandler` impl to `mcp/server.rs`

**Files:**
- Create: `crates/tauri-pilot-cli/src/mcp/server.rs`
- Modify: `crates/tauri-pilot-cli/src/mcp/_legacy.rs` (drained)

- [ ] **Step 5.1: Create `mcp/server.rs`**

Move from `_legacy.rs` to `crates/tauri-pilot-cli/src/mcp/server.rs`:
- `pub(crate) struct PilotMcpServer` (lines 26–31 of old mcp.rs)
- `pub async fn run_mcp_server` (lines 33–44)
- `impl PilotMcpServer` block: `new`, `connect_client`, `call_app`, `call_app_tool` (lines 73–119)
- `impl ServerHandler for PilotMcpServer` block (search `impl ServerHandler` in `_legacy.rs`)

Imports needed:
```rust
use std::{io::IsTerminal, path::PathBuf, sync::{Arc, OnceLock}};
use anyhow::Result;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ErrorCode, Implementation,
        JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities,
        ServerInfo, Tool, ToolAnnotations},
    service::{MaybeSendFuture, RequestContext, RoleServer},
    transport::stdio,
};
use serde_json::Value;
use crate::client::Client;
use crate::resolve_socket;
use super::banner::print_startup_banner;
use super::responses::{tool_success, tool_error};
use super::tools::tools;
use super::handlers::call_tool_by_name;
```

`call_tool_by_name` is currently a method on `PilotMcpServer`. **Step 5.2** fixes that.

- [ ] **Step 5.2: Convert `call_tool_by_name` from method to free function**

Inside `_legacy.rs`, find:
```rust
async fn call_tool_by_name(
    &self,
    name: &str,
    args: JsonObject,
) -> Result<CallToolResult, McpError> { ... }
```

Change to a free function in `crates/tauri-pilot-cli/src/mcp/handlers/mod.rs` that takes `&PilotMcpServer`:
```rust
pub(super) async fn call_tool_by_name(
    server: &super::server::PilotMcpServer,
    name: &str,
    args: JsonObject,
) -> Result<CallToolResult, McpError> {
    // body — same as the old method, replacing `self` with `server`
}
```

Inside `ServerHandler::call_tool` impl, update the call site:
```rust
// Was: self.call_tool_by_name(name, args).await
super::handlers::call_tool_by_name(self, name, args).await
```

> **Note for impl:** `PilotMcpServer` has methods (`call_app_tool`, `target_call`, `assert_text`, etc.) used by `call_tool_by_name`. Keep those methods on `PilotMcpServer` for now; the handler module calls them via `server.<method>(...)`. The next tasks will progressively extract those too.

- [ ] **Step 5.3: Update `mcp/mod.rs` re-exports for the moved server**

Update `mcp/mod.rs`:
```rust
mod _legacy;
mod args;
mod banner;
mod handlers;
mod responses;
mod schemas;
mod server;

pub use server::run_mcp_server;
pub use _legacy::tools;     // tools() still in _legacy until Task 6
```

> **Note:** `mod handlers;` is added now even though we only created `handlers/mod.rs` with `call_tool_by_name` extracted as a free fn (Step 5.2). The other handler files don't exist yet — they're added in Task 7.

- [ ] **Step 5.4: Compile + tests**

Run:
```bash
cargo build --workspace 2>&1 | tail -40
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: all green.

- [ ] **Step 5.5: Commit**

```bash
git add -A crates/tauri-pilot-cli/src/mcp
git commit -m "refactor(cli): extract PilotMcpServer + run loop into mcp/server.rs (issue #70)"
```

---

## Task 6: Split `tools()` registry into per-domain files

**Files:**
- Create: `crates/tauri-pilot-cli/src/mcp/tools/mod.rs`
- Create: `crates/tauri-pilot-cli/src/mcp/tools/{core,interact,inspect,eval,observe,storage,assert,record}.rs`

- [ ] **Step 6.1: Create `tools/mod.rs`**

Create `crates/tauri-pilot-cli/src/mcp/tools/mod.rs`:
```rust
//! MCP tool registry. Each per-domain file returns its own `Vec<Tool>`,
//! and `tools()` concatenates them.

mod assert;
mod core;
mod eval;
mod inspect;
mod interact;
mod observe;
mod record;
mod storage;

use rmcp::model::Tool;

pub fn tools() -> Vec<Tool> {
    let mut all = Vec::new();
    all.extend(core::tools());
    all.extend(interact::tools());
    all.extend(inspect::tools());
    all.extend(eval::tools());
    all.extend(observe::tools());
    all.extend(storage::tools());
    all.extend(assert::tools());
    all.extend(record::tools());
    all
}
```

- [ ] **Step 6.2: Create `tools/core.rs`**

Open `_legacy.rs`, find the `fn tools()` body (search for `Tool {`). For each tool defined there, copy the tool definition into the appropriate per-domain file. For `core.rs`, include: ping, windows, state, snapshot, diff, screenshot, navigate, url, title, wait.

Pattern:
```rust
use rmcp::model::Tool;
use serde_json::json;
use super::super::schemas::{schema_object, properties, string_prop, bool_prop, number_prop};

pub(super) fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "ping".into(),
            description: Some("Check that the bridge is reachable.".into()),
            input_schema: schema_object(properties([]), &[]),
            annotations: None,
        },
        // ... more tools
    ]
}
```

> **Note for impl:** the exact tool definitions live in `_legacy.rs`. Do not paraphrase — copy the existing `Tool { ... }` literals byte-for-byte into the per-domain file, then delete from `_legacy.rs`. The char tests in `tests/mcp_dispatch.rs` will catch any drift in name or count.

- [ ] **Step 6.3: Repeat for each domain file**

Create `tools/interact.rs`, `tools/inspect.rs`, `tools/eval.rs`, `tools/observe.rs`, `tools/storage.rs`, `tools/assert.rs`, `tools/record.rs` — same pattern, partition tools by name prefix as listed in the file structure section above.

- [ ] **Step 6.4: Remove `fn tools()` from `_legacy.rs`**

Once all per-domain `tools()` functions exist, delete the original `fn tools()` from `_legacy.rs`. The `mcp/mod.rs` already re-exports `tools::tools` (Step 4.1).

Remove the temporary `pub use _legacy::tools;` if still present.

- [ ] **Step 6.5: Cap check**

Run: `find crates/tauri-pilot-cli/src/mcp/tools -name "*.rs" | xargs wc -l | sort -rn`
Expected: every file ≤ 150 lines. If `interact.rs` or another exceeds, split further (e.g., `interact/{mod,click,fill,scroll,drag}.rs`).

- [ ] **Step 6.6: Run tests**

Run:
```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```
Expected: all green. The `test_tools_registry_count_baseline` and `test_tools_registry_contains_baseline_tools` tests verify nothing was lost.

- [ ] **Step 6.7: Commit**

```bash
git add -A crates/tauri-pilot-cli/src/mcp
git commit -m "refactor(cli): split MCP tools registry by domain (issue #70)

Kills #[allow(clippy::too_many_lines)] on tools()."
```

---

## Task 7: Split `call_tool_by_name` into per-domain handlers

**Files:**
- Create: `crates/tauri-pilot-cli/src/mcp/handlers/{core,interact,inspect,eval,observe,storage,assert,record}.rs`
- Modify: `crates/tauri-pilot-cli/src/mcp/handlers/mod.rs` (already exists from Task 5.2, refactor to dispatch)

- [ ] **Step 7.1: Refactor `handlers/mod.rs` to prefix-dispatch**

Change `crates/tauri-pilot-cli/src/mcp/handlers/mod.rs` to:
```rust
//! MCP tool dispatcher. Routes a tool name to the per-domain handler module
//! that owns it. Each domain handler returns `Result<CallToolResult, McpError>`.

mod assert;
mod core;
mod eval;
mod inspect;
mod interact;
mod observe;
mod record;
mod storage;

use rmcp::{ErrorData as McpError, model::{CallToolResult, JsonObject}};
use super::responses::mcp_error;
use super::server::PilotMcpServer;

pub(super) async fn call_tool_by_name(
    server: &PilotMcpServer,
    name: &str,
    args: JsonObject,
) -> Result<CallToolResult, McpError> {
    let window = server.window_arg(&args)?;
    match name {
        // core
        "ping" | "windows" | "state" | "snapshot" | "diff" | "screenshot"
        | "navigate" | "url" | "title" | "wait" => {
            core::dispatch(server, name, &args, window).await
        }
        // interact
        "click" | "fill" | "type" | "press" | "select" | "check" | "scroll"
        | "drag" | "drop" => {
            interact::dispatch(server, name, &args, window).await
        }
        // inspect
        "text" | "html" | "value" | "attrs" => {
            inspect::dispatch(server, name, &args, window).await
        }
        // eval
        "eval" | "ipc" => eval::dispatch(server, name, &args, window).await,
        // observe
        "watch" | "logs" | "network" => {
            observe::dispatch(server, name, &args, window).await
        }
        // storage
        "storage_get" | "storage_set" | "storage_list" | "storage_clear"
        | "forms" => storage::dispatch(server, name, &args, window).await,
        // assert
        n if n.starts_with("assert_") => {
            assert::dispatch(server, name, &args, window).await
        }
        // record
        "record_start" | "record_stop" | "record_status" | "replay" => {
            record::dispatch(server, name, &args, window).await
        }
        other => Err(mcp_error(format!("unknown tool: {other}"))),
    }
}
```

- [ ] **Step 7.2: Create each domain handler file**

For each file `handlers/{core,interact,inspect,eval,observe,storage,assert,record}.rs`, write:
```rust
use rmcp::{ErrorData as McpError, model::{CallToolResult, JsonObject}};
use super::super::server::PilotMcpServer;
// Domain-specific imports as needed.

pub(super) async fn dispatch(
    server: &PilotMcpServer,
    name: &str,
    args: &JsonObject,
    window: Option<String>,
) -> Result<CallToolResult, McpError> {
    match name {
        // ... copy the relevant arms from `_legacy.rs::call_tool_by_name`
        _ => unreachable!("handlers/mod.rs guarantees prefix match"),
    }
}
```

For `core.rs`, copy the arms for `ping`, `windows`, `state`, `snapshot`, `diff`, `screenshot`, `navigate`, `url`, `title`, `wait`.

For `interact.rs`, copy arms for `click`, `fill`, `type`, `press`, `select`, `check`, `scroll`, `drag`, `drop`.

For `inspect.rs`: `text`, `html`, `value`, `attrs`.

For `eval.rs`: `eval`, `ipc`.

For `observe.rs`: `watch`, `logs`, `network`.

For `storage.rs`: `storage_get`, `storage_set`, `storage_list`, `storage_clear`, `forms`.

For `assert.rs`: all `assert_*`.

For `record.rs`: `record_start`, `record_stop`, `record_status`, `replay`.

> **Note for impl:** the arm bodies in `_legacy.rs` call methods like `server.assert_text(...)`, `server.target_call(...)`, etc. Keep those methods on `PilotMcpServer` (defined in `server.rs`) — the handler files just call them. Do NOT extract method bodies in PR1 (out of scope). Make the methods `pub(super)` on the `impl PilotMcpServer` block so the handlers module can reach them.

- [ ] **Step 7.3: Delete the old `call_tool_by_name` body from `_legacy.rs`**

After all arms are copied, delete the entire `async fn call_tool_by_name` from `_legacy.rs`.

- [ ] **Step 7.4: Verify cap**

Run: `find crates/tauri-pilot-cli/src/mcp/handlers -name "*.rs" | xargs wc -l | sort -rn`
Expected: every file ≤ 150 lines.

- [ ] **Step 7.5: Run tests + clippy**

Run:
```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```
Expected: all green. The `mcp_dispatch` tests pass — proves no tool was dropped.

- [ ] **Step 7.6: Commit**

```bash
git add -A crates/tauri-pilot-cli/src/mcp
git commit -m "refactor(cli): split MCP call_tool_by_name into per-domain handlers (issue #70)

Kills #[allow(clippy::too_many_lines)] on call_tool_by_name."
```

---

## Task 8: Drain `_legacy.rs` and delete it

**Files:**
- Delete: `crates/tauri-pilot-cli/src/mcp/_legacy.rs`

- [ ] **Step 8.1: Inspect remaining content**

Run: `wc -l crates/tauri-pilot-cli/src/mcp/_legacy.rs && cat crates/tauri-pilot-cli/src/mcp/_legacy.rs | head -30`

Anything still in `_legacy.rs` is a leaf utility or method that didn't get classified. Common leftovers:
- Helper methods on `PilotMcpServer` like `target_call`, `assert_text`, `assert_bool`, `assert_value`, `assert_count`, `assert_url`, `call_drop_tool`, `call_replay_tool`, `call_logs_tool`, `call_network_tool`, `window_arg`.

- [ ] **Step 8.2: Move remaining methods to `mcp/server.rs`**

Move all surviving `impl PilotMcpServer` methods from `_legacy.rs` into the `impl PilotMcpServer` block in `mcp/server.rs`. Visibility: `pub(super) async fn` so the handlers module can reach them.

- [ ] **Step 8.3: Verify `server.rs` cap**

Run: `wc -l crates/tauri-pilot-cli/src/mcp/server.rs`

If > 150 lines: extract methods further. Suggested partition:
- `mcp/server.rs` — `PilotMcpServer` struct, ctor, `connect_client`, `call_app`, `call_app_tool`, `ServerHandler` impl (≤ 150 lines)
- `mcp/server/methods.rs` — assert_*, target_call, call_*_tool helpers
- Convert `mcp/server.rs` → `mcp/server/mod.rs` if needed

> **Note for impl:** if the impl block alone exceeds 150 lines, split the file by inherent-impl method group as needed. Cap is non-negotiable.

- [ ] **Step 8.4: Delete `_legacy.rs`**

Run: `git rm crates/tauri-pilot-cli/src/mcp/_legacy.rs`

Remove `mod _legacy;` from `mcp/mod.rs`.

- [ ] **Step 8.5: Compile + tests**

Run:
```bash
cargo build --workspace 2>&1 | tail -20
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```
Expected: all green.

- [ ] **Step 8.6: Commit**

```bash
git add -A crates/tauri-pilot-cli/src/mcp
git commit -m "refactor(cli): drain mcp/_legacy.rs — split complete (issue #70)"
```

---

## Task 9: Scenario type renames (kill 4 allow attributes = 5 lint instances)

**Files:**
- Modify: `crates/tauri-pilot-cli/src/scenario.rs`
- Modify: `crates/tauri-pilot-cli/src/main.rs` (call sites)

- [ ] **Step 9.1: Rename `Scenario` → `Config`**

In `crates/tauri-pilot-cli/src/scenario.rs`, line 14–22, change:
```rust
#[allow(clippy::module_name_repetitions, clippy::struct_field_names)]
#[derive(Debug, Deserialize)]
pub(crate) struct Scenario {
    pub(crate) connect: Option<Connect>,
    #[serde(default)]
    pub(crate) scenario: ScenarioMeta,
    #[serde(default)]
    pub(crate) step: Vec<Step>,
}
```

To:
```rust
#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    pub(crate) connect: Option<Connect>,
    #[serde(default, rename = "scenario")]
    pub(crate) meta: Meta,
    #[serde(default)]
    pub(crate) step: Vec<Step>,
}
```

Note: `#[serde(rename = "scenario")]` preserves TOML schema (users still write `[scenario]` in their `.toml`).

- [ ] **Step 9.2: Rename `ScenarioMeta` → `Meta`**

In `scenario.rs`, lines 30–47, change:
```rust
#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Deserialize)]
pub(crate) struct ScenarioMeta { ... }

impl Default for ScenarioMeta { ... }
```

To:
```rust
#[derive(Debug, Deserialize)]
pub(crate) struct Meta {
    pub(crate) name: Option<String>,
    #[serde(default = "default_true")]
    pub(crate) fail_fast: bool,
    pub(crate) global_timeout_ms: Option<u64>,
}

impl Default for Meta {
    fn default() -> Self {
        Self {
            name: None,
            fail_fast: true,
            global_timeout_ms: None,
        }
    }
}
```

- [ ] **Step 9.3: Rename `Step.step_ref` → `Step.reference`**

In `scenario.rs`, lines 53–75, change:
```rust
#[allow(clippy::struct_field_names)]
#[derive(Debug, Deserialize)]
pub(crate) struct Step {
    // ...
    #[serde(rename = "ref")]
    pub(crate) step_ref: Option<String>,
    // ...
}
```

To:
```rust
#[derive(Debug, Deserialize)]
pub(crate) struct Step {
    // ...
    #[serde(rename = "ref")]
    pub(crate) reference: Option<String>,
    // ...
}
```

The `#[serde(rename = "ref")]` already preserves TOML schema — only the Rust field name changes.

- [ ] **Step 9.4: Rename `ScenarioReport` → `Report`**

In `scenario.rs`, lines 100–106, change:
```rust
#[allow(clippy::module_name_repetitions)]
#[derive(Debug)]
pub(crate) struct ScenarioReport { ... }
```

To:
```rust
#[derive(Debug)]
pub(crate) struct Report {
    pub(crate) name: String,
    pub(crate) results: Vec<StepResult>,
    pub(crate) total_duration: Duration,
}
```

Update the `impl ScenarioReport` block to `impl Report`.

- [ ] **Step 9.5: Update all call sites inside `scenario.rs`**

Run: `rtk proxy grep -n "Scenario\b\|ScenarioMeta\|ScenarioReport\|step_ref" crates/tauri-pilot-cli/src/scenario.rs`

Replace every reference (function signatures, return types, `Self` constructors, test fixtures) to use the new names: `Scenario` → `Config`, `ScenarioMeta` → `Meta`, `ScenarioReport` → `Report`, `step_ref` → `reference`.

Notable sites:
- Line 141: `pub(crate) fn load_scenario(path: &Path) -> Result<Scenario>` → `Result<Config>`
- Line 150: `scenario: &Scenario` → `scenario: &Config`
- Line 153: `Result<ScenarioReport>` → `Result<Report>`
- Line 204: `Ok(ScenarioReport {` → `Ok(Report {`
- Line 280: `step.step_ref` → `step.reference`
- Line 513: `pub(crate) fn print_report(report: &ScenarioReport)` → `&Report`
- Line 528: `pub(crate) fn write_junit_xml(report: &ScenarioReport, ...)` → `&Report`
- Tests (lines 618, 619, 682, 713, 736, 756, 781) — bulk replace.

- [ ] **Step 9.6: Update `main.rs` call sites**

Run: `rtk proxy grep -n "scenario::" crates/tauri-pilot-cli/src/main.rs`

The functions `load_scenario`, `run_scenario`, `print_report`, `write_junit_xml` keep their names (they take/return the renamed types automatically). Verify no explicit type annotation in `main.rs` references `Scenario` or `ScenarioReport`.

If any explicit type appears in `main.rs`, update: `scenario::Scenario` → `scenario::Config`, `scenario::ScenarioReport` → `scenario::Report`.

- [ ] **Step 9.7: Update `lib.rs` re-exports if needed**

`lib.rs` declares `pub mod scenario;` — no changes needed unless the module re-exports specific types (it doesn't).

- [ ] **Step 9.8: Compile + tests**

Run:
```bash
cargo build --workspace 2>&1 | tail -20
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Expected: all green. The 4 `#[allow]` attributes (kill 5 lint instances) are now unnecessary because:
- `Scenario` → `Config`: no longer matches module name `scenario` → `module_name_repetitions` silent
- Field `scenario: ScenarioMeta` → `meta: Meta`: no longer repeats type name → `struct_field_names` silent
- `ScenarioMeta` → `Meta`: no longer matches module name → `module_name_repetitions` silent
- `Step.step_ref` → `Step.reference`: no longer prefixed with type name → `struct_field_names` silent
- `ScenarioReport` → `Report`: no longer matches module name → `module_name_repetitions` silent

If clippy still complains about `module_name_repetitions` on something else, leave the `#[allow]` only with documented reason for that specific item — but verify first that the rename didn't miss a spot.

- [ ] **Step 9.9: Verify allow count dropped**

Run: `rtk proxy grep -rn "#\[allow" crates/`
Expected: 12 - 4 = **8 allows remaining** (5 in scenario went down to 1: line 225 `too_many_lines` on runner — that's PR2).

If you see more than 8, investigate which renames were missed.

- [ ] **Step 9.10: Commit**

```bash
git add crates/tauri-pilot-cli/src/scenario.rs crates/tauri-pilot-cli/src/main.rs
git commit -m "refactor(cli): rename scenario types to drop module_name_repetitions (issue #70)

- Scenario → Config (matches TOML config naming)
- ScenarioMeta → Meta (with #[serde(rename = \"scenario\")] for TOML compat)
- ScenarioReport → Report
- Step.step_ref → Step.reference (#[serde(rename = \"ref\")] preserves TOML)

Kills 4 #[allow] attributes (5 clippy lint instances). Internal-only break;
TOML schema preserved via serde renames."
```

---

## Task 10: Update CHANGELOG and final verification

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 10.1: Add CHANGELOG entry**

Open `CHANGELOG.md`. Find the `## [Unreleased]` section. Under `### Changed`, add:

```markdown
- **Phase C (PR1)** — refactor pass on the CLI crate (issue #70).
  - Split `output.rs` (1172 lines) into `output/` per-domain modules
    (json, text, snapshot, logs, network, storage, forms, watch, diff,
    windows, record); every file under the 150-line cap.
  - Split `mcp.rs` (1815 lines) into `mcp/` modules: server, banner, args,
    responses, schemas, plus `mcp/tools/` and `mcp/handlers/` per domain
    (core, interact, inspect, eval, observe, storage, assert, record).
  - Renamed scenario types to drop `module_name_repetitions` /
    `struct_field_names` allows: `Scenario → Config`, `ScenarioMeta → Meta`,
    `ScenarioReport → Report`, `Step.step_ref → Step.reference`. TOML
    schema preserved via `#[serde(rename = ...)]`.
  - Removed 6 `#[allow(...)]` attributes (5 in scenario, 2 in mcp). Net
    allow count: 12 → 6.
  - Added characterization tests: `tests/output_smoke.rs`,
    `tests/mcp_dispatch.rs`.
```

- [ ] **Step 10.2: Final verification suite**

Run:
```bash
echo "=== test ===" && cargo test --workspace
echo "=== clippy linux ===" && cargo clippy --workspace --all-targets -- -D warnings
echo "=== clippy windows ===" && cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
echo "=== fmt ===" && cargo fmt --check
echo "=== allows ===" && grep -rn "#\[allow" crates/ | wc -l
echo "=== oversized in PR1 scope ===" && find crates/tauri-pilot-cli/src/output crates/tauri-pilot-cli/src/mcp -name "*.rs" 2>/dev/null | xargs wc -l 2>/dev/null | awk '$1 > 150'
```

Expected:
- tests: green (~241 + new char tests)
- clippy: green on both targets
- fmt: clean
- allows: 6 (was 12)
- oversized in PR1 scope: empty list

If `clippy --target x86_64-pc-windows-gnu` is unavailable on the dev box (no mingw toolchain), document in the commit message and rely on CI to run it. To install: `rustup target add x86_64-pc-windows-gnu` and `apt install mingw-w64`.

- [ ] **Step 10.3: Commit CHANGELOG**

```bash
git add CHANGELOG.md
git commit -m "docs: changelog entry for Phase C PR1 (issue #70)"
```

- [ ] **Step 10.4: Push branch + open PR**

Run:
```bash
git push -u origin chore/phase-c-1-cli-low-risk
gh pr create --title "Phase C PR1: split output.rs + mcp.rs, rename scenario types (issue #70)" --body "$(cat <<'EOF'
## Summary

First stacked PR of Phase C (issue #70).

- Split `output.rs` (1172 lines) into 12 per-domain files under `output/` — every file ≤ 150 lines.
- Split `mcp.rs` (1815 lines) into `mcp/` modules: server, banner, args, responses, schemas, plus per-domain `tools/` and `handlers/` (8 domains each).
- Renamed scenario types to drop `module_name_repetitions` / `struct_field_names` allows: `Scenario → Config`, `ScenarioMeta → Meta`, `ScenarioReport → Report`, `Step.step_ref → Step.reference`. TOML schema preserved via `#[serde(rename = ...)]` — no user-facing break.
- Killed **6 `#[allow]` attributes** (5 in scenario, 2 in mcp). Allow count: 12 → 6.
- Added characterization tests: `tests/output_smoke.rs`, `tests/mcp_dispatch.rs` (pin formatter behavior + tool registry baseline).

Pure refactor — zero behavior change. Existing 241 tests pass plus new char tests.

## Test plan

- [ ] `cargo test --workspace` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` green on Linux
- [ ] `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings` green
- [ ] `cargo fmt --check` clean
- [ ] Manual smoke: `tauri-pilot ping`, `tauri-pilot snapshot` against sample app
- [ ] MCP server boots: `tauri-pilot mcp` followed by a tools/list request

## Spec / plan

Spec: `docs/superpowers/specs/2026-04-24-issue-70-phase-c-design.md`
Plan: `docs/superpowers/plans/2026-04-24-issue-70-pr1-cli-low-risk.md`

PR2 (next, stacked) will tackle `main.rs`, `cli.rs`, `scenario.rs` file split, and `client/` splits.

Refs #70.
EOF
)"
```

- [ ] **Step 10.5: Mark plan complete**

Update `docs/superpowers/plans/2026-04-24-issue-70-pr1-cli-low-risk.md` (this file): no edit needed — checkboxes already track progress.

---

## Self-Review Checklist (run before opening PR)

- [ ] All 10 tasks completed (every checkbox above ticked).
- [ ] `grep -rn "#\[allow" crates/` returns ≤ 6 (was 12).
- [ ] No file in `crates/tauri-pilot-cli/src/output/` or `crates/tauri-pilot-cli/src/mcp/` exceeds 150 lines.
- [ ] CHANGELOG entry added under `[Unreleased]` → `Changed`.
- [ ] No new `#[allow]` introduced anywhere.
- [ ] No `unsafe` blocks added.
- [ ] No new dependencies (only `tempfile` as dev-dep, if added).
- [ ] `cargo test --workspace`: ≥ 244 tests green (241 + 3 mcp dispatch + ≥ 5 output smoke).
- [ ] Clippy strict + fmt green on Linux + `x86_64-pc-windows-gnu`.
- [ ] PR description references issue #70 + spec + plan.
