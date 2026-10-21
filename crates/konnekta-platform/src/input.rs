//! Mouse and keyboard injection.
//!
//! Native injection runs on a dedicated thread (some OS handles are not
//! `Send`) fed by a channel, which also preserves event ordering.
//! On Linux/Wayland we prefer `ydotool` when available, otherwise enigo's
//! Wayland virtual-input protocols / X11 backend.

use crate::keys::{Key, Modifiers};
use anyhow::Result;
use serde::{Deserialize, Serialize};
#[cfg(feature = "desktop")]
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ButtonAction {
    Click,
    DoubleClick,
    Press,
    Release,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputEvent {
    MoveRelative {
        dx: i32,
        dy: i32,
    },
    /// Normalised coordinates on the primary display (0.0..=1.0).
    MoveAbsolute {
        x: f64,
        y: f64,
    },
    Button {
        button: MouseButton,
        action: ButtonAction,
    },
    Scroll {
        dx: i32,
        dy: i32,
    },
    Text {
        text: String,
    },
    Key {
        key: Key,
        modifiers: Modifiers,
    },
}

pub trait InputBackend: Send + Sync {
    fn send(&self, ev: InputEvent) -> Result<()>;
}

#[cfg(feature = "desktop")]
pub fn native() -> Arc<dyn InputBackend> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && crate::cmd::exists("ydotool")
        && std::env::var("KONNEKTA_INPUT").as_deref() != Ok("enigo")
    {
        return Arc::new(ydotool::Ydotool);
    }
    Arc::new(enigo_backend::EnigoInput::spawn())
}

