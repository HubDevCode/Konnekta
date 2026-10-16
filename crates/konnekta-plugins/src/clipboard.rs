//! Bidirectional clipboard sync with loop prevention.
//!
//! * a local change is sent once to every paired device holding the
//!   Clipboard permission;
//! * content received from the network is written locally and remembered, so
//!   the local watcher does not bounce it back (no ping-pong between devices,
//!   no forwarding to third devices);
//! * `clipboard.connect` carries a timestamp: on (re)connection only newer
//!   content wins.

use crate::store::JsonStore;
use anyhow::Result;
use async_trait::async_trait;
use konnekta_core::proto::{Packet, bodies::ClipboardBody, packet::now_millis, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
use konnekta_platform::Platform;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const MAX_CLIPBOARD: usize = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClipboardConfig {
    /// Send local copies to devices.
    pub auto_send: bool,
    /// Apply content received from devices.
    pub auto_receive: bool,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            auto_send: true,
            auto_receive: true,
        }
    }
}

#[derive(Default)]
struct State {
    /// Last content observed / written locally.
    current: Option<String>,
    /// When `current` changed (ms).
    changed_at: i64,
}

pub struct ClipboardPlugin {
    platform: Platform,
    state: Mutex<State>,
    pub config: JsonStore<ClipboardConfig>,
    poll: Duration,
}

impl ClipboardPlugin {
    pub fn new(platform: Platform) -> Self {
        Self::with_poll(platform, Duration::from_millis(500))
    }

    pub fn with_poll(platform: Platform, poll: Duration) -> Self {
        Self {
            platform,
            state: Mutex::default(),
            config: JsonStore::default(),
            poll,
        }
    }

    /// Returns the content to broadcast if this is a genuine local change.
    fn observe_local(&self, text: String) -> Option<String> {
        let mut s = self.state.lock().unwrap();
        if s.current.as_deref() == Some(text.as_str()) {
            return None;
        }
        s.current = Some(text.clone());
        s.changed_at = now_millis();
        Some(text)
    }

    /// Record remote content; returns false if it is identical (no-op).
    fn accept_remote(&self, text: &str, at: i64) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.current.as_deref() == Some(text) {
            return false;
        }
        s.current = Some(text.to_owned());
        s.changed_at = at;
        true
    }

    async fn broadcast(core: &CoreHandle, text: &str) {
        for dev in core.devices() {
            if dev.has_permission(Permission::Clipboard) && dev.supports(types::CLIPBOARD) {
                let p = Packet::new(
                    types::CLIPBOARD,
                    ClipboardBody {
                        content: text.into(),
                        timestamp: None,
                    },
                );
                if let Err(e) = dev.send(p).await {
                    tracing::debug!("clipboard send failed: {e}");
                }
            }
        }
    }

    async fn apply_remote(&self, dev: &DeviceHandle, content: String, at: i64) -> Result<()> {
        if !self.config.with(|c| c.auto_receive) || content.len() > MAX_CLIPBOARD {
            return Ok(());
        }
        if self.accept_remote(&content, at) {
            self.platform.clipboard.set_text(&content).await?;
            tracing::debug!(device = %dev.name(), content = %konnekta_core::util::redact(&content), "clipboard updated");
            dev.emit(
                "clipboard",
                "received",
                json!({ "length": content.chars().count() }),
            );
        }
        Ok(())
    }
}

#[async_trait]
impl Plugin for ClipboardPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "clipboard",
            name: "Clipboard",
            description: "Share the clipboard between devices",
            origin: PluginOrigin::KdeConnect,
            permission: Permission::Clipboard,
            incoming: &[types::CLIPBOARD, types::CLIPBOARD_CONNECT],
            outgoing: &[types::CLIPBOARD, types::CLIPBOARD_CONNECT],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "clipboard", "config.json");
        // Seed with the current content so startup doesn't broadcast it.
        if let Ok(Some(t)) = self.platform.clipboard.get_text().await {
            self.state.lock().unwrap().current = Some(t);
        }
        let me = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(me.poll);
            loop {
                tick.tick().await;
                if !core.is_alive() {
                    break;
                }
                let Ok(Some(text)) = me.platform.clipboard.get_text().await else {
                    continue;
                };
                if text.is_empty() || text.len() > MAX_CLIPBOARD {
                    continue;
                }
                if let Some(changed) = me.observe_local(text)
                    && me.config.with(|c| c.auto_send)
                {
                    Self::broadcast(&core, &changed).await;
                }
            }
        });
    }

    async fn on_connected(&self, dev: &DeviceHandle) {
        if !self.config.with(|c| c.auto_send) {
            return;
        }
        let (content, ts) = {
            let s = self.state.lock().unwrap();
            (s.current.clone(), s.changed_at)
        };
        if let Some(content) = content {
            let p = Packet::new(
                types::CLIPBOARD_CONNECT,
                ClipboardBody {
                    content,
                    timestamp: Some(ts),
                },
            );
            let _ = dev.send_if_supported(p).await;
        }
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        // Parsed leniently: KDE Connect sends `content: null` / no timestamp
        // when its clipboard is empty.
        let Some(content) = p
            .get_str("content")
            .filter(|c| !c.is_empty())
            .map(str::to_owned)
        else {
            return Ok(());
        };
        match p.packet_type.as_str() {
            types::CLIPBOARD => self.apply_remote(dev, content, now_millis()).await,
            types::CLIPBOARD_CONNECT => {
                let ts = p.get_i64("timestamp").unwrap_or(0);
                let newer = ts > 0 && ts > self.state.lock().unwrap().changed_at;
                if newer {
                    self.apply_remote(dev, content, ts).await
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        match action {
            "send" => {
                // Explicit push of the current clipboard to one device.
                let text = self
                    .platform
                    .clipboard
                    .get_text()
                    .await?
                    .unwrap_or_default();
                crate::need_dev(dev)?
                    .send(Packet::new(
                        types::CLIPBOARD,
                        ClipboardBody {
                            content: text,
                            timestamp: None,
                        },
                    ))
                    .await?;
                Ok(Value::Null)
            }
            "getConfig" => Ok(serde_json::to_value(self.config.get())?),
            "setConfig" => {
                let c: ClipboardConfig = crate::args(args)?;
                self.config.update(|v| *v = c);
                Ok(Value::Null)
            }
            _ => anyhow::bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use konnekta_platform::mock::MockPlatform;

    #[test]
    fn kde_empty_clipboard_connect_is_ignored() {
        let p = Packet::new(
            types::CLIPBOARD_CONNECT,
            serde_json::json!({"content": null, "timestamp": 0}),
        );
        assert_eq!(p.get_str("content"), None);
    }

    #[test]
    fn loop_prevention_state_machine() {
        let p = ClipboardPlugin::new(MockPlatform::new().platform());
        assert_eq!(p.observe_local("a".into()), Some("a".into()));
        assert_eq!(
            p.observe_local("a".into()),
            None,
            "unchanged content is not resent"
        );
        assert!(p.accept_remote("b", 1));
        // The watcher now sees "b" locally (we wrote it): must not echo.
        assert_eq!(p.observe_local("b".into()), None);
        assert!(!p.accept_remote("b", 2), "duplicate remote content ignored");
    }
}
