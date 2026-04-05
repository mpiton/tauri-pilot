# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-04-05

### Added

- **Record/replay** — capture user interactions as replayable test scripts (`record start`, `record stop --output`, `record status`, `replay`, `replay --export sh`) ([#15])
- **Multi-window support** — `windows` command lists all windows, `--window` flag targets specific window ([#14])
- **Form dump** — get all form fields at once instead of calling `value` on each input individually ([#13])
  - `tauri-pilot forms` — dump all forms on the page
  - `tauri-pilot forms --selector "#login-form"` — target a specific form
  - Shows field name, type, value, and checked state
- **Storage access** — read and write browser localStorage/sessionStorage from the CLI ([#12])
  - `tauri-pilot storage get "key"` — read a single key
  - `tauri-pilot storage set "key" "value"` — write a key-value pair
  - `tauri-pilot storage list` — dump all key-value pairs
  - `tauri-pilot storage clear` — clear all storage
  - `--session` flag to use sessionStorage instead of localStorage
- **Drag & drop support** — simulate drag interactions and file drops for kanban boards, sortable lists, and drop zones ([#11])
  - `tauri-pilot drag @e5 @e6` — drag element to another element
  - `tauri-pilot drag @e5 --offset 0,100` — drag by pixel offset
  - `tauri-pilot drop @e3 --file ./test.png` — simulate file drop on element
  - Dispatches full HTML5 drag event sequence: `dragstart`, `dragenter`, `dragover`, `drop`, `dragend`
- **DOM watch command** — observe DOM mutations with MutationObserver, debounce until stable, and return a change summary ([#10])
  - `tauri-pilot watch` — block until any DOM change, print summary
  - `--timeout` timeout in ms, `--selector` scope to subtree, `--stable` debounce duration
  - Uses `MutationObserver` with `childList`, `subtree`, `attributes`, `characterData`

## [0.1.0] - 2026-04-03

### Added

- **Built-in assertions** — one-step verification for AI agents instead of manual text+parse+compare ([#9])
  - `tauri-pilot assert text @e1 "Dashboard"` — exact text content match
  - `tauri-pilot assert visible @e3` / `hidden @e3` — element visibility checks
  - `tauri-pilot assert value @e2 "workspace"` — input value match
  - `tauri-pilot assert count ".list-item" 5` — element count by CSS selector
  - `tauri-pilot assert checked @e4` — checkbox state
  - `tauri-pilot assert contains @e1 "error"` — partial text match
  - `tauri-pilot assert url "/dashboard"` — URL substring match
  - Exit code 0 + `ok` on success, exit code 1 + `FAIL: ...` on failure
  - 3 new JS bridge functions: `visible()`, `count()`, `checked()`
- **Snapshot diff command** — compare current page state with a previous snapshot ([#8])
  - `diff` JSON-RPC method in plugin with added/removed/changed detection
  - `tauri-pilot diff` CLI command with `--ref FILE`, `--interactive`, `--selector`, `--depth` flags
  - `tauri-pilot snapshot --save FILE` flag to persist snapshots for later comparison
  - Colored diff output: red `-` removed, green `+` added, yellow `~` changed with field-level detail
  - Snapshot storage in `EvalEngine` — last snapshot retained automatically after each `snapshot` call
- **Network request interception** — monkey-patch `fetch` and `XMLHttpRequest` in the JS bridge with a 200-entry ring buffer ([#7])
  - `network.getRequests` and `network.clear` JSON-RPC methods
  - `tauri-pilot network` CLI command with `--filter`, `--failed`, `--last`, `--follow`, `--clear` flags
  - Colored status codes (2xx green, 3xx cyan, 4xx yellow, 5xx red)
  - NDJSON output in `--follow --json` mode for `jq` compatibility
- **Console log capture** — monkey-patch `console.log/warn/error/info` in the JS bridge with a 500-entry ring buffer ([#17])
  - `console.getLogs` and `console.clear` JSON-RPC methods
  - `tauri-pilot logs` CLI command with `--level`, `--last`, `--follow`, `--clear` flags
  - Colored output formatting for log levels
  - NDJSON output in `--follow --json` mode for `jq` compatibility
- **Colored CLI output** — TTY-aware formatting with `owo-colors` + `indicatif` ([#3])
  - `style.rs` reusable helpers (success/error/warning/info/dim/bold)
  - Automatic `NO_COLOR` support
  - Colored accessibility tree (cyan roles, bold names, dim refs)
  - Spinner for screenshot capture
- **Phase 1: Skeleton, Protocol, and Snapshot** — full foundation ([#1])
  - Cargo workspace with two crates (`tauri-plugin-pilot`, `tauri-pilot-cli`)
  - JSON-RPC 2.0 protocol types with round-trip tests
  - JS bridge (`window.__PILOT__`) with TreeWalker snapshot, refs, and `ROLE_MAP`
  - Unix socket server with newline-delimited JSON-RPC framing
  - `EvalEngine` with callback pattern (eval + oneshot channel + timeout)
  - `__callback` IPC handler with `__TAURI_INTERNALS__.invoke`
  - 23 JSON-RPC methods: `ping`, `snapshot`, `click`, `fill`, `type`, `press`, `select`, `check`, `scroll`, `eval`, `screenshot`, `text`, `html`, `value`, `attrs`, `wait`, `navigate`, `url`, `title`, `state`, `ipc`, `console.getLogs`, `console.clear`
  - CLI with Clap: all subcommands, target resolution (`@ref`, CSS selector, `x,y` coords), `--json` flag
  - `waitFor` with `MutationObserver` + configurable timeout
  - Screenshot support via `html-to-image` (base64 PNG save-to-file)
  - `SKILL.md` for Claude Code integration
  - Prism integration script (`scripts/integrate-prism.sh`)
  - `SocketGuard` RAII for socket cleanup on shutdown/panic
  - `resolveTarget()` helper for ref/selector/coords
  - Identifier sanitization for socket paths
  - Debug-only compilation (`#[cfg(debug_assertions)]`)
- **Documentation site** — Astro Starlight at `docs/` ([#5])
  - 6 pages: Getting Started, CLI Reference, Plugin Setup, Architecture, AI Agent Integration, Contributing
  - Dark theme with cyan accent
  - GitHub Actions workflow for GitHub Pages deployment
- **Project logo and badges** in README ([#4])

### Fixed

- Bundle `html-to-image` into bridge JS for screenshot support ([#2])
- Upgrade Node.js to 22 for Astro 6.x compatibility in CI
- IPC command injection via `JSON.parse` — use `serde_json` string literal
- JSON-RPC version field validation at server boundary
- Socket bind failure propagation from plugin setup
- `set_nonblocking` error propagation (replace `expect()` with `?`)
- `fstat`-based inode guard for socket cleanup race conditions
- `std::os::unix::net::UnixListener` for sync bind in Tauri plugin setup
- `__TAURI_INTERNALS__.invoke` instead of `__TAURI__.core.invoke`
- Bridge functions accept params object (not positional arguments)
- `build.rs` + permissions for `__callback` IPC command

[#15]: https://github.com/mpiton/tauri-pilot/issues/15
[#14]: https://github.com/mpiton/tauri-pilot/issues/14
[#13]: https://github.com/mpiton/tauri-pilot/issues/13
[#12]: https://github.com/mpiton/tauri-pilot/issues/12
[#11]: https://github.com/mpiton/tauri-pilot/issues/11
[#10]: https://github.com/mpiton/tauri-pilot/issues/10
[Unreleased]: https://github.com/mpiton/tauri-pilot/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/mpiton/tauri-pilot/releases/tag/v0.2.0
[0.1.0]: https://github.com/mpiton/tauri-pilot/releases/tag/v0.1.0
[#9]: https://github.com/mpiton/tauri-pilot/issues/9
[#1]: https://github.com/mpiton/tauri-pilot/pull/1
[#2]: https://github.com/mpiton/tauri-pilot/pull/2
[#3]: https://github.com/mpiton/tauri-pilot/pull/3
[#4]: https://github.com/mpiton/tauri-pilot/pull/4
[#5]: https://github.com/mpiton/tauri-pilot/pull/5
[#7]: https://github.com/mpiton/tauri-pilot/issues/7
[#8]: https://github.com/mpiton/tauri-pilot/issues/8
[#17]: https://github.com/mpiton/tauri-pilot/pull/17
