//! Native OS-level keyboard event injection.
//!
//! Synthetic `KeyboardEvent`s dispatched from JS are flagged `isTrusted: false`
//! and never reach the OS / window-manager layer where Tauri accelerators are
//! registered. Injecting at the OS layer (via [`enigo`]) produces "real" key
//! events that traverse the full pipeline:
//!
//! ```text
//! enigo → OS input subsystem → window manager → toolkit (GTK/AppKit/Win32)
//!       → webview → DOM listeners
//!       → Tauri accelerator handlers
//! ```
//!
//! # Platform caveats (X11 global shortcuts)
//!
//! `tauri-plugin-global-shortcut` registers its accelerators through the
//! `global-hotkey` crate, which on X11 uses `XGrabKey` passive grabs keyed on a
//! *physical* keycode (e.g. `Code::Digit1` → keycode 10, derived from the evdev
//! scancode and independent of the active layout).
//!
//! `enigo` injects a [`Key::Unicode`] character by looking its keysym up in the
//! keymap, but only at shift-level 0. On a layout where the wanted character
//! sits at a higher level — most importantly the digit row on AZERTY and
//! similar layouts, where `1` is `Shift`+`&` — enigo cannot find it on the
//! physical key and instead remaps it onto a spare keycode. That spare keycode
//! never matches a physical-key grab, so `Control+1`-style global shortcuts
//! silently fail to fire even though the same key still reaches DOM listeners
//! as a trusted event.
//!
//! The same trap catches an uppercase *letter* written into a `Shift` combo.
//! `Control+Shift+P` parses to the pre-shifted character `'P'`, whose keysym
//! lives at shift-level 1; on AZERTY enigo cannot place it on the physical key
//! and remaps it onto a spare keycode, so the grab never fires (issue #121).
//! Lowercase `p` sits at level 0, which is why `Control+Shift+p` works.
//!
//! Two injection-path normalisations keep modified combos hitting the grab
//! regardless of layout:
//!
//! - A digit in a modified combo (e.g. `Control+1`) is injected as a raw
//!   physical keycode (see [`physical_digit_keycode`] / [`tap_main_key`]) — the
//!   exact keycode `global-hotkey` grabbed.
//! - An ASCII-uppercase letter in a modified combo is lowered to its base key
//!   (see [`normalize_main_key`]); the explicitly held `Shift` then yields the
//!   uppercase character, matching the grab at level 0.
//!
//! A bare `press "1"` or `press "P"` keeps enigo's layout-aware `Key::Unicode`
//! path, so it still types the layout's character rather than the unshifted
//! physical key (`&` on AZERTY); reach for `fill` to set input text. Other
//! characters that live above shift-level 0 on an exotic layout may still be
//! remapped and miss a physical-key grab.
//!
//! See issues #45, #75, #114 and #121.
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::sync::Mutex;
use thiserror::Error;

