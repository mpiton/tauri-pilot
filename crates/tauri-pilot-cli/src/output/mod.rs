//! Per-format / per-domain output renderers for the `tauri-pilot` CLI.
//!
//! Split out of the original `output.rs` (1172 lines, issue #70) by domain
//! to keep each file under the 150-line cap defined in `CLAUDE.md`.

mod diff;
mod forms;
mod json;
mod logs;
mod network;
mod record;
mod snapshot;
mod storage;
mod text;
mod watch;
mod windows;

pub use diff::format_diff;
pub use forms::format_forms;
pub use json::format_json;
pub use logs::format_logs;
pub use network::format_network;
pub use record::{format_record, format_replay_step};
pub use snapshot::format_snapshot;
pub use storage::{format_storage, format_storage_value};
pub use text::{format_assert_fail, format_text};
pub use watch::format_watch;
pub use windows::format_windows;

/// Strip ANSI escape sequences and C0 control characters from a string
/// to prevent terminal injection.
pub(super) fn strip_ansi(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip CSI sequences: ESC [ ... final_byte
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() || next == '~' {
                        break;
                    }
                }
            // Skip OSC sequences: ESC ] ... BEL or ESC \ (ST)
            } else if chars.peek() == Some(&']') {
                chars.next();
                let mut prev_was_esc = false;
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '\x07' {
                        break;
                    }
                    if prev_was_esc && next == '\\' {
                        break;
                    }
                    prev_was_esc = next == '\x1b';
                }
            } else {
                // Skip single-char escape (ESC + one char)
                chars.next();
            }
        } else if c.is_control() && c != '\n' && c != '\t' && c != '\r' {
            // Strip C0/C1 control characters (except common whitespace)
        } else {
            result.push(c);
        }
    }
    result
}

/// Format a millisecond timestamp as `HH:MM:SS.mmm`.
pub(super) fn format_timestamp(timestamp: u64) -> String {
    let secs = (timestamp / 1000) % 86400;
    let ms = timestamp % 1000;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}
