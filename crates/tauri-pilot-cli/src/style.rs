use owo_colors::{OwoColorize, Stream::Stdout};
use std::fmt::Display;

/// Format a success message: "✓ {msg}" in green (if terminal supports color).
pub(crate) fn success(msg: &str) -> String {
    format!(
        "{} {}",
        "✓".if_supports_color(Stdout, |t| t.green()),
        msg.if_supports_color(Stdout, |t| t.green()),
    )
}

/// Format an error message: "✗ {msg}" in red.
pub(crate) fn error(msg: &str) -> String {
    format!(
        "{} {}",
        "✗".if_supports_color(Stdout, |t| t.red()),
        msg.if_supports_color(Stdout, |t| t.red()),
    )
}

/// Format info text in cyan.
pub(crate) fn info(msg: impl Display) -> String {
    format!("{}", msg.if_supports_color(Stdout, |t| t.cyan()))
}

/// Format text as dimmed (secondary information).
pub(crate) fn dim(msg: impl Display) -> String {
    format!("{}", msg.if_supports_color(Stdout, |t| t.dimmed()))
}

/// Format text as bold.
pub(crate) fn bold(msg: impl Display) -> String {
    format!("{}", msg.if_supports_color(Stdout, |t| t.bold()))
}

/// Format a warning message in yellow.
pub(crate) fn warn(msg: impl Display) -> String {
    format!("{}", msg.if_supports_color(Stdout, |t| t.yellow()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_contains_checkmark_and_message() {
        let output = success("ok");
        assert!(output.contains('✓'));
        assert!(output.contains("ok"));
    }

    #[test]
    fn error_contains_cross_and_message() {
        let output = error("failed");
        assert!(output.contains('✗'));
        assert!(output.contains("failed"));
    }

    #[test]
    fn info_contains_message() {
        let output = info("note");
        assert!(output.contains("note"));
    }

    #[test]
    fn dim_contains_message() {
        let output = dim("secondary");
        assert!(output.contains("secondary"));
    }

    #[test]
    fn bold_contains_message() {
        let output = bold("important");
        assert!(output.contains("important"));
    }
}
