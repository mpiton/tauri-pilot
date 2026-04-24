# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Windows support** — named pipe server and client for Windows, with security hardening (DACL, SID validation), registry-based instance discovery, and platform-specific tests ([#64])
- **Batch scenario runner** — `tauri-pilot run <scenario.toml>` executes declarative TOML scenarios with 18 action types (click, fill, type, press, select, check, scroll, navigate, wait, watch, eval, screenshot, assert-text, assert-exists, assert-visible, assert-hidden, assert-value, assert-url). Supports `fail_fast` (default true), `--no-fail-fast` override, `--junit <file>` for JUnit XML output, and auto-captures failure screenshots to `./tauri-pilot-failures/`. Exit code 0 = all pass, 1 = any failure. Example: `docs/examples/login-flow.toml` ([#62])
- `connect.timeout_ms` in TOML scenarios — wraps `Client::connect` in `tokio::time::timeout` ([#63])
- `global_timeout_ms` in TOML scenarios — hard deadline around `run_scenario` ([#63])
- Per-step `timeout_ms` applied to all non-`wait`/`watch` actions via `tokio::time::timeout` ([#63])
- `<testsuites>` JUnit XML root now carries `tests`, `failures`, `errors`, `skipped`, `time` aggregate attributes for CI reporters (Jenkins, GitHub Actions, Allure) ([#63])

### Fixed

- **Windows security hardening** — fixed heap overflow in ACL allocation, use-after-free on the SID buffer, a no-op peer-SID check (`OpenProcessToken` on the impersonated thread replaced with `OpenThreadToken`), UB on the alloc-failure path, silent fallback to a broader default DACL, and a permanently-aborting accept loop. Also switched `instances_dir` creation to `std::fs::create_dir_all` for correctness on clean profiles, and added a regression test asserting the bound pipe carries a user-only DACL with one ACE ([#64])
- `tauri-pilot-cli` Windows builds now enable the `Win32_Foundation` and `Win32_System_Threading` features on the `windows` crate so `is_pid_alive` compiles on Windows CI ([#64])
- `tauri-plugin-pilot` marks the Linux-target `enigo` entry as `optional = true` so `--no-default-features` actually drops the dependency — previously the `press` feature gate was silently defeated by cargo merging the target-specific entry with the top-level one ([#64])
- `tauri-plugin-pilot` server now caps per-line reads using `AsyncReadExt::take` before invoking `read_line`, so a peer flooding bytes without a newline can no longer OOM the host process — the existing `MAX_LINE_LENGTH` check was applied after the full line had already been buffered ([#64])
- `tauri-pilot-cli` Unix client tests use a per-process, atomic-counter socket path instead of hard-coded `/tmp/tauri-pilot-test-*.sock` paths, so parallel `cargo test` runs no longer cross-wire through the same socket file ([#64])
- `tauri-pilot-cli` Windows registry-resolution tests mock entries with `std::process::id()` instead of a fabricated dead PID, so the liveness filter added in the Windows support work doesn't skip them ([#64])
- `assert-exists` now verifies the `visible` key is present in the RPC response to catch missing DOM elements ([#63])

### Changed

- `tauri-plugin-pilot` `init()` doc comment clarifies the no-op fallback now excludes Windows too and mentions the Named Pipe server path ([#64])
- Bumped `indicatif` from `0.17` to `0.18` (brings the transitive `console` update to `0.16`; only `ProgressBar::new_spinner()` is used — API stable)
- Bumped docs dependencies: `astro` `6.1.3` → `6.1.9`, `@astrojs/starlight` `0.38.2` → `0.38.4`, transitively `vite` `7.3.1` → `7.3.2`
- Refreshed `Cargo.lock` patch-level updates: `libc` `0.2.184` → `0.2.186`, `clap` / `clap_derive` `4.6.0` → `4.6.1`, `assert_cmd` `2.2.0` → `2.2.1`

### Security

- Resolved Dependabot alerts #1–#4 (all in `docs/`):
  - `astro` XSS via `define:vars` incomplete `</script>` sanitization ([GHSA-j687-52p2-xcff](https://github.com/advisories/GHSA-j687-52p2-xcff), CVE-2026-41067) — patched upstream in `astro@6.1.6`; included via bump to `astro@6.1.9`
  - `vite` path traversal in optimized deps `.map` handling ([GHSA-4w7w-66w2-5vf9](https://github.com/advisories/GHSA-4w7w-66w2-5vf9), CVE-2026-39365)
  - `vite` `server.fs.deny` bypass via query parameters ([GHSA-v2wj-q39q-566r](https://github.com/advisories/GHSA-v2wj-q39q-566r), CVE-2026-39364)
  - `vite` arbitrary file read via dev-server WebSocket ([GHSA-p9ff-h696-f583](https://github.com/advisories/GHSA-p9ff-h696-f583), CVE-2026-39363)
  - All three `vite` CVEs fixed transitively via `astro@6.1.9` → `vite@7.3.2`

### Removed

- `.gitignore` no longer ignores `CLAUDE.md`, `.sisyphus/`, `.agents/`, or `skills-lock.json` — these are personal tooling entries that belong in a user-level `~/.gitignore_global`, not in the repo ([#64])

## [0.4.0] - 2026-04-17

### Added

- `eval` command now reads the script from stdin when the argument is `-` or omitted ([#41])
- Stdin heredoc and pipe examples for `eval -` in README, SKILL.md, and CLI reference ([#50])
- MCP server mode for exposing tauri-pilot commands as structured tools over stdio ([#51])
- `watch --require-mutation` flag defers the stability timer until at least one DOM mutation occurs, then waits for `--stable` ms of quiet. Rejects on timeout if nothing mutated. Use after IPC calls that trigger async re-renders (e.g. React state updates) where you need to block until the re-render lands ([#49])

### Changed

- **Breaking:** `watch` default semantics changed — the stability timer now arms at startup instead of on the first mutation. Idle runs resolve after `--stable` ms with an empty change set (`{added:[], removed:[], modified:[]}`) instead of rejecting with a "no DOM changes" timeout error. Scripts that relied on `watch` as a "did anything change?" assertion should switch to `watch --require-mutation` to keep the old reject-on-idle behaviour ([#49])
- `press` command now injects keyboard events at the OS level via `enigo` instead of dispatching synthetic JS `KeyboardEvent`s. Events are now `isTrusted=true` and traverse the full input pipeline, reaching DOM listeners, Tauri accelerators, and global shortcut handlers ([#45])
- The plugin now requests window focus before injecting keys so events land on the correct webview
- `tauri-plugin-pilot` exposes a default-on `press` feature that gates the `enigo` dependency. Build with `--no-default-features` to drop it from release builds where the whole plugin is already no-op'd ([#53])
- Bumped MSRV from Rust 1.94.0 to **1.95.0** (workspace `Cargo.toml`, `ci.yml`, `release.yml`). Public docs (README, CONTRIBUTING, docs site) updated and the inaccurate "LTS" wording dropped — Rust does not yet ship an LTS channel ([#54])

### Fixed

- `eval` now accepts `const`, `let`, `var`, function declarations and other statements — the previous `new Function("return (…)")` wrapper forced an expression context and rejected top-level declarations. Scripts now run through indirect `eval`, which returns the completion value of the last expression ([#46])
- `click` now dispatches pointer events before mouse events so Radix UI dropdown, select, and dialog triggers open correctly ([#52])
- `press "Control+1"` and similar combos now trigger Tauri global shortcuts and any handler that requires trusted keyboard events ([#45])
- `press` with an explicit `--window <label>` now returns an error when the target window cannot be focused, instead of silently delivering the key to whatever window currently holds focus ([#53])
- `press` serializes the full `focus → settle → inject` sequence, so two concurrent calls targeting different windows can no longer race on the focus step and cross their keys ([#53])
- `press` combo parser now rejects empty segments between `+` (e.g. `Control++P`, `+A`) instead of silently normalizing them into different shortcuts ([#53])
- `press` now explicitly enables enigo's `wayland` backend so OS-level key injection works on Wayland sessions, not just X11 ([#53])
- `press` validates the combo string before taking the focus lock or stealing focus, so malformed input returns `-32602` (invalid params) immediately instead of `-32603` after an 80ms focus settle ([#53])
- `simulate_press` now propagates modifier-release failures instead of dropping them, so a combo can no longer return `Ok(())` while leaving a modifier stuck down ([#53])
- `press` with `--window <label>` but no focus hook installed now errors instead of silently injecting into whatever window has focus ([#53])
- `Enigo::new` failure hint about macOS Accessibility permission is now gated to macOS builds — Linux and Windows errors no longer point users at the wrong remediation ([#53])
- `press` JoinError handling distinguishes panics from cancellation and runtime-shutdown cases instead of reporting every failure as "panicked" ([#53])
- `eval` now exits with code 0 when the JS expression returns `undefined` (e.g. `element.click()`, void functions). Previously the CLI bailed with `Error: Server returned empty result without error`, breaking bash `&&` chains and `set -e` scripts even though the eval had succeeded ([#48])

## [0.3.0] - 2026-04-10

### Added

- **macOS support** — CI verification on macOS, updated documentation and platform requirements ([#37])

## [0.2.1] - 2026-04-05

### Security

- **Socket hardening** — three layers of defense against local privilege escalation ([#31])
  - Socket permissions set to `0o600` (owner-only) immediately after bind
  - `umask(0o177)` guard around bind to eliminate TOCTOU race window
  - Peer credential (UID) verification rejects connections from other local users
  - Socket placed in `$XDG_RUNTIME_DIR` (user-private `0o700` directory) with `/tmp` fallback
  - `XDG_RUNTIME_DIR` validated for ownership and permissions before use
  - CLI `resolve_socket()` filters candidates by UID ownership

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
[#37]: https://github.com/mpiton/tauri-pilot/issues/37
[#41]: https://github.com/mpiton/tauri-pilot/pull/41
[#45]: https://github.com/mpiton/tauri-pilot/issues/45
[#46]: https://github.com/mpiton/tauri-pilot/issues/46
[#48]: https://github.com/mpiton/tauri-pilot/issues/48
[#49]: https://github.com/mpiton/tauri-pilot/issues/49
[#50]: https://github.com/mpiton/tauri-pilot/pull/50
[#51]: https://github.com/mpiton/tauri-pilot/pull/51
[#52]: https://github.com/mpiton/tauri-pilot/pull/52
[#53]: https://github.com/mpiton/tauri-pilot/pull/53
[#54]: https://github.com/mpiton/tauri-pilot/issues/54
[#62]: https://github.com/mpiton/tauri-pilot/pull/62
[#63]: https://github.com/mpiton/tauri-pilot/pull/63
[Unreleased]: https://github.com/mpiton/tauri-pilot/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/mpiton/tauri-pilot/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/mpiton/tauri-pilot/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/mpiton/tauri-pilot/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/mpiton/tauri-pilot/compare/v0.1.0...v0.2.0
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
[#31]: https://github.com/mpiton/tauri-pilot/issues/31
[#64]: https://github.com/mpiton/tauri-pilot/pull/64
