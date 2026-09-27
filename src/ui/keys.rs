//! Configurable key bindings.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;

/// Everything a key can do in the TUI.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Quit,
    Help,
    Filter,
    Mark,
    Trash,
    ClearMarks,
    Rescan,
    Mode,
    Hidden,
    Apparent,
    Reset,
    DepthDec,
    DepthInc,
    ZoomIn,
    ZoomOut,
    ColorMode,
    Up,
    Down,
    Left,
    Right,
}

impl Action {
    pub fn from_name(name: &str) -> Option<Action> {
        Some(
            match name.trim().to_ascii_lowercase().replace('-', "_").as_str() {
                "quit" => Action::Quit,
                "help" => Action::Help,
                "filter" => Action::Filter,
                "mark" => Action::Mark,
                "trash" | "delete" => Action::Trash,
                "clear_marks" => Action::ClearMarks,
                "rescan" => Action::Rescan,
                "mode" => Action::Mode,
                "colors" | "color_mode" => Action::ColorMode,
                "hidden" => Action::Hidden,
                "apparent" => Action::Apparent,
                "reset" => Action::Reset,
                "depth_dec" => Action::DepthDec,
                "depth_inc" => Action::DepthInc,
                "zoom_in" | "open" => Action::ZoomIn,
                "zoom_out" | "up_level" => Action::ZoomOut,
                "up" => Action::Up,
                "down" => Action::Down,
                "left" => Action::Left,
                "right" => Action::Right,
                _ => return None,
            },
        )
    }
}

/// A key-to-action lookup built from defaults plus config overrides.
pub struct KeyMap {
    bindings: Vec<(KeyCode, KeyModifiers, Action)>,
}

impl KeyMap {
    pub fn default_map() -> KeyMap {
        KeyMap {
            bindings: DEFAULT_BINDINGS.to_vec(),
        }
    }

    /// Build a map from defaults, replacing the bindings of every action
    /// listed in `overrides`.
    pub fn with_overrides(overrides: &BTreeMap<String, Vec<String>>) -> Result<KeyMap, String> {
        let mut map = KeyMap::default_map();
        for (action_name, keys) in overrides {
            let action = Action::from_name(action_name)
                .ok_or_else(|| format!("unknown key action {action_name:?}"))?;
            map.bindings.retain(|b| b.2 != action);
            for key in keys {
                let (code, mods) = parse_key(key)?;
                map.bindings.retain(|b| !(b.0 == code && b.1 == mods));
                map.bindings.push((code, mods, action));
            }
        }
        Ok(map)
    }

    pub fn action(&self, key: KeyEvent) -> Option<Action> {
        let (code, mods) = normalize(key.code, key.modifiers);
        self.bindings
            .iter()
            .find(|(c, m, _)| *c == code && *m == mods)
            .map(|(_, _, action)| *action)
    }
}

/// Shift is implied by uppercase character keys.
fn normalize(code: KeyCode, mods: KeyModifiers) -> (KeyCode, KeyModifiers) {
    let mut mods = mods;
    if matches!(code, KeyCode::Char(_)) {
        mods.remove(KeyModifiers::SHIFT);
    }
    (code, mods)
}

