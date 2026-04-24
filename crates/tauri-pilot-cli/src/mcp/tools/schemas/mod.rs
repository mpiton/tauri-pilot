//! Domain-specific schema builders used by per-domain tool specs.
//!
//! Each sub-module owns one domain. All functions are re-exported flat so
//! callers can keep `use super::schemas::foo_schema;` with no path changes.

mod assert;
mod core;
mod eval;
mod interact;
mod observe;
mod record;
mod storage;

pub(super) use assert::assert_count_schema;
pub(super) use core::{diff_schema, navigate_schema, snapshot_schema, wait_schema};
pub(super) use eval::{eval_schema, ipc_schema};
pub(super) use interact::{drag_schema, drop_schema, press_schema, scroll_schema, type_schema};
pub(super) use observe::{logs_schema, network_schema, watch_schema};
pub(super) use record::replay_schema;
pub(super) use storage::{storage_get_schema, storage_set_schema};
