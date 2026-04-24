//! MCP server for `tauri-pilot`. Split out of the original `mcp.rs`
//! (1913 lines after PR1 Task 3, issue #70) by responsibility: server lifecycle, banner,
//! argument extractors, JSON-RPC response helpers, JSON schema builders,
//! per-domain tool registries, per-domain handlers.

mod args;
mod banner;
mod handlers;
mod responses;
mod schemas;
mod server;
mod tools;

pub(crate) use server::run_mcp_server;