fn parse_key(spec: &str) -> Result<(KeyCode, KeyModifiers), String> {
    // Modifiers are separated by `+` or `-`; a bare `+` or `-` is the key
    // itself.
    if spec.len() > 1 && (spec.ends_with('+') || spec.ends_with('-')) {
        return Err(format!("incomplete key binding {spec:?}"));
    }
    let parts: Vec<&str> = if spec == "+" || spec == "-" {
        vec![spec]
    } else {
        spec.split(['+', '-'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect()
    };
    let Some((last, mod_parts)) = parts.split_last() else {
        return Err(format!("empty key binding {spec:?}"));
    };
    let mut mods = KeyModifiers::NONE;
    for part in mod_parts {
        match part.to_ascii_lowercase().as_str() {
            "c" | "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "s" | "shift" => mods |= KeyModifiers::SHIFT,
            "a" | "alt" => mods |= KeyModifiers::ALT,
            "super" | "cmd" | "meta" => mods |= KeyModifiers::SUPER,
            other => return Err(format!("unknown modifier {other:?} in key {spec:?}")),
        }
    }
    let lower = last.to_ascii_lowercase();
    let code = match lower.as_str() {
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        f if f.len() >= 2 && f.starts_with('f') => {
            let n: u8 = f[1..]
                .parse()
                .map_err(|_| format!("unknown key {spec:?}"))?;
            if !(1..=12).contains(&n) {
                return Err(format!("unknown key {spec:?}"));
            }
            KeyCode::F(n)
        }
        _ => {
            let mut chars = last.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => return Err(format!("unknown key {spec:?}")),
            }
        }
    };
    Ok((code, mods))
}

const NONE: KeyModifiers = KeyModifiers::NONE;

const DEFAULT_BINDINGS: &[(KeyCode, KeyModifiers, Action)] = &[
    (KeyCode::Char('q'), NONE, Action::Quit),
    (KeyCode::Char('?'), NONE, Action::Help),
    (KeyCode::Char('/'), NONE, Action::Filter),
    (KeyCode::Char(' '), NONE, Action::Mark),
    (KeyCode::Char('d'), NONE, Action::Trash),
    (KeyCode::Char('c'), NONE, Action::ClearMarks),
    (KeyCode::Char('r'), NONE, Action::Rescan),
    (KeyCode::Char('t'), NONE, Action::Mode),
    (KeyCode::Char('m'), NONE, Action::ColorMode),
    (KeyCode::Char('H'), NONE, Action::Hidden),
    (KeyCode::Char('a'), NONE, Action::Apparent),
    (KeyCode::Char('0'), NONE, Action::Reset),
    (KeyCode::Char('['), NONE, Action::DepthDec),
    (KeyCode::Char(']'), NONE, Action::DepthInc),
    (KeyCode::Enter, NONE, Action::ZoomIn),
    (KeyCode::Char('o'), NONE, Action::ZoomIn),
    (KeyCode::Backspace, NONE, Action::ZoomOut),
    (KeyCode::Char('u'), NONE, Action::ZoomOut),
    (KeyCode::Up, NONE, Action::Up),
    (KeyCode::Char('k'), NONE, Action::Up),
    (KeyCode::Down, NONE, Action::Down),
    (KeyCode::Char('j'), NONE, Action::Down),
    (KeyCode::Left, NONE, Action::Left),
    (KeyCode::Char('h'), NONE, Action::Left),
    (KeyCode::Right, NONE, Action::Right),
    (KeyCode::Char('l'), NONE, Action::Right),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys() {
        assert_eq!(parse_key("q").unwrap(), (KeyCode::Char('q'), NONE));
        assert_eq!(parse_key("H").unwrap(), (KeyCode::Char('H'), NONE));
        assert_eq!(
            parse_key("C-c").unwrap(),
            (KeyCode::Char('c'), KeyModifiers::CONTROL)
        );
        assert_eq!(parse_key("enter").unwrap(), (KeyCode::Enter, NONE));
        assert_eq!(parse_key("F5").unwrap(), (KeyCode::F(5), NONE));
        assert_eq!(parse_key("space").unwrap(), (KeyCode::Char(' '), NONE));
        assert!(parse_key("nope").is_err());
        assert!(parse_key("C-").is_err());
    }

    #[test]
    fn resolves_default_bindings() {
        let map = KeyMap::default_map();
        let key = KeyEvent::new(KeyCode::Char('t'), NONE);
        assert_eq!(map.action(key), Some(Action::Mode));
        let shifted = KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT);
        assert_eq!(map.action(shifted), Some(Action::Hidden));
    }

    #[test]
    fn applies_overrides() {
        let mut overrides = BTreeMap::new();
        overrides.insert("quit".to_string(), vec!["x".to_string(), "C-q".to_string()]);
        let map = KeyMap::with_overrides(&overrides).unwrap();
        assert_eq!(
            map.action(KeyEvent::new(KeyCode::Char('x'), NONE)),
            Some(Action::Quit)
        );
        assert_eq!(map.action(KeyEvent::new(KeyCode::Char('q'), NONE)), None);
        assert!(
            KeyMap::with_overrides(&BTreeMap::from([(
                "nope".to_string(),
                vec!["x".to_string()]
            )]))
            .is_err()
        );
    }
}
