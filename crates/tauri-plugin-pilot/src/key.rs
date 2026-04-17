//! Native OS-level keyboard event injection.
//!
//! Synthetic `KeyboardEvent`s dispatched from JS are flagged `isTrusted: false`
//! and never reach the OS / window-manager layer where Tauri global shortcuts
//! and accelerators are registered. Injecting at the OS layer (via [`enigo`])
//! produces "real" key events that traverse the full pipeline:
//!
//! ```text
//! enigo → OS input subsystem → window manager → toolkit (GTK/AppKit/Win32)
//!       → webview → DOM listeners
//!       → Tauri accelerator/global-shortcut handlers
//! ```
//!
//! See issue #45.
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KeyError {
    #[error("empty key combo")]
    Empty,
    #[error("unknown key: {0}")]
    UnknownKey(String),
    #[error("enigo init failed: {0}")]
    EnigoInit(String),
    #[error("enigo input failed: {0}")]
    EnigoInput(String),
}

/// Parsed combo: zero or more modifiers and exactly one main key.
#[derive(Debug, PartialEq, Eq)]
pub struct Combo {
    pub modifiers: Vec<Key>,
    pub key: Key,
}

/// Parse a combo string like `"Control+Shift+P"` into modifiers + main key.
///
/// Accepts `+` or `-` as separators. Tokens are matched case-insensitively
/// against a small alias table; single characters become `Key::Unicode`.
pub fn parse_combo(combo: &str) -> Result<Combo, KeyError> {
    let trimmed = combo.trim();
    if trimmed.is_empty() {
        return Err(KeyError::Empty);
    }

    let tokens: Vec<&str> = trimmed
        .split(['+', '-'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return Err(KeyError::Empty);
    }

    let (last, rest) = tokens.split_last().expect("non-empty");
    let mut modifiers = Vec::with_capacity(rest.len());
    for tok in rest {
        modifiers.push(parse_modifier(tok)?);
    }
    let key = parse_key(last)?;
    Ok(Combo { modifiers, key })
}

fn parse_modifier(token: &str) -> Result<Key, KeyError> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Ok(Key::Control),
        "shift" => Ok(Key::Shift),
        "alt" | "option" => Ok(Key::Alt),
        "meta" | "super" | "cmd" | "command" | "win" => Ok(Key::Meta),
        other => Err(KeyError::UnknownKey(other.to_owned())),
    }
}

#[allow(clippy::too_many_lines)]
fn parse_key(token: &str) -> Result<Key, KeyError> {
    if let Some(ch) = single_char(token) {
        return Ok(Key::Unicode(ch));
    }
    let lower = token.to_ascii_lowercase();
    let key = match lower.as_str() {
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "space" | "spacebar" => Key::Space,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "page_up" => Key::PageUp,
        "pagedown" | "page_down" => Key::PageDown,
        "up" | "uparrow" | "arrowup" => Key::UpArrow,
        "down" | "downarrow" | "arrowdown" => Key::DownArrow,
        "left" | "leftarrow" | "arrowleft" => Key::LeftArrow,
        "right" | "rightarrow" | "arrowright" => Key::RightArrow,
        "ctrl" | "control" => Key::Control,
        "shift" => Key::Shift,
        "alt" | "option" => Key::Alt,
        "meta" | "super" | "cmd" | "command" | "win" => Key::Meta,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        _ => return Err(KeyError::UnknownKey(token.to_owned())),
    };
    Ok(key)
}

fn single_char(token: &str) -> Option<char> {
    let mut chars = token.chars();
    let first = chars.next()?;
    if chars.next().is_none() {
        Some(first)
    } else {
        None
    }
}

