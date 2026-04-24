# Phase C — Refactor files >150 lines and eliminate remaining `#[allow(*)]`

**Issue:** [#70](https://github.com/mpiton/tauri-pilot/issues/70)
**Date:** 2026-04-24
**Author:** Mathieu Piton (designed with Claude)
**Status:** Approved — ready for implementation planning

## Context

Phases A+B (PR #71, commit `f89a056`) cleaned up dead code, converted `unwrap → expect`, and removed easy clippy `#[allow]` entries. `cargo clippy --workspace --all-targets -- -D warnings` is green on Linux and `x86_64-pc-windows-gnu`. A new rule was added to `CLAUDE.md`: **NEVER suppress errors/warnings** (no `#[allow(...)]`).

Phase C tackles the remaining structural debt that Phases A+B deferred:

- **12 surviving `#[allow(*)]` entries** across 5 files (each requires a refactor, not a one-liner).
- **15 source files exceed the 150-line cap** defined in `CLAUDE.md` (issue 70 lists 9 — audit found 6 more: `recorder.rs`, `client/mod.rs`, `lib.rs`, `eval.rs`, `client/windows.rs`, plus `diff.rs` borderline).

## Goals

1. Eliminate every `#[allow(*)]` by fixing the root cause — zero suppression.
2. Bring every `crates/**/src/*.rs` under 150 lines (CLAUDE.md cap).
3. Zero behavior change: `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` stay green after every step (Linux + `x86_64-pc-windows-gnu`).
4. Update issue 70 to reflect the actual 15-file scope.

## Non-Goals

- No new features.
- No dependency bumps.
- No JS bridge changes (`bridge.js`, html-to-image bundle untouched).
- No public API rename outside the `scenario` module (which has no external consumers — issue 70 confirms internal-only break).
- No MSRV change.
- No behavior change anywhere — pure refactor.

## Strategy

**Hybrid 3 stacked PRs**, grouped by risk profile:

| PR | Branch | Scope | Risk |
|----|--------|-------|------|
| 1 | `chore/phase-c-1-cli-low-risk` (off `main`) | `scenario` type renames (Step 1: `ScenarioYaml→Scenario`, `ScenarioStep→Step`, drop redundant field prefixes — kills 5 allows) + `output` file split + `mcp` file split | Low (CLI-only, well-tested) |
| 2 | `chore/phase-c-2-cli-deeper` (off PR1) | `main.rs`, `cli.rs`, `scenario.rs` (Step 2: split into `mod/runner/report`), `client/` splits | Medium (binary entry, scenario runner) |
| 3 | `chore/phase-c-3-plugin-and-allows` (off PR2) | `handler`, `server/{unix,windows}`, `key`, `recorder`, `lib`, `eval`, `diff` splits + all remaining allows | Higher (plugin core, FFI) |

Two-step `scenario` refactor: PR1 only renames types and drops field prefixes (kills 5 allows, no file split). PR2 splits the now-renamed `scenario.rs` into `scenario/mod.rs` + `scenario/runner.rs` + `scenario/report.rs`. Splitting in two steps keeps the rename diff reviewable independently from the structural split.

Each PR ships independently mergeable; PR3 depends on PR2 module structure for clean diffs.

## TDD Discipline (hybrid)

- **Pure structural splits** (output rendering, mod re-exports, table extraction): rely on existing 241 tests + `cargo build` type-check.
- **Branching logic** (handler dispatch, scenario runner, MCP dispatcher, `parse_combo`): characterization tests RED first, split GREEN second, refactor THIRD.

Naming convention (per `CLAUDE.md`): `test_{action}_{condition}_{expected_result}`.

Coverage target: +30 tests across 3 PRs (~241 → ~271).

## Architecture

### Plugin (`tauri-plugin-pilot`) — final module tree

```
crates/tauri-plugin-pilot/src/
├── lib.rs                  # Builder + Plugin impl + js_init_script (split: ≤150 lines)
├── error.rs                # unchanged
├── protocol.rs             # unchanged (140 lines)
├── diff/                   # 350 → split (see `diff` exception clause below)
│   ├── mod.rs              # public API
│   ├── algorithm.rs        # core diff algorithm (must stay cohesive)
│   └── format.rs           # rendering helpers
├── eval/                   # 252 → split
│   ├── mod.rs              # EvalEngine public surface
│   └── resolver.rs         # callback resolution
├── recorder/               # 271 → split
│   ├── mod.rs              # Recorder public surface
│   ├── state.rs            # ring buffer state
│   └── events.rs           # event types + push/drain
├── key/                    # 387 → split
│   ├── mod.rs              # parse_combo entry
│   └── table.rs            # const name → Key map
├── handler/                # 1162 → split
│   ├── mod.rs              # dispatch() entry
│   ├── context.rs          # struct Context (replaces 7 args)
│   ├── table.rs            # method-name → handler fn pointer
│   ├── press.rs            # handle_press
│   ├── record.rs           # handle_record
│   ├── snapshot.rs         # handle_snapshot
│   ├── eval.rs             # handle_eval
│   └── callback.rs         # __callback + handle_callback (no allow)
└── server/
    ├── mod.rs              # 127 lines (under cap, unchanged)
    ├── unix/               # 387 → split
    │   ├── mod.rs
    │   ├── bind.rs         # socket creation, perms (0o600)
    │   └── accept.rs       # accept loop + connection handling
    └── windows/            # 704 → split
        ├── mod.rs
        ├── registry.rs     # pipe name lookup
        ├── dacl.rs         # ACL/SID logic + Vec<u64> backing
        └── server.rs       # accept loop + impersonation
```

### CLI (`tauri-pilot-cli`) — final module tree

```
crates/tauri-pilot-cli/src/
├── main.rs                 # 1743 → ≤150 lines (argv parse + dispatch only)
├── error.rs                # unchanged (2 lines)
├── style.rs                # unchanged (88 lines)
├── protocol.rs             # unchanged (141 lines)
├── commands/               # extracted from main.rs
│   ├── mod.rs
│   ├── socket.rs           # resolve_socket
│   ├── script.rs           # export_script, read_script (file + stdin)
│   └── dispatch.rs         # subcommand routing
├── cli/                    # 902 → split per subcommand group
│   ├── mod.rs              # top-level Cli struct + Subcommand enum
│   ├── eval.rs             # EvalArgs
│   ├── snapshot.rs         # SnapshotArgs
│   ├── interact.rs         # ClickArgs, FillArgs, PressArgs, etc.
│   ├── record.rs           # RecordArgs
│   └── scenario.rs         # ScenarioArgs (run subcommand)
├── output/                 # 1172 → split
│   ├── mod.rs
│   ├── json.rs             # JSON renderer
│   ├── text.rs             # colored text renderer (owo_colors)
│   └── table.rs            # table renderer
├── scenario/               # 847 → split + rename
│   ├── mod.rs              # public Scenario, Step (renamed from ScenarioYaml/ScenarioStep)
│   ├── runner.rs           # execute_step, apply_timeout, capture_failure_screenshot
│   └── report.rs           # output formatting
├── mcp/                    # 1815 → split
│   ├── mod.rs              # run_mcp_server entry
│   ├── server.rs           # MCP loop
│   ├── schemas.rs          # JSON schemas
│   ├── tools/
│   │   ├── mod.rs          # tools() registry
│   │   ├── assert.rs       # assert_* tool defs
│   │   ├── record.rs       # record_* tool defs
│   │   ├── dom.rs          # dom_* tool defs
│   │   └── capture.rs      # screenshot, snapshot, console
│   └── handlers/
│       ├── mod.rs          # call_tool_by_name dispatcher
│       ├── assert.rs       # assert_* impls
│       ├── record.rs       # record_* impls
│       ├── dom.rs          # dom_* impls
│       └── capture.rs      # screenshot, snapshot, console impls
└── client/                 # 253 → split if needed
    ├── mod.rs              # public Client trait + facade
    ├── unix.rs             # 17 lines (unchanged)
    └── windows.rs          # 203 → split internally if >150
```

## Module Boundaries

- **Each file ≤ 150 lines** (CLAUDE.md cap, enforced via `find ... | xargs wc -l | awk '$1 > 150'`).
- **One responsibility per file** (existing convention from CLAUDE.md).
- **`mod.rs` files** are re-export glue only — target ≤ 50 lines each.
- **No circular dependencies**: `cli → commands → output` (one-way), `handler → eval, recorder` (one-way).

## Data Flow & Dispatch Refactors

### `handler::dispatch` — Context struct

**Before** (7 args, `#[allow(clippy::too_many_lines, clippy::too_many_arguments)]`):

```rust
pub(crate) async fn dispatch(
    method: &str,
    params: Option<&Value>,
    engine: &EvalEngine,
    app: Option<&AppHandle>,
    window: Option<&Window>,
    webview: Option<&Webview>,
    recorder: &Recorder,
) -> Result<Value, RpcError>
```

**After** (no allow):

```rust
// handler/context.rs
pub(crate) struct Context<'a, R: Runtime> {
    pub engine: &'a EvalEngine,
    pub app: Option<&'a AppHandle<R>>,
    pub window: Option<&'a Window<R>>,
    pub webview: Option<&'a Webview<R>>,
    pub recorder: &'a Recorder,
}

// handler/mod.rs
pub(crate) async fn dispatch<R: Runtime>(
    ctx: Context<'_, R>,
    method: &str,
    params: Option<&Value>,
) -> Result<Value, RpcError> {
    match method {
        "ping"     => Ok(json!({"status": "ok"})),
        "snapshot" => snapshot::handle(ctx, params).await,
        "press"    => press::handle(ctx, params).await,
        "record"   => record::handle(ctx, params).await,
        "eval"     => eval::handle(ctx, params).await,
        // ...
        _ => Err(RpcError::method_not_found(method)),
    }
}
```

### `__callback` — AppHandle pattern

**Before** (`#[allow(clippy::needless_pass_by_value)]`):

```rust
#[tauri::command]
pub(crate) fn __callback(
    eval_engine: tauri::State<'_, EvalEngine>,
    id: u64,
    result: Option<String>,
    error: Option<String>,
) {
    handle_callback(&eval_engine, id, result, error);
}
```

**After** (no allow — state acquired locally inside body, lint targets args only):

```rust
#[tauri::command]
pub(crate) fn __callback<R: Runtime>(
    app: AppHandle<R>,
    id: u64,
    result: Option<String>,
    error: Option<String>,
) {
    let engine = app.state::<EvalEngine>();
    handle_callback(&engine, id, result, error);
}
```

`AppHandle<R>` is cheap-clone (Arc-based wrapper). `app.state::<EvalEngine>()` returns the same managed instance registered in `Builder::setup`. Verified by existing `test_eval_callback_round_trip`.

### Win32 `cast_ptr_alignment` — Vec<u64> backing

`clippy::cast_ptr_alignment` is **type-based**, not runtime-based: it fires only on stricter alignment cast. Switching backing storage from `Vec<u8>` (align 1) to `Vec<u64>` (align 8) reverses the cast direction (`*mut u64 → *mut ACL` = align 8 → 2 = looser → no lint, no allow).

**Before** (`#[allow(clippy::cast_ptr_alignment)]`):

```rust
let mut buf = vec![0u8; return_length as usize];
// ...
#[allow(clippy::cast_ptr_alignment)]
let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
```

**After** (no allow):

```rust
let words = (return_length as usize).div_ceil(8);
let mut buf: Vec<u64> = vec![0; words];
// SAFETY: GetTokenInformation accepts the byte capacity; oversize is ignored.
unsafe {
    GetTokenInformation(
        token.raw(),
        TokenUser,
        Some(buf.as_mut_ptr().cast::<c_void>()),
        return_length,
        &raw mut return_length,
    )
}.map_err(|e| std::io::Error::other(e.to_string()))?;
// align(8) → align(8): no lint.
let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
Ok((buf, sid))
```

Same trick applied to `AclBuffer`: backing storage `Box<[u64]>` via `vec![0u64; (acl_size + 7) / 8].into_boxed_slice()`. Bonus: drops manual `alloc_zeroed` / `dealloc` dance — `Box` provides automatic RAII, eliminates UB risk on null.

### `mcp::call_tool_by_name` — prefix dispatch

**After** (replaces 200+ line `match name { ... }`):

```rust
// mcp/handlers/mod.rs
pub(crate) async fn call_tool_by_name(
    name: &str,
    args: Value,
    client: &mut Client,
) -> Result<ToolResponse> {
    match name {
        n if n.starts_with("assert_")  => assert::dispatch(n, args, client).await,
        n if n.starts_with("record_")  => record::dispatch(n, args, client).await,
        n if n.starts_with("dom_")     => dom::dispatch(n, args, client).await,
        n if n.starts_with("capture_") => capture::dispatch(n, args, client).await,
        _ => Err(anyhow!("unknown tool: {name}")),
    }
}
```

Per-prefix `dispatch()` lives in its own file, ≤150 lines.

### `scenario::run` — extracted helpers

`scenario/runner.rs`:
- `execute_step(step, ctx) -> StepResult`
- `apply_timeout(fut, deadline) -> Result<T>`
- `capture_failure_screenshot(client, step) -> Option<PathBuf>`
- `record_step_outcome(report, step, result)`

Top-level `run()` becomes a loop reading from the runner module.

### `key::parse_combo` — table extraction

Move the `match name { "Enter" => Key::Enter, ... }` block to `key/table.rs` as a `const NAME_TO_KEY: &[(&str, Key)] = &[...]`. `parse_combo` shrinks to ≤30 lines: tokenize, lookup, combine.

## Error Handling

- **Plugin:** `RpcError` JSON-RPC 2.0 codes (`-32600`, `-32601`, `-32602`, `-32603`) — no change.
- **CLI:** `anyhow::Result` for binary, `thiserror` boundaries unchanged.
- **Win32 ACL:** `std::io::Error::other(...)` strings byte-identical (existing tests grep on text).

## Public API

- `tauri-plugin-pilot::init()` — unchanged signature.
- `tauri-plugin-pilot::Builder` — unchanged surface.
- CLI binary `tauri-pilot` — argv + stdout format byte-identical.
- `scenario::Scenario`, `scenario::Step` types renamed (was `ScenarioYaml`, `ScenarioStep`) — **internal-only break** per issue 70 (scenario module not re-exported in lib root, no external user).

## Testing Strategy

### Char tests added per PR

**PR1:**
- `output::json::render` — snapshot of canonical event payloads (snapshot, console, eval result)
- `output::text::render` — colored output (`owo_colors`) snapshot, ANSI strip
- `output::table::render` — table layout snapshot
- `mcp::tools::registry` — assert tool count + names sorted
- `mcp::handlers::dispatch` — one test per prefix (`assert_*`, `record_*`, `dom_*`, `capture_*`)

**PR2:**
- `commands::socket::resolve` — env override, default path, identifier interpolation
- `commands::script::read_stdin` vs `read_file` — both paths
- `cli::*` — clap parser tests per subcommand group via `clap::CommandFactory::command().debug_assert()`
- `scenario::runner::execute_step` — timeout fires, screenshot captured on fail, success no-op

**PR3:**
- `handler::dispatch` — one test per method (ping, snapshot, press, eval, record, callback)
- `handler::context` — Context construction with optional fields
- `handler::callback::__callback` — verify state lookup via AppHandle (mock with `tauri::test::mock_app`)
- `key::table` — every key name in table → expected `Key` variant (table-driven)
- `server::windows::dacl` — buffer alignment + DACL initialization (Windows-only `#[cfg(target_os = "windows")]`)
- `recorder::events::push` — ring buffer wraparound

### Manual smoke test before each PR merge

```bash
# Build sample tauri-app with plugin, run CLI happy path
cd examples/sample-app && cargo tauri dev &
tauri-pilot ping
tauri-pilot snapshot
tauri-pilot eval "document.title"
```

## Acceptance Criteria

Each PR must pass before merge:

```bash
cargo test --workspace                                       # 241 + new char tests
cargo clippy --workspace --all-targets -- -D warnings        # Linux
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
cargo fmt --check
```

Per-PR ad-hoc checks:

```bash
grep -rn "#\[allow" crates/                                  # must shrink each PR; zero after PR3
find crates -name "*.rs" -not -path "*/target/*" \
  | xargs wc -l | awk '$1 > 150'                             # must shrink each PR; zero after PR3
```

Final state after PR3:
- `grep -rn "#\[allow" crates/` returns zero results (excluding legitimate `cfg_attr`, if any).
- No file under `crates/**/src/*.rs` exceeds 150 lines (see `diff` exception clause).
- All 241 existing tests pass; ~30 new char tests added.
- Clippy strict + fmt green on Linux + `x86_64-pc-windows-gnu`.

### `diff` exception clause

`diff.rs` (350 lines) implements a tightly cohesive diff algorithm. Default plan: split into `diff/{mod,algorithm,format}.rs` per the architecture tree above. **If** the split is attempted in PR3 and the algorithm cannot be cleanly cut without leaking internal state across module boundaries (e.g., shared mutable cursor, intertwined helper functions), the file may stay monolithic at >150 lines provided:

1. The PR description documents the cohesion blocker explicitly with concrete examples.
2. The exception is reviewed and approved before merge.
3. A follow-up issue is opened to track the unresolved oversize.

Any other file >150 lines is non-negotiable — must be split.

## Rollback Plan

- Each PR is squash-merged → single revert commit if regression slips.
- Stacked branches: PR3 revert isolates to PR3 changes; PR1+PR2 stay merged.
- Char tests added per PR pin behavior — regression caught at `cargo test`.

## Edge Cases Flagged

- `__callback` rewrite changes `tauri::State<EvalEngine>` lookup site — verify `app.state::<EvalEngine>()` returns the same managed instance via existing `test_eval_callback_round_trip`.
- ACL buffer storage swap to `Vec<u64>` — `as_mut_ptr().cast::<c_void>()` returns 8-byte aligned ptr; verify GetTokenInformation accepts oversized buffer (it does — uses `return_length` as cap, ignores extra capacity).
- Win32 testing on Linux dev box: `cargo clippy --target x86_64-pc-windows-gnu` cross-compile via mingw, no runtime test on Linux. CI runs Windows native via GHA matrix.

## CHANGELOG

Each PR updates `CHANGELOG.md` `[Unreleased]` section under `Changed` (per `CLAUDE.md` discipline). Final PR3 entry summarizes Phase C completion + acceptance criteria checklist.

## References

- Issue: <https://github.com/mpiton/tauri-pilot/issues/70>
- Phase A+B PR: <https://github.com/mpiton/tauri-pilot/pull/71> (`f89a056`)
- `CLAUDE.md` rules: max 150 lines/file, no `#[allow]`, TDD RED→GREEN→REFACTOR
- Tauri State pattern: <https://stackoverflow.com/questions/79095760/mutable-app-state-in-tauri-accessible-from-tauricommand>
- clippy `cast_ptr_alignment` semantics: <https://rust-lang.github.io/rust-clippy/stable/index.html#cast_ptr_alignment>
