//! Remote touchpad and keyboard (`konnekta.input`), with the absolute
//! pointer used by remote desktop.

use anyhow::Result;
use async_trait::async_trait;
use konnekta_core::proto::{Packet, bodies::MousepadBody, types};
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo};
use konnekta_platform::input::InputEvent;
use konnekta_platform::{ButtonAction, Key, Modifiers, MouseButton, Platform, SpecialKey};
use serde_json::{Value, json};
use std::sync::Mutex;

pub struct InputPlugin {
    platform: Platform,
    /// Sub-pixel remainder of relative movements.
    acc: Mutex<(f64, f64)>,
    /// Scroll remainder in wheel steps (x, y).
    scroll_acc: Mutex<(f64, f64)>,
}

/// `specialKey` codes (the same numbers KDE Connect apps send through the
/// bridge).
pub fn special_key(code: u32) -> Option<SpecialKey> {
    use SpecialKey::*;
    Some(match code {
        1 => Backspace,
        2 => Tab,
        3 => Return, // Linefeed
        4 => Left,
        5 => Up,
        6 => Right,
        7 => Down,
        8 => PageUp,
        9 => PageDown,
        10 => Home,
        11 => End,
        12 => Return,
        13 => Delete,
        14 => Escape,
        21..=32 => F((code - 20) as u8),
        // Konnekta: modifiers pressed alone.
        40 => Super,
        41 => Control,
        42 => Alt,
        43 => Shift,
        _ => return None,
    })
}

impl InputPlugin {
    pub fn new(platform: Platform) -> Self {
        Self {
            platform,
            acc: Mutex::new((0.0, 0.0)),
            scroll_acc: Mutex::new((0.0, 0.0)),
        }
    }

    /// Translate a mousepad packet into input events (pure, testable).
    pub fn events(&self, b: &MousepadBody) -> Vec<InputEvent> {
        let mut out = Vec::new();
        let mods = Modifiers {
            ctrl: b.ctrl,
            shift: b.shift,
            alt: b.alt,
            meta: b.super_key,
        };
        if let (Some(x), Some(y)) = (b.abs_x, b.abs_y) {
            out.push(InputEvent::MoveAbsolute { x, y });
        }
        let click = |button, action| InputEvent::Button { button, action };
        if b.singleclick {
            out.push(click(MouseButton::Left, ButtonAction::Click));
        }
        if b.doubleclick {
            out.push(click(MouseButton::Left, ButtonAction::DoubleClick));
        }
        if b.middleclick {
            out.push(click(MouseButton::Middle, ButtonAction::Click));
        }
        if b.rightclick {
            out.push(click(MouseButton::Right, ButtonAction::Click));
        }
        if b.singlehold {
            out.push(click(MouseButton::Left, ButtonAction::Press));
        }
        if b.singlerelease {
            out.push(click(MouseButton::Left, ButtonAction::Release));
        }
        if let Some(code) = b.special_key.filter(|c| *c != 0) {
            if let Some(k) = special_key(code) {
                out.push(InputEvent::Key {
                    key: Key::Special(k),
                    modifiers: mods,
                });
            }
        } else if let Some(text) = b.key.as_deref().filter(|k| !k.is_empty()) {
            let text: String = text.chars().take(4096).collect();
            if mods.ctrl || mods.alt || mods.meta {
                // Shortcut such as ctrl+c: one key with modifiers.
                if let Some(c) = text.chars().next() {
                    out.push(InputEvent::Key {
                        key: Key::Char(c.to_ascii_lowercase()),
                        modifiers: mods,
                    });
                }
            } else {
                out.push(InputEvent::Text { text });
            }
        }
        let (dx, dy) = (b.dx.unwrap_or(0.0), b.dy.unwrap_or(0.0));
        if (dx != 0.0 || dy != 0.0) && dx.is_finite() && dy.is_finite() {
            if b.scroll {
                // KDE: negative dy scrolls down. One wheel step per 10 px,
                // remainders carried over so small moves add up smoothly
                // instead of each becoming a full step.
                let mut acc = self.scroll_acc.lock().unwrap();
                acc.0 += (dx / 10.0).clamp(-20.0, 20.0);
                acc.1 += (-dy / 10.0).clamp(-20.0, 20.0);
                let (sx, sy) = (acc.0.trunc(), acc.1.trunc());
                acc.0 -= sx;
                acc.1 -= sy;
                if sx != 0.0 || sy != 0.0 {
                    out.push(InputEvent::Scroll {
                        dx: -(sx as i32),
                        dy: sy as i32,
                    });
                }
            } else {
                let mut acc = self.acc.lock().unwrap();
                acc.0 += dx.clamp(-2000.0, 2000.0);
                acc.1 += dy.clamp(-2000.0, 2000.0);
                let (ix, iy) = (acc.0.trunc(), acc.1.trunc());
                acc.0 -= ix;
                acc.1 -= iy;
                if ix != 0.0 || iy != 0.0 {
                    out.push(InputEvent::MoveRelative {
                        dx: ix as i32,
                        dy: iy as i32,
                    });
                }
            }
        }
        out
    }
}

