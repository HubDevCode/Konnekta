//! Presentation remote.
//!
//! KDE Connect's presenter sends pointer deltas (laser pointer). Konnekta
//! adds `konnektaKey`: a *named* slide command mapped locally to a key
//! shortcut according to the selected presentation program, so the phone
//! needs only the Presenter permission, not full remote input.

use crate::store::JsonStore;
use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::{Packet, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
use konnekta_platform::input::InputEvent;
use konnekta_platform::{Platform, parse_shortcut};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PresenterConfig {
    /// Active keymap name.
    pub program: String,
    /// program -> (command -> shortcut)
    pub keymaps: BTreeMap<String, BTreeMap<String, String>>,
    /// Pointer sensitivity multiplier for presenter deltas.
    pub sensitivity: f64,
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

impl Default for PresenterConfig {
    fn default() -> Self {
        let mut keymaps = BTreeMap::new();
        keymaps.insert(
            "powerpoint".into(),
            map(&[
                ("next", "PageDown"),
                ("previous", "PageUp"),
                ("start", "F5"),
                ("startCurrent", "shift+F5"),
                ("stop", "Escape"),
                ("black", "b"),
                ("white", "w"),
            ]),
        );
        keymaps.insert(
            "libreoffice".into(),
            map(&[
                ("next", "PageDown"),
                ("previous", "PageUp"),
                ("start", "F5"),
                ("startCurrent", "shift+F5"),
                ("stop", "Escape"),
                ("black", "b"),
                ("white", "w"),
            ]),
        );
        keymaps.insert(
            "keynote".into(),
            map(&[
                ("next", "Right"),
                ("previous", "Left"),
                ("start", "meta+alt+p"),
                ("stop", "Escape"),
                ("black", "b"),
                ("white", "w"),
            ]),
        );
        keymaps.insert(
            "googleslides".into(),
            map(&[
                ("next", "Right"),
                ("previous", "Left"),
                ("start", "ctrl+F5"),
                ("stop", "Escape"),
                ("black", "b"),
                ("white", "w"),
            ]),
        );
        keymaps.insert(
            "pdf".into(),
            map(&[
                ("next", "Right"),
                ("previous", "Left"),
                ("start", "F11"),
                ("stop", "Escape"),
            ]),
        );
        Self {
            program: "powerpoint".into(),
            keymaps,
            sensitivity: 1000.0,
        }
    }
}

pub struct PresenterPlugin {
    platform: Platform,
    pub config: JsonStore<PresenterConfig>,
}

impl PresenterPlugin {
    pub fn new(platform: Platform) -> Self {
        Self {
            platform,
            config: JsonStore::default(),
        }
    }

    fn press(&self, command: &str) -> Result<()> {
        let shortcut = self.config.with(|c| {
            c.keymaps
                .get(&c.program)
                .and_then(|m| m.get(command))
                .cloned()
        });
        let Some(shortcut) = shortcut else {
            bail!("no shortcut for '{command}'")
        };
        let (modifiers, key) = parse_shortcut(&shortcut)?;
        self.platform.input.send(InputEvent::Key { key, modifiers })
    }
}

#[async_trait]
impl Plugin for PresenterPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "presenter",
            name: "Presentation remote",
            description: "Control slides and use the phone as a laser pointer",
            origin: PluginOrigin::KdeConnect,
            permission: Permission::Presenter,
            incoming: &[types::PRESENTER],
            outgoing: &[],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "presenter", "config.json");
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        if let Some(cmd) = p.get_str("konnektaKey") {
            self.press(cmd)?;
            dev.emit("presenter", "command", json!({ "command": cmd }));
            return Ok(());
        }
        if p.get_bool("stop") {
            return Ok(());
        }
        let k = self.config.with(|c| c.sensitivity);
        let dx = p.get_f64("dx").unwrap_or(0.0) * k;
        let dy = p.get_f64("dy").unwrap_or(0.0) * k;
        if dx.is_finite() && dy.is_finite() && (dx.abs() >= 1.0 || dy.abs() >= 1.0) {
            self.platform.input.send(InputEvent::MoveRelative {
                dx: dx.clamp(-500.0, 500.0) as i32,
                dy: dy.clamp(-500.0, 500.0) as i32,
            })?;
        }
        Ok(())
    }

    async fn handle_action(
        &self,
        _: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        match action {
            "getConfig" => Ok(serde_json::to_value(self.config.get())?),
            "setConfig" => {
                let c: PresenterConfig = crate::args(args)?;
                for m in c.keymaps.values() {
                    for s in m.values() {
                        parse_shortcut(s)?;
                    }
                }
                self.config.update(|v| *v = c);
                Ok(Value::Null)
            }
            "press" => {
                let cmd = args
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.press(cmd)?;
                Ok(Value::Null)
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use konnekta_platform::mock::MockPlatform;

    #[test]
    fn default_keymaps_parse_and_press() {
        let mock = MockPlatform::new();
        let p = PresenterPlugin::new(mock.platform());
        for m in p.config.get().keymaps.values() {
            for s in m.values() {
                parse_shortcut(s).unwrap();
            }
        }
        p.press("next").unwrap();
        assert!(mock.calls()[0].contains("pageDown"));
        assert!(p.press("explode").is_err());
    }
}
