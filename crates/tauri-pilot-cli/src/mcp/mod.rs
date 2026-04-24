//! MCP server for `tauri-pilot`. Split out of the original `mcp.rs`
//! (1913 lines after PR1 Task 3, issue #70) by responsibility: server lifecycle, banner,
//! argument extractors, JSON-RPC response helpers, JSON schema builders,
//! per-domain tool registries, per-domain handlers.
//!
//! `_legacy` is a TEMPORARY container that shrinks step-by-step as
//! helpers are extracted into sibling modules. It will be deleted when
//! empty (Task 8).

mod _legacy;
mod args;
mod banner;
mod handlers;
mod responses;
mod schemas;
mod server;

pub(crate) use server::run_mcp_server;
