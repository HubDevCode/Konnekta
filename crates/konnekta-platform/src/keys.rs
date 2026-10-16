use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecialKey {
    Backspace,
    Tab,
    Return,
    Escape,
    Space,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Right,
    Up,
    Down,
    CapsLock,
    F(u8),
    MediaPlayPause,
    MediaNext,
    MediaPrevious,
    MediaStop,
    VolumeUp,
    VolumeDown,
    VolumeMute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Key {
    Special(SpecialKey),
    Char(char),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modifiers {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    /// Super / Windows / Command.
    #[serde(default)]
    pub meta: bool,
}

impl Modifiers {
    pub fn any(&self) -> bool {
        self.ctrl || self.shift || self.alt || self.meta
    }
}

/// Parse shortcuts like `ctrl+shift+t`, `alt+F4`, `meta+Left`, `PageDown`.
pub fn parse_shortcut(s: &str) -> Result<(Modifiers, Key)> {
    let mut mods = Modifiers::default();
    let mut key = None;
    for part in s.split('+').map(str::trim) {
        if part.is_empty() {
            bail!("invalid shortcut '{s}'");
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "shift" => mods.shift = true,
            "alt" | "option" => mods.alt = true,
            "meta" | "super" | "win" | "cmd" | "command" => mods.meta = true,
            other => {
                if key.is_some() {
                    bail!("shortcut '{s}' has more than one key");
                }
                key = Some(parse_key(other).ok_or_else(|| anyhow::anyhow!("unknown key '{part}'"))?);
            }
        }
    }
    Ok((mods, key.ok_or_else(|| anyhow::anyhow!("shortcut '{s}' has no key"))?))
}

fn parse_key(k: &str) -> Option<Key> {
    use SpecialKey::*;
    let sp = match k {
        "backspace" => Backspace,
        "tab" => Tab,
        "enter" | "return" => Return,
        "esc" | "escape" => Escape,
        "space" => Space,
        "delete" | "del" => Delete,
        "insert" | "ins" => Insert,
        "home" => Home,
        "end" => End,
        "pageup" | "pgup" => PageUp,
        "pagedown" | "pgdn" => PageDown,
        "left" => Left,
        "right" => Right,
        "up" => Up,
        "down" => Down,
        "capslock" => CapsLock,
        "playpause" | "mediaplaypause" => MediaPlayPause,
        "next" | "medianext" => MediaNext,
        "previous" | "prev" | "mediaprevious" => MediaPrevious,
        "stop" | "mediastop" => MediaStop,
        "volumeup" => VolumeUp,
        "volumedown" => VolumeDown,
        "mute" | "volumemute" => VolumeMute,
        f if f.len() >= 2 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => {
            let n: u8 = f[1..].parse().ok()?;
            if !(1..=24).contains(&n) {
                return None;
            }
            F(n)
        }
        c if c.chars().count() == 1 => return Some(Key::Char(c.chars().next()?)),
        _ => return None,
    };
    Some(Key::Special(sp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts() {
        let (m, k) = parse_shortcut("ctrl+shift+t").unwrap();
        assert!(m.ctrl && m.shift && !m.alt);
        assert_eq!(k, Key::Char('t'));
        let (m, k) = parse_shortcut("Alt+F4").unwrap();
        assert!(m.alt);
        assert_eq!(k, Key::Special(SpecialKey::F(4)));
        assert_eq!(parse_shortcut("PageDown").unwrap().1, Key::Special(SpecialKey::PageDown));
        assert!(parse_shortcut("ctrl+").is_err());
        assert!(parse_shortcut("a+b").is_err());
        assert!(parse_shortcut("ctrl").is_err());
        assert!(parse_shortcut("f99").is_err());
    }
}
