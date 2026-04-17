---
title: Contributing
description: Guidelines for contributing to tauri-pilot — setup, code standards, TDD workflow, and commit conventions.
---

Contributions are welcome. Bug reports, feature requests, and pull requests are all appreciated.

## Prerequisites

- Rust 1.95.0+ with edition 2024
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

- No `.unwrap()` outside of tests — use `thiserror` (plugin) or `anyhow` (CLI)
- Clippy strict: `cargo clippy --workspace -- -D warnings`
- Modules < 150 lines, functions < 50 lines
- Edition 2024, rust-version 1.95.0

## Workflow

1. Fork the repo and create a feature branch from `main`
2. Write tests first (TDD: RED → GREEN → REFACTOR)
3. Implement the minimum to pass tests
4. Run `cargo test --workspace && cargo clippy --workspace -- -D warnings`
5. Commit with conventional messages: `feat(plugin): ...`, `fix(cli): ...`
6. Open a PR against `main`

## Commit Scopes

Use one of the following scopes in your commit messages:

| Scope | Area |
|-------|------|
| `plugin` | `crates/tauri-plugin-pilot` |
| `cli` | `crates/tauri-pilot-cli` |
| `bridge` | `crates/tauri-plugin-pilot/js/bridge.js` |
| `protocol` | JSON-RPC protocol definitions |
| `workspace` | Root `Cargo.toml`, workspace config |
| `docs` | Documentation |
| `ci` | GitHub Actions workflows |

## Reporting Issues

Use the issue templates on GitHub — bug reports and feature requests are welcome.

## License

MIT