#[async_trait]
impl Plugin for InputPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "input",
            name: "Remote input",
            description: "Use the phone as touchpad and keyboard",
            permission: Permission::RemoteInput,
            incoming: &[types::INPUT],
            outgoing: &[types::INPUT_ECHO, types::INPUT_STATE],
        }
    }

    async fn on_connected(&self, dev: &DeviceHandle) {
        let _ = dev
            .send_if_supported(Packet::new(types::INPUT_STATE, json!({ "state": true })))
            .await;
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        let body: MousepadBody = p.body_as()?;
        for ev in self.events(&body) {
            self.platform.input.send(ev)?;
        }
        if body.send_ack {
            let mut echo = p.body.clone();
            echo.insert("isAck".into(), Value::Bool(true));
            dev.send_if_supported(Packet::new(types::INPUT_ECHO, echo))
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use konnekta_platform::mock::MockPlatform;

    fn plugin() -> InputPlugin {
        InputPlugin::new(MockPlatform::new().platform())
    }

    #[test]
    fn subpixel_accumulation() {
        let p = plugin();
        let b = MousepadBody {
            dx: Some(0.6),
            dy: Some(-0.6),
            ..Default::default()
        };
        assert!(p.events(&b).is_empty());
        assert_eq!(
            p.events(&b),
            vec![InputEvent::MoveRelative { dx: 1, dy: -1 }]
        );
    }

    #[test]
    fn keys_and_clicks() {
        let p = plugin();
        let b = MousepadBody {
            special_key: Some(12),
            ..Default::default()
        };
        assert_eq!(
            p.events(&b),
            vec![InputEvent::Key {
                key: Key::Special(SpecialKey::Return),
                modifiers: Modifiers::default()
            }]
        );
        let b = MousepadBody {
            key: Some("C".into()),
            ctrl: true,
            ..Default::default()
        };
        assert!(matches!(
            p.events(&b)[0],
            InputEvent::Key {
                key: Key::Char('c'),
                modifiers: Modifiers { ctrl: true, .. }
            }
        ));
        let b = MousepadBody {
            key: Some("ciao".into()),
            ..Default::default()
        };
        assert_eq!(
            p.events(&b),
            vec![InputEvent::Text {
                text: "ciao".into()
            }]
        );
        let b = MousepadBody {
            rightclick: true,
            ..Default::default()
        };
        assert_eq!(
            p.events(&b),
            vec![InputEvent::Button {
                button: MouseButton::Right,
                action: ButtonAction::Click
            }]
        );
        assert_eq!(special_key(21), Some(SpecialKey::F(1)));
        assert_eq!(special_key(32), Some(SpecialKey::F(12)));
        assert_eq!(special_key(99), None);
    }

    #[test]
    fn scroll_direction() {
        let p = plugin();
        let b = MousepadBody {
            dy: Some(-30.0),
            scroll: true,
            ..Default::default()
        };
        assert_eq!(p.events(&b), vec![InputEvent::Scroll { dx: 0, dy: 3 }]);
        // Small moves add up instead of each being a full step.
        let b = MousepadBody {
            dy: Some(4.0),
            scroll: true,
            ..Default::default()
        };
        assert!(p.events(&b).is_empty());
        assert!(p.events(&b).is_empty());
        assert_eq!(p.events(&b), vec![InputEvent::Scroll { dx: 0, dy: -1 }]);
    }

    #[test]
    fn lone_modifiers() {
        assert_eq!(special_key(40), Some(SpecialKey::Super));
        assert_eq!(special_key(43), Some(SpecialKey::Shift));
    }
}
