//! Bidirectional clipboard sync (text and images) with loop prevention, a
//! local history and saved snippets.
//!
//! * a local change is sent once to every paired device holding the
//!   Clipboard permission;
//! * content received from the network is written locally and remembered, so
//!   the local watcher does not bounce it back (no ping-pong between devices,
//!   no forwarding to third devices);
//! * `clipboard.connect` carries a timestamp: on (re)connection only newer
//!   content wins;
//! * images travel as PNG payloads (`konnekta.clipboard.image`, Konnekta
//!   peers only), one at a time per device: a newer copy cancels the image
//!   still on its way.

use crate::store::JsonStore;
use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::payload::PayloadSource;
use konnekta_core::proto::{Packet, bodies::ClipboardBody, packet::now_millis, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo};
use konnekta_platform::Platform;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::AbortHandle;

/// Largest text sent to devices.
pub const MAX_CLIPBOARD: usize = 256 * 1024;
/// Largest text accepted from devices.
pub const MAX_INCOMING_TEXT: usize = 1024 * 1024;
pub const MAX_IMAGE: u64 = 16 * 1024 * 1024;
/// Total text kept in the history.
pub const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;
/// Characters of each entry exposed in events (chat view).
pub const PREVIEW_CHARS: usize = 400;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClipboardConfig {
    /// Send local copies to devices.
    pub auto_send: bool,
    /// Apply content received from devices.
    pub auto_receive: bool,
    /// Also sync images.
    pub images: bool,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            auto_send: true,
            auto_receive: true,
            images: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub text: String,
    /// Device id, or None for a local copy.
    pub from: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snippets {
    pub items: Vec<Snippet>,
}

#[derive(Default)]
struct State {
    /// Last content observed / written locally.
    current: Option<String>,
    /// When `current` changed (ms).
    changed_at: i64,
    image_hash: Option<[u8; 32]>,
    history: VecDeque<HistoryEntry>,
    history_bytes: usize,
    /// Image transfers in flight, per device.
    sending_image: HashMap<String, AbortHandle>,
}

pub struct ClipboardPlugin {
    platform: Platform,
    state: Mutex<State>,
    pub config: JsonStore<ClipboardConfig>,
    pub snippets: JsonStore<Snippets>,
    poll: Duration,
}

fn hash(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

fn preview(s: &str) -> String {
    s.chars().take(PREVIEW_CHARS).collect()
}

fn is_png(b: &[u8]) -> bool {
    b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
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
            snippets: JsonStore::default(),
            poll,
        }
    }

    fn remember(&self, text: &str, from: Option<&str>) {
        let mut s = self.state.lock().unwrap();
        s.history.push_front(HistoryEntry {
            text: text.to_owned(),
            from: from.map(str::to_owned),
            at: now_millis(),
        });
        s.history_bytes += text.len();
        while s.history_bytes > MAX_HISTORY_BYTES {
            let Some(old) = s.history.pop_back() else {
                break;
            };
            s.history_bytes -= old.text.len();
        }
    }

    /// Returns the content to broadcast if this is a genuine local change.
    fn observe_local(&self, text: String) -> Option<String> {
        {
            let mut s = self.state.lock().unwrap();
            if s.current.as_deref() == Some(text.as_str()) {
                return None;
            }
            s.current = Some(text.clone());
            s.changed_at = now_millis();
        }
        self.remember(&text, None);
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

    fn send_image(&self, dev: DeviceHandle, png: Vec<u8>) {
        let id = dev.id().to_owned();
        let task = tokio::spawn(async move {
            let p = Packet::new(types::CLIPBOARD_IMAGE, json!({ "mime": "image/png" }));
            match dev
                .send_with_payload(p, PayloadSource::Bytes(png), None)
                .await
            {
                Ok(t) => {
                    let _ = t.await;
                }
                Err(e) => tracing::debug!("clipboard image send failed: {e}"),
            }
        });
        // A newer image replaces the one still on its way.
        if let Some(old) = self
            .state
            .lock()
            .unwrap()
            .sending_image
            .insert(id, task.abort_handle())
        {
            old.abort();
        }
    }

    async fn broadcast_image(&self, core: &CoreHandle, png: Vec<u8>) {
        for dev in core.devices() {
            if dev.has_permission(Permission::Clipboard) && dev.supports(types::CLIPBOARD_IMAGE) {
                self.send_image(dev, png.clone());
            }
        }
    }

    async fn apply_remote(&self, dev: &DeviceHandle, content: String, at: i64) -> Result<()> {
        if !self.config.with(|c| c.auto_receive) || content.len() > MAX_INCOMING_TEXT {
            if content.len() > MAX_INCOMING_TEXT {
                dev.emit("clipboard", "tooLarge", json!({ "length": content.len() }));
            }
            return Ok(());
        }
        if self.accept_remote(&content, at) {
            self.platform.clipboard.set_text(&content).await?;
            self.remember(&content, Some(dev.id()));
            tracing::debug!(device = %dev.name(), content = %konnekta_core::util::redact(&content), "clipboard updated");
            dev.emit(
                "clipboard",
                "received",
                json!({ "text": preview(&content), "length": content.chars().count() }),
            );
        }
        Ok(())
    }

    async fn apply_image(&self, dev: &DeviceHandle, p: &Packet) -> Result<()> {
        let size = p.payload_size.unwrap_or(-1);
        if size < 0 || size as u64 > MAX_IMAGE {
            bail!("clipboard image too large");
        }
        let mut data = Vec::with_capacity(size as usize);
        dev.fetch_payload(p, &mut data, None, None).await?;
        if !self.config.with(|c| c.auto_receive && c.images) {
            return Ok(());
        }
        if !is_png(&data) {
            bail!("only PNG clipboard images are accepted");
        }
        let h = hash(&data);
        {
            let mut s = self.state.lock().unwrap();
            if s.image_hash == Some(h) {
                return Ok(());
            }
            s.image_hash = Some(h);
        }
        self.platform.clipboard.set_png(&data).await?;
        dev.emit("clipboard", "image", json!({ "bytes": data.len() }));
        Ok(())
    }
}

#[async_trait]
impl Plugin for ClipboardPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "clipboard",
            name: "Clipboard",
            description: "Share the clipboard (text and images) between devices",
            permission: Permission::Clipboard,
            incoming: &[
                types::CLIPBOARD,
                types::CLIPBOARD_INITIAL,
                types::CLIPBOARD_IMAGE,
            ],
            outgoing: &[
                types::CLIPBOARD,
                types::CLIPBOARD_INITIAL,
                types::CLIPBOARD_IMAGE,
            ],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "clipboard", "config.json");
        self.snippets.attach(&core, "clipboard", "snippets.json");
        // Seed with the current content so startup doesn't broadcast it.
        if let Ok(Some(t)) = self.platform.clipboard.get_text().await {
            self.state.lock().unwrap().current = Some(t);
        }
        if let Ok(Some(png)) = self.platform.clipboard.get_png().await {
            self.state.lock().unwrap().image_hash = Some(hash(&png));
        }
        let me = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(me.poll);
            let mut n: u64 = 0;
            loop {
                tick.tick().await;
                if !core.is_alive() {
                    break;
                }
                n += 1;
                let cfg = me.config.get();
                if let Ok(Some(text)) = me.platform.clipboard.get_text().await
                    && !text.is_empty()
                    && let Some(changed) = me.observe_local(text)
                {
                    if cfg.auto_send && changed.len() <= MAX_CLIPBOARD {
                        Self::broadcast(&core, &changed).await;
                    } else if changed.len() > MAX_CLIPBOARD {
                        core.emit_plugin(
                            None,
                            "clipboard",
                            "tooLargeToSend",
                            json!({ "length": changed.len() }),
                        );
                    }
                }
                // Images are checked less often (they are larger to read).
                if cfg.images
                    && n.is_multiple_of(4)
                    && let Ok(Some(png)) = me.platform.clipboard.get_png().await
                {
                    let h = hash(&png);
                    let changed = {
                        let mut s = me.state.lock().unwrap();
                        let c = s.image_hash != Some(h);
                        s.image_hash = Some(h);
                        c
                    };
                    if changed && cfg.auto_send && (png.len() as u64) <= MAX_IMAGE {
                        me.broadcast_image(&core, png).await;
                    }
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
        if let Some(content) = content.filter(|c| c.len() <= MAX_CLIPBOARD) {
            let p = Packet::new(
                types::CLIPBOARD_INITIAL,
                ClipboardBody {
                    content,
                    timestamp: Some(ts),
                },
            );
            let _ = dev.send_if_supported(p).await;
        }
    }

    async fn on_disconnected(&self, id: &str) {
        if let Some(t) = self.state.lock().unwrap().sending_image.remove(id) {
            t.abort();
        }
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        if p.packet_type == types::CLIPBOARD_IMAGE {
            return self.apply_image(dev, &p).await;
        }
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
            types::CLIPBOARD_INITIAL => {
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
        let text_arg = || args.get("text").and_then(Value::as_str).map(str::to_owned);
        match action {
            "send" => {
                // Explicit push of the current (or given) text to one device.
                let text = match text_arg() {
                    Some(t) => t,
                    None => self
                        .platform
                        .clipboard
                        .get_text()
                        .await?
                        .unwrap_or_default(),
                };
                if text.len() > MAX_CLIPBOARD {
                    bail!("text larger than 256 KiB");
                }
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
            "history" => {
                let s = self.state.lock().unwrap();
                Ok(serde_json::to_value(&s.history)?)
            }
            "copy" => {
                // Put a history entry or given text back on the local clipboard.
                let text = match (text_arg(), args.get("index").and_then(Value::as_u64)) {
                    (Some(t), _) => t,
                    (None, Some(i)) => self
                        .state
                        .lock()
                        .unwrap()
                        .history
                        .get(i as usize)
                        .map(|e| e.text.clone())
                        .ok_or_else(|| anyhow::anyhow!("no such entry"))?,
                    _ => bail!("give text or index"),
                };
                self.state.lock().unwrap().current = Some(text.clone());
                self.platform.clipboard.set_text(&text).await?;
                Ok(Value::Null)
            }
            "clearHistory" => {
                let mut s = self.state.lock().unwrap();
                s.history.clear();
                s.history_bytes = 0;
                Ok(Value::Null)
            }
            "snippets" => Ok(serde_json::to_value(self.snippets.get().items)?),
            "saveSnippet" => {
                let text = text_arg().ok_or_else(|| anyhow::anyhow!("missing text"))?;
                if text.len() > MAX_CLIPBOARD {
                    bail!("snippet larger than 256 KiB");
                }
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| text.chars().take(40).collect());
                let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
                self.snippets.update(|s| {
                    s.items.push(Snippet {
                        id: id.clone(),
                        name,
                        text,
                    })
                });
                Ok(json!({ "id": id }))
            }
            "deleteSnippet" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                self.snippets.update(|s| s.items.retain(|x| x.id != id));
                Ok(Value::Null)
            }
            "getConfig" => Ok(serde_json::to_value(self.config.get())?),
            "setConfig" => {
                let c: ClipboardConfig = crate::args(args)?;
                self.config.update(|v| *v = c);
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
    fn kde_empty_clipboard_connect_is_ignored() {
        let p = Packet::new(
            types::CLIPBOARD_INITIAL,
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
        assert_eq!(p.observe_local("b".into()), None);
        assert!(!p.accept_remote("b", 2), "duplicate remote content ignored");
    }

    #[test]
    fn history_is_bounded() {
        let p = ClipboardPlugin::new(MockPlatform::new().platform());
        let big = "x".repeat(MAX_HISTORY_BYTES / 2 + 1);
        p.remember(&big, None);
        p.remember(&big, None);
        p.remember("small", Some("dev"));
        let s = p.state.lock().unwrap();
        assert!(s.history_bytes <= MAX_HISTORY_BYTES);
        assert_eq!(s.history.front().unwrap().text, "small");
    }

    #[tokio::test]
    async fn snippets_and_copy() {
        let mock = MockPlatform::new();
        let p = ClipboardPlugin::new(mock.platform());
        let r = p
            .handle_action(None, "saveSnippet", json!({"text": "indirizzo"}))
            .await
            .unwrap();
        assert_eq!(p.snippets.get().items.len(), 1);
        p.handle_action(None, "copy", json!({"text": "indirizzo"}))
            .await
            .unwrap();
        assert_eq!(mock.clipboard.lock().unwrap().as_deref(), Some("indirizzo"));
        p.handle_action(None, "deleteSnippet", json!({"id": r["id"]}))
            .await
            .unwrap();
        assert!(p.snippets.get().items.is_empty());
    }
}
