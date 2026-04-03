# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

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

[#1]: https://github.com/mpiton/tauri-pilot/pull/1
[#2]: https://github.com/mpiton/tauri-pilot/pull/2
[#3]: https://github.com/mpiton/tauri-pilot/pull/3
[#4]: https://github.com/mpiton/tauri-pilot/pull/4
[#5]: https://github.com/mpiton/tauri-pilot/pull/5
[#7]: https://github.com/mpiton/tauri-pilot/issues/7
[#17]: https://github.com/mpiton/tauri-pilot/pull/17
