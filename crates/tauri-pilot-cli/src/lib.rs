//! Library facade for integration tests.
//!
//! Binary code lives in `main.rs`; this file re-exports the modules used by
//! the integration test suite under `tests/`. Keeping the binary entry point
//! and the test surface separate avoids having to duplicate code.
//!
//! NOTE: `mcp`, `scenario`, `client`, `protocol` are intentionally omitted —
//! they reference helper functions (`resolve_socket`, `target_params`,
//! `with_window`, etc.) or have visibility mismatches that are only resolved
//! when the binary and lib targets share `main.rs`. Those modules will be
//! made self-contained in Task 4–7 (issue #70).

pub mod output;
pub mod style;