#[cfg(feature = "desktop")]
mod enigo_backend {
    use super::*;
    use crate::keys::SpecialKey;
    use enigo::{Axis, Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
    use std::sync::mpsc;

    pub struct EnigoInput {
        tx: mpsc::Sender<InputEvent>,
    }

    impl EnigoInput {
        pub fn spawn() -> Self {
            let (tx, rx) = mpsc::channel::<InputEvent>();
            std::thread::Builder::new()
                .name("konnekta-input".into())
                .spawn(move || {
                    let mut enigo = None;
                    while let Ok(ev) = rx.recv() {
                        if enigo.is_none() {
                            match Enigo::new(&Settings::default()) {
                                Ok(e) => enigo = Some(e),
                                Err(e) => {
                                    tracing::error!("input injection unavailable: {e}");
                                    continue;
                                }
                            }
                        }
                        if let Err(e) = apply(enigo.as_mut().unwrap(), ev) {
                            tracing::warn!("input event failed: {e}");
                        }
                    }
                })
                .expect("spawn input thread");
            Self { tx }
        }
    }

    impl InputBackend for EnigoInput {
        fn send(&self, ev: InputEvent) -> Result<()> {
            self.tx
                .send(ev)
                .map_err(|_| anyhow::anyhow!("input thread stopped"))
        }
    }

    fn button(b: MouseButton) -> Button {
        match b {
            MouseButton::Left => Button::Left,
            MouseButton::Right => Button::Right,
            MouseButton::Middle => Button::Middle,
        }
    }

    pub(super) fn map_key(k: Key) -> enigo::Key {
        use enigo::Key as E;
        match k {
            Key::Char(c) => E::Unicode(c),
            Key::Special(s) => match s {
                SpecialKey::Backspace => E::Backspace,
                SpecialKey::Tab => E::Tab,
                SpecialKey::Return => E::Return,
                SpecialKey::Escape => E::Escape,
                SpecialKey::Space => E::Space,
                SpecialKey::Delete => E::Delete,
                #[cfg(not(target_os = "macos"))]
                SpecialKey::Insert => E::Insert,
                #[cfg(target_os = "macos")]
                SpecialKey::Insert => E::Help,
                SpecialKey::Home => E::Home,
                SpecialKey::End => E::End,
                SpecialKey::PageUp => E::PageUp,
                SpecialKey::PageDown => E::PageDown,
                SpecialKey::Left => E::LeftArrow,
                SpecialKey::Right => E::RightArrow,
                SpecialKey::Up => E::UpArrow,
                SpecialKey::Down => E::DownArrow,
                SpecialKey::CapsLock => E::CapsLock,
                SpecialKey::F(n) => match n {
                    1 => E::F1,
                    2 => E::F2,
                    3 => E::F3,
                    4 => E::F4,
                    5 => E::F5,
                    6 => E::F6,
                    7 => E::F7,
                    8 => E::F8,
                    9 => E::F9,
                    10 => E::F10,
                    11 => E::F11,
                    _ => E::F12,
                },
                SpecialKey::MediaPlayPause => E::MediaPlayPause,
                SpecialKey::MediaNext => E::MediaNextTrack,
                SpecialKey::MediaPrevious => E::MediaPrevTrack,
                #[cfg(not(target_os = "macos"))]
                SpecialKey::MediaStop => E::MediaStop,
                #[cfg(target_os = "macos")]
                SpecialKey::MediaStop => E::MediaPlayPause,
                SpecialKey::VolumeUp => E::VolumeUp,
                SpecialKey::VolumeDown => E::VolumeDown,
                SpecialKey::VolumeMute => E::VolumeMute,
                SpecialKey::Super => E::Meta,
                SpecialKey::Control => E::Control,
                SpecialKey::Alt => E::Alt,
                SpecialKey::Shift => E::Shift,
            },
        }
    }

    fn apply(e: &mut Enigo, ev: InputEvent) -> Result<()> {
        match ev {
            InputEvent::MoveRelative { dx, dy } => e.move_mouse(dx, dy, Coordinate::Rel)?,
            InputEvent::MoveAbsolute { x, y } => {
                let (w, h) = e.main_display()?;
                let px = (x.clamp(0.0, 1.0) * f64::from(w - 1)).round() as i32;
                let py = (y.clamp(0.0, 1.0) * f64::from(h - 1)).round() as i32;
                e.move_mouse(px, py, Coordinate::Abs)?
            }
            InputEvent::Button { button: b, action } => {
                let b = button(b);
                match action {
                    ButtonAction::Click => e.button(b, Direction::Click)?,
                    ButtonAction::DoubleClick => {
                        e.button(b, Direction::Click)?;
                        e.button(b, Direction::Click)?
                    }
                    ButtonAction::Press => e.button(b, Direction::Press)?,
                    ButtonAction::Release => e.button(b, Direction::Release)?,
                }
            }
            InputEvent::Scroll { dx, dy } => {
                if dy != 0 {
                    e.scroll(dy, Axis::Vertical)?;
                }
                if dx != 0 {
                    e.scroll(dx, Axis::Horizontal)?;
                }
            }
            InputEvent::Text { text } => e.text(&text)?,
            InputEvent::Key { key, modifiers: m } => {
                let mods: Vec<enigo::Key> = [
                    (m.ctrl, enigo::Key::Control),
                    (m.shift, enigo::Key::Shift),
                    (m.alt, enigo::Key::Alt),
                    (m.meta, enigo::Key::Meta),
                ]
                .into_iter()
                .filter_map(|(on, k)| on.then_some(k))
                .collect();
                for k in &mods {
                    e.key(*k, Direction::Press)?;
                }
                let r = e.key(map_key(key), Direction::Click);
                for k in mods.iter().rev() {
                    e.key(*k, Direction::Release)?;
                }
                r?
            }
        }
        Ok(())
    }
}

/// `ydotool` backend for Wayland compositors without virtual-input protocols
/// (GNOME). Requires the `ydotoold` daemon.
#[cfg(all(feature = "desktop", target_os = "linux"))]
mod ydotool {
    use super::*;
    use crate::keys::SpecialKey;

    pub struct Ydotool;

