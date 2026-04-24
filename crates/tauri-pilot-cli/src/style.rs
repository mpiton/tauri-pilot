use owo_colors::{OwoColorize, Stream::Stdout};
use std::fmt::Display;

/// Format a success message: "✓ {msg}" in green (if terminal supports color).
#[must_use]
pub fn success(msg: &str) -> String {
    format!(
        "{} {}",
        "✓".if_supports_color(Stdout, |t| t.green()),
        msg.if_supports_color(Stdout, |t| t.green()),
    )
}

/// Format an error message: "✗ {msg}" in red.
#[must_use]
pub fn error(msg: &str) -> String {
    format!(
        "{} {}",
        "✗".if_supports_color(Stdout, |t| t.red()),
        msg.if_supports_color(Stdout, |t| t.red()),
    )
}

/// Format info text in cyan.
#[must_use]
pub fn info(msg: impl Display) -> String {
    format!("{}", msg.if_supports_color(Stdout, |t| t.cyan()))
}

/// Format text as dimmed (secondary information).
#[must_use]
pub fn dim(msg: impl Display) -> String {
    format!("{}", msg.if_supports_color(Stdout, |t| t.dimmed()))
}

/// Format text as bold.
#[must_use]
pub fn bold(msg: impl Display) -> String {
    format!("{}", msg.if_supports_color(Stdout, |t| t.bold()))
}

/// Format a failure message in red (no icon, unlike `error`).
#[must_use]
pub fn failure(msg: impl Display) -> String {
    format!("{}", msg.if_supports_color(Stdout, |t| t.red()))
}

/// Format a warning message in yellow.
#[must_use]
pub fn warn(msg: impl Display) -> String {
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

    #[test]
    fn warn_contains_message() {
        let output = warn("caution");
        assert!(output.contains("caution"));
    }
}
