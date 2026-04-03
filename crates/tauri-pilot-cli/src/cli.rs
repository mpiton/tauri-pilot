use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tauri-pilot", about = "Interactive testing CLI for Tauri apps")]
pub(crate) struct Cli {
    /// Socket path (auto-detected if omitted).
    #[arg(long, env = "TAURI_PILOT_SOCKET")]
    pub socket: Option<PathBuf>,

    /// Output JSON instead of text.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Check connectivity with a running Tauri app.
    Ping,
    /// Get app state (url, title, ready).
    State,
    /// Capture an accessibility snapshot of the UI.
    Snapshot {
        #[arg(short, long)]
        interactive: bool,
        #[arg(short, long)]
        selector: Option<String>,
        #[arg(short, long)]
        depth: Option<u8>,
    },
    /// Click an element.
    Click { target: String },
    /// Clear and fill an input with a value.
    Fill { target: String, value: String },
    /// Type text character by character.
    Type { target: String, text: String },
    /// Press a keyboard key.
    Press { key: String },
    /// Select an option in a <select>.
    Select { target: String, value: String },
    /// Toggle a checkbox.
    Check { target: String },
    /// Scroll the page or an element.
    Scroll {
        direction: String,
        amount: Option<i32>,
        #[arg(long)]
        r#ref: Option<String>,
    },
    /// Get text content of an element.
    Text { target: String },
    /// Get inner HTML (of an element, or full page).
    Html { target: Option<String> },
    /// Get the value of an input/select/textarea.
    Value { target: String },
    /// Get all attributes of an element.
    Attrs { target: String },
    /// Evaluate arbitrary JavaScript.
    Eval { script: String },
    /// Invoke a Tauri IPC command.
    Ipc {
        command: String,
        #[arg(long)]
        args: Option<String>,
    },
    /// Capture a screenshot (PNG).
    Screenshot {
        path: Option<PathBuf>,
        #[arg(long)]
        selector: Option<String>,
    },
    /// Navigate to a URL.
    Navigate { url: String },
    /// Get current URL.
    Url,
    /// Get page title.
    Title,
    /// Wait for an element or condition.
    Wait {
        target: Option<String>,
        #[arg(long)]
        selector: Option<String>,
        #[arg(long)]
        gone: bool,
        #[arg(long, default_value = "10000")]
        timeout: u64,
    },
    /// Display or stream captured console logs.
    Logs {
        /// Filter by log level (log, info, warn, error).
        #[arg(long, value_parser = ["log", "info", "warn", "error"])]
        level: Option<String>,

        /// Show last N log entries.
        #[arg(long)]
        last: Option<usize>,

        /// Clear the log buffer.
        #[arg(long, conflicts_with = "follow")]
        clear: bool,

        /// Continuously poll for new logs.
        #[arg(long, short = 'f')]
        follow: bool,
    },
    /// Display or stream captured network requests.
    Network {
        /// Filter by URL pattern (substring match).
        #[arg(long)]
        filter: Option<String>,

        /// Show only failed requests (4xx/5xx and network errors).
        #[arg(long)]
        failed: bool,

        /// Show last N requests.
        #[arg(long)]
        last: Option<usize>,

        /// Clear the request buffer.
        #[arg(long, conflicts_with = "follow")]
        clear: bool,

        /// Continuously poll for new requests.
        #[arg(long, short = 'f')]
        follow: bool,
    },
}

/// Parsed target for element-targeting commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Ref(String),
    Selector(String),
    Coords(i32, i32),
}

/// Parse a target string into a `Target` variant.
pub(crate) fn parse_target(s: &str) -> Target {
    if let Some(r) = s.strip_prefix('@') {
        return Target::Ref(r.to_owned());
    }

    if let Some((x_str, y_str)) = s.split_once(',')
        && let (Ok(x), Ok(y)) = (x_str.trim().parse::<i32>(), y_str.trim().parse::<i32>())
    {
        return Target::Coords(x, y);
    }

    Target::Selector(s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_target_ref() {
        assert_eq!(parse_target("@e1"), Target::Ref("e1".to_owned()));
        assert_eq!(parse_target("@e42"), Target::Ref("e42".to_owned()));
    }

    #[test]
    fn test_parse_target_selector() {
        assert_eq!(
            parse_target("#submit-btn"),
            Target::Selector("#submit-btn".to_owned())
        );
        assert_eq!(
            parse_target(".class"),
            Target::Selector(".class".to_owned())
        );
    }

    #[test]
    fn test_parse_target_coords() {
        assert_eq!(parse_target("100,200"), Target::Coords(100, 200));
        assert_eq!(parse_target("0, 0"), Target::Coords(0, 0));
    }

    #[test]
    fn test_parse_target_invalid_coords_as_selector() {
        assert_eq!(
            parse_target("abc,def"),
            Target::Selector("abc,def".to_owned())
        );
    }
}
