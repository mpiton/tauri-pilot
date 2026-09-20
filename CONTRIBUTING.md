# Contributing to tauri-pilot

Thanks for your interest in contributing! This document covers the development workflow.

## Prerequisites

- Rust 1.95.0+ with edition 2024
- A Tauri v2 app for testing (or use the examples)
- Linux (WebKitGTK) or macOS (WebKit) — Windows planned

## Development Setup

```bash
git clone https://github.com/mpiton/tauri-pilot.git
cd tauri-pilot
cargo build --workspace
cargo test --workspace
node --test 'crates/tauri-plugin-pilot/js/*.test.mjs'
```

The injected bridge is plain JS with its own `node:test` suite. Pass the glob:
`node --test crates/tauri-plugin-pilot/js/` cannot resolve the files.

## Code Standards

- **No `.unwrap()`** outside of tests — use `thiserror` (plugin) or `anyhow` (CLI)
- **Clippy strict**: `cargo clippy --workspace --all-targets -- -D warnings`, then again with the plugin's debug assertions off, as release builds compile it: `cargo clippy --workspace --all-targets --config 'profile.dev.package.tauri-plugin-pilot.debug-assertions=false' -- -D warnings`
- **Modules < 150 lines**, functions < 50 lines
- **Edition 2024**, rust-version 1.95.0

## Workflow

1. Fork the repo and create a feature branch from `main`
2. Write tests first (TDD: RED → GREEN → REFACTOR)
3. Implement the minimum to pass tests
4. Run `cargo test --workspace && node --test 'crates/tauri-plugin-pilot/js/*.test.mjs' && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy --workspace --all-targets --config 'profile.dev.package.tauri-plugin-pilot.debug-assertions=false' -- -D warnings`
5. If you touched a `Cargo.toml` or called a newly added API, check the floors
   too: `cargo +nightly update -Z direct-minimal-versions && cargo check --workspace --all-targets --locked`.
   CI runs this as the required `Direct minimal versions` job. When it fails,
   raise the floor in `Cargo.toml` for the crate cargo or rustc names. It
   rewrites the local (gitignored) `Cargo.lock`, so run `cargo update`
   afterwards to get back to the newest versions.
6. Commit with conventional messages: `feat(plugin): ...`, `fix(cli): ...`
7. Open a PR against `main`

## Commit Scopes

`plugin`, `cli`, `bridge`, `protocol`, `workspace`, `docs`, `ci`

## Architecture

See the [Architecture guide](https://mpiton.github.io/tauri-pilot/guides/architecture/) for design decisions and module structure.

## `run` vs `record`/`replay`

tauri-pilot offers two complementary automation modes:

| Mode | Command | Format | Use case |
|------|---------|--------|----------|
| **Declarative** | `tauri-pilot run <file.toml>` | TOML scenario | Structured tests with assertions and timeouts (CI-friendly) |
| **Capture-replay** | `tauri-pilot record start` / `replay` | JSON session | Quick capture of manual interactions for later replay |

**`run` (TOML scenario)** — define steps declaratively with action types, assertions, and
timeouts. Exits 0 on success, 1 on any failure. Supports JUnit XML output (`--junit`).
Automatically captures failure screenshots to `./tauri-pilot-failures/`, or to
`--screenshots-dir <DIR>`. The failed step's result carries the absolute path as
`screenshot`, or the reason the shot could not be written as `screenshot_error`.

**`record` / `replay` (JSON session)** — record interactions as they happen, then replay
the timing-accurate sequence. Useful for smoke tests derived from manual exploration.
Export to shell script with `replay --export sh`.

## Reporting Issues

Use the issue templates — bug reports and feature requests are welcome.