/// Press `combo` at the OS level. Modifiers are pressed in order, then the
/// main key is tapped (press+release), then modifiers are released in reverse
/// order — the same pattern a human or Playwright would produce.
///
/// This is a blocking OS call; callers in async contexts should wrap with
/// `tokio::task::spawn_blocking` to avoid stalling the runtime.
pub fn simulate_press(combo: &str) -> Result<(), KeyError> {
    let parsed = parse_combo(combo)?;
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| KeyError::EnigoInit(e.to_string()))?;

    for m in &parsed.modifiers {
        enigo
            .key(*m, Direction::Press)
            .map_err(|e| KeyError::EnigoInput(e.to_string()))?;
    }
    let tap_result = enigo.key(parsed.key, Direction::Click);
    // Always release modifiers, even on tap failure, to avoid leaving the OS
    // in a stuck-modifier state.
    for m in parsed.modifiers.iter().rev() {
        let _ = enigo.key(*m, Direction::Release);
    }
    tap_result.map_err(|e| KeyError::EnigoInput(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_eq(a: &Key, b: &Key) -> bool {
        // Key doesn't impl PartialEq for all variants in older versions; format!-compare.
        format!("{a:?}") == format!("{b:?}")
    }

    #[test]
    fn test_parse_single_char_returns_unicode() {
        let combo = parse_combo("a").unwrap();
        assert!(combo.modifiers.is_empty());
        assert!(key_eq(&combo.key, &Key::Unicode('a')));
    }

    #[test]
    fn test_parse_ctrl_plus_one() {
        let combo = parse_combo("Control+1").unwrap();
        assert_eq!(combo.modifiers.len(), 1);
        assert!(key_eq(&combo.modifiers[0], &Key::Control));
        assert!(key_eq(&combo.key, &Key::Unicode('1')));
    }

    #[test]
    fn test_parse_ctrl_shift_p() {
        let combo = parse_combo("Ctrl+Shift+P").unwrap();
        assert_eq!(combo.modifiers.len(), 2);
        assert!(key_eq(&combo.modifiers[0], &Key::Control));
        assert!(key_eq(&combo.modifiers[1], &Key::Shift));
        assert!(key_eq(&combo.key, &Key::Unicode('P')));
    }

    #[test]
    fn test_parse_named_key_enter() {
        let combo = parse_combo("Enter").unwrap();
        assert!(key_eq(&combo.key, &Key::Return));
    }

    #[test]
    fn test_parse_function_key_f5() {
        let combo = parse_combo("F5").unwrap();
        assert!(key_eq(&combo.key, &Key::F5));
    }

    #[test]
    fn test_parse_arrow_key() {
        let combo = parse_combo("ArrowUp").unwrap();
        assert!(key_eq(&combo.key, &Key::UpArrow));
    }

    #[test]
    fn test_parse_meta_aliases_resolve_to_meta() {
        for alias in ["Meta+a", "Cmd+a", "Super+a", "Win+a", "Command+a"] {
            let combo = parse_combo(alias).unwrap();
            assert!(key_eq(&combo.modifiers[0], &Key::Meta), "alias: {alias}");
        }
    }

    #[test]
    fn test_parse_dash_separator_accepted() {
        let combo = parse_combo("Ctrl-Shift-P").unwrap();
        assert_eq!(combo.modifiers.len(), 2);
    }

    #[test]
    fn test_parse_case_insensitive_modifiers() {
        let combo = parse_combo("CONTROL+a").unwrap();
        assert!(key_eq(&combo.modifiers[0], &Key::Control));
    }

    #[test]
    fn test_parse_empty_returns_error() {
        assert!(matches!(parse_combo(""), Err(KeyError::Empty)));
        assert!(matches!(parse_combo("   "), Err(KeyError::Empty)));
        assert!(matches!(parse_combo("+++"), Err(KeyError::Empty)));
    }

    #[test]
    fn test_parse_unknown_modifier_returns_error() {
        assert!(matches!(
            parse_combo("Hyper+a"),
            Err(KeyError::UnknownKey(_))
        ));
    }

    #[test]
    fn test_parse_unknown_key_returns_error() {
        assert!(matches!(
            parse_combo("Ctrl+NotAKey"),
            Err(KeyError::UnknownKey(_))
        ));
    }
}
