# Contributing to tauri-pilot

Thanks for your interest in contributing! This document covers the development workflow.

## Prerequisites

- Rust 1.94.1+ (LTS) with edition 2024
- A Tauri v2 app for testing (or use the examples)
- Linux (WebKitGTK) — macOS/Windows not yet supported

## Development Setup

```bash
git clone https://github.com/mpiton/tauri-pilot.git
cd tauri-pilot
cargo build --workspace
cargo test --workspace
```

## Code Standards

- **No `.unwrap()`** outside of tests — use `thiserror` (plugin) or `anyhow` (CLI)
- **Clippy strict**: `cargo clippy --workspace -- -D warnings`
- **Modules < 150 lines**, functions < 50 lines
- **Edition 2024**, rust-version 1.94.1

## Workflow

1. Fork the repo and create a feature branch from `main`
2. Write tests first (TDD: RED → GREEN → REFACTOR)
3. Implement the minimum to pass tests
4. Run `cargo test --workspace && cargo clippy --workspace -- -D warnings`
5. Commit with conventional messages: `feat(plugin): ...`, `fix(cli): ...`
6. Open a PR against `main`

## Commit Scopes

`plugin`, `cli`, `bridge`, `protocol`, `workspace`, `docs`, `ci`

## Architecture

See [ARCHI.md](ARCHI.md) for architecture decisions and module structure.

## Reporting Issues

Use the issue templates — bug reports and feature requests are welcome.
