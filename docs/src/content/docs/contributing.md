---
title: Contributing
description: Guidelines for contributing to tauri-pilot — setup, code standards, TDD workflow, and commit conventions.
---

Contributions are welcome. Bug reports, feature requests, and pull requests are all appreciated.

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
```

## Code Standards

- No `.unwrap()` outside of tests — use `thiserror` (plugin) or `anyhow` (CLI)
- Clippy strict: `cargo clippy --workspace --all-targets -- -D warnings`, then again with the plugin's debug assertions off, as release builds compile it: `cargo clippy --workspace --all-targets --config 'profile.dev.package.tauri-plugin-pilot.debug-assertions=false' -- -D warnings`
- Modules < 150 lines, functions < 50 lines
- Edition 2024, rust-version 1.95.0

## Workflow

1. Fork the repo and create a feature branch from `main`
2. Write tests first (TDD: RED → GREEN → REFACTOR)
3. Implement the minimum to pass tests
4. Run `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy --workspace --all-targets --config 'profile.dev.package.tauri-plugin-pilot.debug-assertions=false' -- -D warnings`
5. If you touched a `Cargo.toml` or called a newly added API, check the floors
   too: `cargo +nightly update -Z direct-minimal-versions && cargo check --workspace --all-targets --locked`.
   CI runs this as the required `Direct minimal versions` job. When it fails,
   raise the floor in `Cargo.toml` for the crate cargo or rustc names. It
   rewrites the local (gitignored) `Cargo.lock`, so run `cargo update`
   afterwards to get back to the newest versions.
6. Commit with conventional messages: `feat(plugin): ...`, `fix(cli): ...`
7. Open a PR against `main`

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
