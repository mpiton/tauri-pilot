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
        #[arg(long, value_name = "FILE")]
        save: Option<std::path::PathBuf>,
    },
    /// Compare current page with previous snapshot, showing only differences
    Diff {
        /// Path to a saved snapshot file to compare against
        #[arg(long, value_name = "FILE")]
        r#ref: Option<std::path::PathBuf>,
        /// Only include interactive elements
        #[arg(short, long)]
        interactive: bool,
        /// CSS selector to scope the snapshot
        #[arg(short, long)]
        selector: Option<String>,
        /// Maximum depth to traverse
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
    /// Drag an element to another element or by offset.
    Drag {
        source: String,
        #[arg(conflicts_with = "offset")]
        target: Option<String>,
        /// Pixel offset as X,Y (e.g., "0,100").
        #[arg(long, value_name = "X,Y", conflicts_with = "target")]
        offset: Option<String>,
    },
    /// Simulate a file drop on an element.
    Drop {
        target: String,
        /// File(s) to drop. Can be repeated.
        #[arg(long, required = true)]
        file: Vec<std::path::PathBuf>,
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
    /// Watch for DOM mutations and report changes.
    Watch {
        /// CSS selector to scope observation to a subtree.
        #[arg(long)]
        selector: Option<String>,
        /// Timeout in ms (reject if no changes).
        #[arg(long, default_value = "10000")]
        timeout: u64,
        /// Wait until DOM is stable for N ms (no new mutations).
        #[arg(long, default_value = "300")]
        stable: u64,
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
    /// Assert element state
    #[command(subcommand)]
    Assert(AssertKind),
    /// Read and write browser storage (localStorage/sessionStorage).
    Storage(StorageArgs),
    /// Dump all form fields on the page.
    Forms(FormsArgs),
}

#[derive(Subcommand, Debug)]
pub(crate) enum AssertKind {
    /// Assert exact text content
    Text { target: String, expected: String },
    /// Assert element is visible
    Visible { target: String },
    /// Assert element is hidden
    Hidden { target: String },
    /// Assert input value
    Value { target: String, expected: String },
    /// Assert element count matching selector
    Count { selector: String, expected: u64 },
    /// Assert checkbox is checked
    Checked { target: String },
    /// Assert text contains substring
    Contains { target: String, expected: String },
    /// Assert current URL contains string
    Url { expected: String },
}

#[derive(clap::Args, Debug)]
pub(crate) struct StorageArgs {
    /// Use sessionStorage instead of localStorage.
    #[arg(long)]
    pub session: bool,
    #[command(subcommand)]
    pub action: StorageAction,
}

#[derive(clap::Args, Debug)]
pub(crate) struct FormsArgs {
    /// Target a specific form by CSS selector.
    #[arg(long)]
    pub selector: Option<String>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum StorageAction {
    /// Get a value by key.
    Get { key: String },
    /// Set a key-value pair.
    Set { key: String, value: String },
    /// List all key-value pairs.
    List,
    /// Clear all storage.
    Clear,
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

    #[test]
    fn test_parse_diff_command() {
        let cli = Cli::parse_from(["tauri-pilot", "--socket", "/tmp/test.sock", "diff"]);
        assert!(matches!(
            cli.command,
            Command::Diff {
                r#ref: None,
                interactive: false,
                selector: None,
                depth: None,
            }
        ));
    }

    #[test]
    fn test_parse_diff_with_ref() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/test.sock",
            "diff",
            "--ref",
            "/tmp/snap.json",
        ]);
        if let Command::Diff {
            r#ref: Some(path), ..
        } = cli.command
        {
            assert_eq!(path, std::path::PathBuf::from("/tmp/snap.json"));
        } else {
            panic!("Expected Diff command with ref");
        }
    }

    #[test]
    fn test_parse_assert_text() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/test.sock",
            "assert",
            "text",
            "@e1",
            "Dashboard",
        ]);
        if let Command::Assert(AssertKind::Text { target, expected }) = cli.command {
            assert_eq!(target, "@e1");
            assert_eq!(expected, "Dashboard");
        } else {
            panic!("Expected Assert Text command");
        }
    }

    #[test]
    fn test_parse_assert_visible() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/test.sock",
            "assert",
            "visible",
            "#submit",
        ]);
        if let Command::Assert(AssertKind::Visible { target }) = cli.command {
            assert_eq!(target, "#submit");
        } else {
            panic!("Expected Assert Visible command");
        }
    }

    #[test]
    fn test_parse_assert_count() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/test.sock",
            "assert",
            "count",
            ".list-item",
            "5",
        ]);
        if let Command::Assert(AssertKind::Count { selector, expected }) = cli.command {
            assert_eq!(selector, ".list-item");
            assert_eq!(expected, 5);
        } else {
            panic!("Expected Assert Count command");
        }
    }

    #[test]
    fn test_parse_assert_url() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/test.sock",
            "assert",
            "url",
            "/dashboard",
        ]);
        if let Command::Assert(AssertKind::Url { expected }) = cli.command {
            assert_eq!(expected, "/dashboard");
        } else {
            panic!("Expected Assert Url command");
        }
    }

    #[test]
    fn test_parse_watch_command() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/test.sock",
            "watch",
            "--selector",
            ".results",
            "--timeout",
            "5000",
            "--stable",
            "500",
        ]);
        if let Command::Watch {
            selector,
            timeout,
            stable,
        } = cli.command
        {
            assert_eq!(selector, Some(".results".to_owned()));
            assert_eq!(timeout, 5000);
            assert_eq!(stable, 500);
        } else {
            panic!("Expected Watch command");
        }
    }

    #[test]
    fn test_parse_watch_defaults() {
        let cli = Cli::parse_from(["tauri-pilot", "--socket", "/tmp/test.sock", "watch"]);
        if let Command::Watch {
            selector,
            timeout,
            stable,
        } = cli.command
        {
            assert_eq!(selector, None);
            assert_eq!(timeout, 10000);
            assert_eq!(stable, 300);
        } else {
            panic!("Expected Watch command");
        }
    }

    #[test]
    fn test_parse_drag_to_element() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/t.sock",
            "drag",
            "@e5",
            "@e6",
        ]);
        assert!(
            matches!(cli.command, Command::Drag { ref source, target: Some(_), .. } if source == "@e5")
        );
    }

    #[test]
    fn test_parse_drag_with_offset() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/t.sock",
            "drag",
            "@e5",
            "--offset",
            "0,100",
        ]);
        assert!(
            matches!(cli.command, Command::Drag { ref source, offset: Some(ref off), .. } if source == "@e5" && off == "0,100")
        );
    }

    #[test]
    fn test_parse_drop_with_file() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/t.sock",
            "drop",
            "@e3",
            "--file",
            "test.png",
        ]);
        assert!(matches!(cli.command, Command::Drop { ref target, .. } if target == "@e3"));
    }

    #[test]
    fn test_parse_drop_multiple_files() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/t.sock",
            "drop",
            "@e3",
            "--file",
            "a.png",
            "--file",
            "b.txt",
        ]);
        if let Command::Drop { file, .. } = cli.command {
            assert_eq!(file.len(), 2);
        } else {
            panic!("expected Drop command");
        }
    }

    #[test]
    fn test_parse_drag_rejects_both_target_and_offset() {
        let result = Cli::try_parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/t.sock",
            "drag",
            "@e5",
            "@e6",
            "--offset",
            "0,100",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_drop_requires_file() {
        let result = Cli::try_parse_from(["tauri-pilot", "--socket", "/tmp/t.sock", "drop", "@e3"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_storage_get() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/t.sock",
            "storage",
            "get",
            "auth_token",
        ]);
        if let Command::Storage(StorageArgs {
            session,
            action: StorageAction::Get { key },
        }) = cli.command
        {
            assert!(!session);
            assert_eq!(key, "auth_token");
        } else {
            panic!("Expected Storage Get command");
        }
    }

    #[test]
    fn test_parse_storage_set() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/t.sock",
            "storage",
            "set",
            "theme",
            "dark",
        ]);
        if let Command::Storage(StorageArgs {
            session,
            action: StorageAction::Set { key, value },
        }) = cli.command
        {
            assert!(!session);
            assert_eq!(key, "theme");
            assert_eq!(value, "dark");
        } else {
            panic!("Expected Storage Set command");
        }
    }

    #[test]
    fn test_parse_storage_list_session() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/t.sock",
            "storage",
            "--session",
            "list",
        ]);
        if let Command::Storage(StorageArgs {
            session,
            action: StorageAction::List,
        }) = cli.command
        {
            assert!(session);
        } else {
            panic!("Expected Storage List command with session flag");
        }
    }

    #[test]
    fn test_parse_storage_clear() {
        let cli = Cli::parse_from(["tauri-pilot", "--socket", "/tmp/t.sock", "storage", "clear"]);
        assert!(matches!(
            cli.command,
            Command::Storage(StorageArgs {
                action: StorageAction::Clear,
                ..
            })
        ));
    }

    #[test]
    fn test_parse_forms_command() {
        let cli = Cli::parse_from(["tauri-pilot", "--socket", "/tmp/test.sock", "forms"]);
        if let Command::Forms(FormsArgs { selector }) = cli.command {
            assert_eq!(selector, None);
        } else {
            panic!("Expected Forms command");
        }
    }

    #[test]
    fn test_parse_forms_with_selector() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/test.sock",
            "forms",
            "--selector",
            "#login",
        ]);
        if let Command::Forms(FormsArgs { selector }) = cli.command {
            assert_eq!(selector, Some("#login".to_owned()));
        } else {
            panic!("Expected Forms command with selector");
        }
    }

    #[test]
    fn test_parse_snapshot_with_save() {
        let cli = Cli::parse_from([
            "tauri-pilot",
            "--socket",
            "/tmp/test.sock",
            "snapshot",
            "--save",
            "/tmp/snap.json",
        ]);
        if let Command::Snapshot {
            save: Some(path), ..
        } = cli.command
        {
            assert_eq!(path, std::path::PathBuf::from("/tmp/snap.json"));
        } else {
            panic!("Expected Snapshot command with save");
        }
    }
}