    fn keycode(k: Key) -> Option<u16> {
        // Linux input-event-codes.h
        Some(match k {
            Key::Special(s) => match s {
                SpecialKey::Escape => 1,
                SpecialKey::Backspace => 14,
                SpecialKey::Tab => 15,
                SpecialKey::Return => 28,
                SpecialKey::Space => 57,
                SpecialKey::CapsLock => 58,
                SpecialKey::F(n @ 1..=10) => 58 + n as u16,
                SpecialKey::F(11) => 87,
                SpecialKey::F(_) => 88,
                SpecialKey::Home => 102,
                SpecialKey::Up => 103,
                SpecialKey::PageUp => 104,
                SpecialKey::Left => 105,
                SpecialKey::Right => 106,
                SpecialKey::End => 107,
                SpecialKey::Down => 108,
                SpecialKey::PageDown => 109,
                SpecialKey::Insert => 110,
                SpecialKey::Delete => 111,
                SpecialKey::VolumeMute => 113,
                SpecialKey::Super => 125,
                SpecialKey::Control => 29,
                SpecialKey::Alt => 56,
                SpecialKey::Shift => 42,
                SpecialKey::VolumeDown => 114,
                SpecialKey::VolumeUp => 115,
                SpecialKey::MediaNext => 163,
                SpecialKey::MediaPlayPause => 164,
                SpecialKey::MediaPrevious => 165,
                SpecialKey::MediaStop => 166,
            },
            Key::Char(c) => {
                let c = c.to_ascii_lowercase();
                let row = |s: &str, base: u16| s.find(c).map(|i| base + i as u16);
                row("1234567890", 2)
                    .or_else(|| row("qwertyuiop", 16))
                    .or_else(|| row("asdfghjkl", 30))
                    .or_else(|| row("zxcvbnm", 44))?
            }
        })
    }

    fn run(args: Vec<String>) {
        tokio::spawn(async move {
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            if let Err(e) = crate::cmd::output("ydotool", &a).await {
                tracing::warn!("ydotool: {e}");
            }
        });
    }

    impl InputBackend for Ydotool {
        fn send(&self, ev: InputEvent) -> Result<()> {
            let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            match ev {
                InputEvent::MoveRelative { dx, dy } => run(s(&[
                    "mousemove",
                    "-x",
                    &dx.to_string(),
                    "-y",
                    &dy.to_string(),
                ])),
                InputEvent::MoveAbsolute { .. } => {
                    anyhow::bail!("absolute pointer not supported by ydotool backend")
                }
                InputEvent::Button { button, action } => {
                    let base = match button {
                        MouseButton::Left => 0x00,
                        MouseButton::Right => 0x01,
                        MouseButton::Middle => 0x02,
                    };
                    let code = match action {
                        ButtonAction::Click | ButtonAction::DoubleClick => base | 0xC0,
                        ButtonAction::Press => base | 0x40,
                        ButtonAction::Release => base | 0x80,
                    };
                    let mut a = s(&["click"]);
                    if action == ButtonAction::DoubleClick {
                        a.extend(s(&["--repeat", "2"]));
                    }
                    a.push(format!("0x{code:02X}"));
                    run(a)
                }
                InputEvent::Scroll { dx, dy } => run(s(&[
                    "mousemove",
                    "--wheel",
                    "-x",
                    &dx.to_string(),
                    "-y",
                    &(-dy).to_string(),
                ])),
                InputEvent::Text { text } => run(vec!["type".into(), "--".into(), text]),
                InputEvent::Key { key, modifiers: m } => {
                    let Some(code) = keycode(key) else {
                        if let Key::Char(c) = key {
                            run(vec!["type".into(), "--".into(), c.to_string()]);
                        }
                        return Ok(());
                    };
                    let mods: Vec<u16> = [(m.ctrl, 29), (m.shift, 42), (m.alt, 56), (m.meta, 125)]
                        .into_iter()
                        .filter_map(|(on, c)| on.then_some(c))
                        .collect();
                    let mut a = vec!["key".to_string()];
                    a.extend(mods.iter().map(|c| format!("{c}:1")));
                    a.push(format!("{code}:1"));
                    a.push(format!("{code}:0"));
                    a.extend(mods.iter().rev().map(|c| format!("{c}:0")));
                    run(a)
                }
            }
            Ok(())
        }
    }
}