/// Serializes all OS-level key injections from this process. Concurrent calls
/// to `simulate_press` from multiple tokio tasks would otherwise interleave
/// modifier-down/up events on the libei/uinput backends, producing scrambled
/// shortcuts. The lock is held only for the duration of one combo (a few ms).
static PRESS_LOCK: Mutex<()> = Mutex::new(());

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
/// `+` is the only separator. Trailing `+` is interpreted as the literal `+`
/// key (e.g. `"Control++"` == Control plus the `+` key), and `"+"` alone is
/// just the `+` key. This keeps combos like `"Shift+-"` (Shift + minus)
/// unambiguous.
///
/// Empty segments between `+` separators are rejected — `"Control++P"` and
/// `"+A"` are errors, not silently normalized into `"Control+P"` / `"A"`.
pub fn parse_combo(combo: &str) -> Result<Combo, KeyError> {
    let trimmed = combo.trim();
    if trimmed.is_empty() {
        return Err(KeyError::Empty);
    }
    if trimmed == "+" {
        return Ok(Combo {
            modifiers: Vec::new(),
            key: Key::Unicode('+'),
        });
    }

    // Determine the modifier section and the main key token. A trailing `+`
    // means "the main key is `+`"; the `+` immediately before it is the
    // separator between the last modifier (if any) and that key.
    let (modifier_section, main) = if let Some(prefix) = trimmed.strip_suffix('+') {
        match prefix.strip_suffix('+') {
            Some(body) => (body, "+"),
            // `"abc+"` with no separator `+` before — malformed.
            None if !prefix.is_empty() => {
                return Err(KeyError::UnknownKey(combo.trim().to_owned()));
            }
            None => (prefix, "+"),
        }
    } else {
        match trimmed.rsplit_once('+') {
            // A leading `+` (e.g. `"+A"`) produces `mods = ""` while a `+`
            // actually exists in the input — reject instead of silently
            // treating it as "no modifiers".
            Some(("", _)) => {
                return Err(KeyError::UnknownKey(combo.trim().to_owned()));
            }
            Some((mods, k)) => (mods, k.trim()),
            None => ("", trimmed),
        }
    };

    let modifiers = if modifier_section.is_empty() {
        Vec::new()
    } else {
        modifier_section
            .split('+')
            .map(|tok| {
                let trimmed_tok = tok.trim();
                if trimmed_tok.is_empty() {
                    // An empty segment between separators is a typo, not a
                    // modifier — reject rather than silently collapsing.
                    Err(KeyError::UnknownKey(combo.trim().to_owned()))
                } else {
                    parse_modifier(trimmed_tok)
                }
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    if main.is_empty() {
        return Err(KeyError::Empty);
    }
    let key = parse_key(main)?;
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

/// Physical X11/xkb keycodes for the main number-row digits.
///
/// These are layout-independent: X11 derives them from the Linux evdev
/// scancodes (`KEY_1`..`KEY_0`) offset by 8, so `1` is always keycode 10 and
/// `0` is keycode 19 — whether the active layout puts the digit on an unshifted
/// (US QWERTY) or a shifted (e.g. French AZERTY) level. They are exactly the
/// keycodes `global-hotkey` grabs for `Code::Digit1`..`Code::Digit0`.
#[cfg(target_os = "linux")]
fn linux_digit_keycode(ch: char) -> Option<u16> {
    match ch {
        '1'..='9' => Some(10 + (ch as u16 - '1' as u16)),
        '0' => Some(19),
        _ => None,
    }
}

/// The raw physical keycode to inject for the combo's main key, or `None` to
/// use enigo's layout-aware [`Key`] path.
///
/// Returns a keycode only for a digit that is part of a *modified* combo (e.g.
/// `Control+1`): that is the case targeting a physical-key grab, where matching
/// `global-hotkey`'s keycode matters more than the character produced. A bare
/// digit press returns `None` so `press "1"` keeps typing the layout's digit
/// (`1` on AZERTY) instead of the unshifted physical key (`&`).
#[cfg(target_os = "linux")]
fn physical_digit_keycode(key: Key, has_modifiers: bool) -> Option<u16> {
    match key {
        Key::Unicode(ch) if has_modifiers => linux_digit_keycode(ch),
        _ => None,
    }
}

/// Normalise the combo's main key for injection under any currently-held
/// modifiers.
///
/// When a modifier is held, an ASCII-uppercase letter main key (e.g. `'P'` from
/// `Control+Shift+P`) is lowered to its base form so enigo resolves it at
/// shift-level 0 — the physical key the X11 grab is keyed on. The explicitly
/// held `Shift` then produces the uppercase character, exactly as a human
/// pressing the key would. Without this, enigo looks the uppercase keysym up at
/// shift-level 1, fails to place it on the physical key on layouts like AZERTY,
/// and remaps it onto a spare keycode that matches no grab — the press is lost
/// even though the exit status is `0` (issue #121).
///
/// A bare `press "P"` (no modifiers) keeps `Key::Unicode('P')` so enigo applies
/// its own shift and types an uppercase `P`, mirroring how a bare digit keeps
/// the layout-aware path rather than a raw physical keycode. Non-ASCII
/// uppercase letters are left untouched — only `A`–`Z` are handled.
///
/// Linux-only, like the digit raw-keycode path: macOS and Windows resolve a
/// `Key::Unicode` character through their own keymap lookups that already land
/// on the physical key regardless of letter case, so they keep enigo's plain
/// `Key` path unchanged.
#[cfg(target_os = "linux")]
fn normalize_main_key(key: Key, has_modifiers: bool) -> Key {
    match key {
        Key::Unicode(ch) if has_modifiers && ch.is_ascii_uppercase() => {
            Key::Unicode(ch.to_ascii_lowercase())
        }
        _ => key,
    }
}

/// Tap (press + release) the combo's main key, given whether any modifier is
/// currently held.
///
/// On Linux a modified digit combo is injected as a raw physical keycode (see
/// [`physical_digit_keycode`]) so it reaches the exact physical key that
/// `global-hotkey` grabs. enigo's layout-aware [`Key::Unicode`] path resolves a
/// digit's keysym only at shift-level 0; on layouts where the digit is shifted
/// it fails to find it on the physical key and remaps it onto a spare keycode,
/// which no physical-key grab can match. Bare digits and every other key — and
/// every key on macOS/Windows — go through enigo's `Key` path unchanged.
#[cfg(target_os = "linux")]
fn tap_main_key(enigo: &mut Enigo, key: Key, has_modifiers: bool) -> Result<(), KeyError> {
    if let Some(keycode) = physical_digit_keycode(key, has_modifiers) {
        return enigo
            .raw(keycode, Direction::Click)
            .map_err(|e| KeyError::EnigoInput(e.to_string()));
    }
    enigo
        .key(normalize_main_key(key, has_modifiers), Direction::Click)
        .map_err(|e| KeyError::EnigoInput(e.to_string()))
}

/// macOS and Windows have no layout-dependent key-remapping problem (their
/// keymap lookups resolve a `Key::Unicode` character to the physical key
/// regardless of case or shift level), so every key goes through enigo's
/// layout-aware [`Key`] path.
#[cfg(not(target_os = "linux"))]
fn tap_main_key(enigo: &mut Enigo, key: Key, _has_modifiers: bool) -> Result<(), KeyError> {
    enigo
        .key(key, Direction::Click)
        .map_err(|e| KeyError::EnigoInput(e.to_string()))
}

/// Press `combo` at the OS level. Modifiers are pressed in order, then the
/// main key is tapped (press+release), then modifiers are released in reverse
/// order — the same pattern a human or Playwright would produce.
///
/// If a modifier press or the key tap fails, every modifier successfully
/// pressed so far is released before returning, so the OS is never left with
/// a stuck modifier (e.g. permanent Shift). On macOS, when Enigo silently
/// no-ops because Accessibility permission is missing, the first failing call
/// returns an error; the `EnigoInput` message includes a hint pointing at the
/// permission.
///
/// All callers serialize through a process-global lock so two concurrent
/// `press` RPC calls never interleave their modifier-down/key/modifier-up
/// sequences. This is a blocking OS call; async callers should wrap with
/// `tokio::task::spawn_blocking` to avoid stalling the runtime.
pub fn simulate_press(combo: &str) -> Result<(), KeyError> {
    let parsed = parse_combo(combo)?;
    let _guard = PRESS_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| {
        // The Accessibility permission hint only applies to macOS — on Linux
        // (libei/X11) and Windows the remediation is different, so don't
        // point users at the wrong fix.
        #[cfg(target_os = "macos")]
        let msg =
            format!("{e} (on macOS, grant Accessibility permission to the launching terminal)");
        #[cfg(not(target_os = "macos"))]
        let msg = e.to_string();
        KeyError::EnigoInit(msg)
    })?;

    // Track how many modifiers were pressed so we can release exactly those
    // on any failure path — including failures during the modifier press loop.
    let mut pressed: Vec<Key> = Vec::with_capacity(parsed.modifiers.len());
    let press_outcome = (|| -> Result<(), KeyError> {
        for m in &parsed.modifiers {
            enigo
                .key(*m, Direction::Press)
                .map_err(|e| KeyError::EnigoInput(e.to_string()))?;
            pressed.push(*m);
        }
        tap_main_key(&mut enigo, parsed.key, !parsed.modifiers.is_empty())
    })();

    // Always release the modifiers we actually pressed, in reverse order.
    // We keep going after a release failure so subsequent modifiers still
    // get a chance to come back up — but we remember the first error so we
    // don't return Ok with a modifier potentially stuck down.
    let mut release_error: Option<KeyError> = None;
    for m in pressed.iter().rev() {
        if let Err(e) = enigo.key(*m, Direction::Release)
            && release_error.is_none()
        {
            release_error = Some(KeyError::EnigoInput(format!(
                "modifier release failed (possible stuck key): {e}"
            )));
        }
    }

    // If the press itself failed, report that (more actionable than a
    // downstream release error). Otherwise surface any release failure so
    // callers learn about a stuck modifier instead of seeing Ok(()).
    press_outcome?;
    match release_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_eq(a: Key, b: Key) -> bool {
        // Key doesn't impl PartialEq for all variants in older versions; format!-compare.
        format!("{a:?}") == format!("{b:?}")
    }

    #[test]
    fn test_parse_single_char_returns_unicode() {
        let combo = parse_combo("a").expect("parse_combo succeeds");
        assert!(combo.modifiers.is_empty());
        assert!(key_eq(combo.key, Key::Unicode('a')));
    }

    #[test]
    fn test_parse_ctrl_plus_one() {
        let combo = parse_combo("Control+1").expect("parse_combo succeeds");
        assert_eq!(combo.modifiers.len(), 1);
        assert!(key_eq(combo.modifiers[0], Key::Control));
        assert!(key_eq(combo.key, Key::Unicode('1')));
    }

    #[test]
    fn test_parse_ctrl_shift_p() {
        let combo = parse_combo("Ctrl+Shift+P").expect("parse_combo succeeds");
        assert_eq!(combo.modifiers.len(), 2);
        assert!(key_eq(combo.modifiers[0], Key::Control));
        assert!(key_eq(combo.modifiers[1], Key::Shift));
        assert!(key_eq(combo.key, Key::Unicode('P')));
    }

    #[test]
    fn test_parse_named_key_enter() {
        let combo = parse_combo("Enter").expect("parse_combo succeeds");
        assert!(key_eq(combo.key, Key::Return));
    }

    #[test]
    fn test_parse_function_key_f5() {
        let combo = parse_combo("F5").expect("parse_combo succeeds");
        assert!(key_eq(combo.key, Key::F5));
    }

    #[test]
    fn test_parse_arrow_key() {
        let combo = parse_combo("ArrowUp").expect("parse_combo succeeds");
        assert!(key_eq(combo.key, Key::UpArrow));
    }

    #[test]
    fn test_parse_meta_aliases_resolve_to_meta() {
        for alias in ["Meta+a", "Cmd+a", "Super+a", "Win+a", "Command+a"] {
            let combo = parse_combo(alias).expect("parse_combo succeeds");
            assert!(key_eq(combo.modifiers[0], Key::Meta), "alias: {alias}");
        }
    }

    #[test]
    fn test_parse_dash_is_treated_as_minus_key() {
        // `-` is no longer a separator (review #45 finding 3): "Shift+-" must
        // parse as Shift + the literal `-` key, not as nonsense.
        let combo = parse_combo("Shift+-").expect("parse_combo succeeds");
        assert_eq!(combo.modifiers.len(), 1);
        assert!(key_eq(combo.modifiers[0], Key::Shift));
        assert!(key_eq(combo.key, Key::Unicode('-')));
    }

    #[test]
    fn test_parse_plus_alone_is_plus_key() {
        let combo = parse_combo("+").expect("parse_combo succeeds");
        assert!(combo.modifiers.is_empty());
        assert!(key_eq(combo.key, Key::Unicode('+')));
    }

    #[test]
    fn test_parse_trailing_plus_is_plus_key_with_modifiers() {
        let combo = parse_combo("Control++").expect("parse_combo succeeds");
        assert_eq!(combo.modifiers.len(), 1);
        assert!(key_eq(combo.modifiers[0], Key::Control));
        assert!(key_eq(combo.key, Key::Unicode('+')));
    }

    #[test]
    fn test_parse_case_insensitive_modifiers() {
        let combo = parse_combo("CONTROL+a").expect("parse_combo succeeds");
        assert!(key_eq(combo.modifiers[0], Key::Control));
    }

    #[test]
    fn test_parse_empty_returns_error() {
        assert!(matches!(parse_combo(""), Err(KeyError::Empty)));
        assert!(matches!(parse_combo("   "), Err(KeyError::Empty)));
    }

    #[test]
    fn test_parse_triple_plus_returns_error() {
        // `"+++"` has an empty modifier segment once the trailing `+` is
        // stripped off as the main key. Rejecting is safer than silently
        // collapsing: if a user typed three `+`, something is wrong.
        assert!(matches!(parse_combo("+++"), Err(KeyError::UnknownKey(_))));
    }

    #[test]
    fn test_parse_empty_modifier_segment_returns_error() {
        // `"Control++P"` previously parsed as `"Control+P"` because the empty
        // segment between the two `+` was silently dropped. That turns typos
        // into different shortcuts — reject instead.
        assert!(matches!(
            parse_combo("Control++P"),
            Err(KeyError::UnknownKey(_))
        ));
    }

    #[test]
    fn test_parse_leading_plus_returns_error() {
        // `"+A"` previously parsed as just `"A"` — the leading `+` was
        // discarded. Same reasoning: silent normalization hides typos.
        assert!(matches!(parse_combo("+A"), Err(KeyError::UnknownKey(_))));
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

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_digit_keycode_maps_digits_to_physical_x11_keycodes() {
        // evdev KEY_1..KEY_9 are X keycodes 10..18 and KEY_0 is 19 — the exact
        // keycodes `global-hotkey` grabs for Code::Digit1..Digit9 / Digit0.
        // Injecting these as raw keycodes is what fixes Control+digit global
        // shortcuts on layouts (e.g. AZERTY) where the digit is shifted (#114).
        assert_eq!(linux_digit_keycode('1'), Some(10));
        assert_eq!(linux_digit_keycode('2'), Some(11));
        assert_eq!(linux_digit_keycode('5'), Some(14));
        assert_eq!(linux_digit_keycode('9'), Some(18));
        assert_eq!(linux_digit_keycode('0'), Some(19));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_linux_digit_keycode_returns_none_for_non_digits() {
        // Letters and symbols keep enigo's layout-aware Key::Unicode path;
        // only main-row digits are diverted to a raw physical keycode.
        for ch in ['a', 'A', 'p', '+', '-', ' ', 'é'] {
            assert_eq!(linux_digit_keycode(ch), None, "char: {ch}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_physical_digit_keycode_used_for_modified_digit_combos() {
        // A modified digit combo (e.g. Control+1) targets a physical-key grab,
        // so it is injected as the raw physical keycode global-hotkey grabbed.
        assert_eq!(physical_digit_keycode(Key::Unicode('1'), true), Some(10));
        assert_eq!(physical_digit_keycode(Key::Unicode('0'), true), Some(19));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_physical_digit_keycode_skips_bare_digit_for_character_entry() {
        // A bare `press "1"` keeps the layout-aware Key::Unicode path so it
        // types the layout's digit (`1` on AZERTY) rather than the unshifted
        // physical key (`&`). Only modified combos divert to a raw keycode.
        assert_eq!(physical_digit_keycode(Key::Unicode('1'), false), None);
        assert_eq!(physical_digit_keycode(Key::Unicode('9'), false), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_physical_digit_keycode_skips_non_digit_keys() {
        // Letters and named keys always use enigo's layout-aware path, even in
        // a modified combo — they resolve correctly at shift-level 0.
        assert_eq!(physical_digit_keycode(Key::Unicode('a'), true), None);
        assert_eq!(physical_digit_keycode(Key::Return, true), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_normalize_main_key_lowercases_modified_uppercase_letter() {
        // `Control+Shift+P` parses to Key::Unicode('P'); enigo resolves the
        // uppercase keysym only at shift-level 1, so on AZERTY it remaps to a
        // spare keycode no XGrabKey grab matches (#121). Tapping the base
        // physical key ('p', level 0) under the held Shift hits the grab.
        assert!(key_eq(
            normalize_main_key(Key::Unicode('P'), true),
            Key::Unicode('p')
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_normalize_main_key_keeps_bare_uppercase_letter() {
        // A bare `press "P"` (no modifiers) means "type uppercase P" — keep the
        // pre-shifted character so enigo applies its own shift, mirroring how a
        // bare digit keeps the layout-aware Key::Unicode path.
        assert!(key_eq(
            normalize_main_key(Key::Unicode('P'), false),
            Key::Unicode('P')
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_normalize_main_key_keeps_lowercase_letter() {
        // Lowercase already resolves at shift-level 0 — untouched in either case.
        assert!(key_eq(
            normalize_main_key(Key::Unicode('p'), true),
            Key::Unicode('p')
        ));
        assert!(key_eq(
            normalize_main_key(Key::Unicode('p'), false),
            Key::Unicode('p')
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_normalize_main_key_keeps_non_letter_keys() {
        // Digits and named keys are not ASCII-uppercase letters, so they pass
        // through unchanged — digits go through the raw-keycode path instead.
        assert!(key_eq(
            normalize_main_key(Key::Unicode('1'), true),
            Key::Unicode('1')
        ));
        assert!(key_eq(
            normalize_main_key(Key::Unicode('+'), true),
            Key::Unicode('+')
        ));
        assert!(key_eq(normalize_main_key(Key::Return, true), Key::Return));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_normalize_main_key_keeps_non_ascii_uppercase() {
        // Only ASCII A–Z are handled (the case enigo can resolve from the held
        // Shift). A non-ASCII uppercase letter is left as-is rather than guessed.
        assert!(key_eq(
            normalize_main_key(Key::Unicode('É'), true),
            Key::Unicode('É')
        ));
    }
}
