//! Automatic transfer of phone screenshots.
//!
//! The phone detects new screenshots (Android MediaStore observer / iOS
//! `userDidTakeScreenshotNotification` + Photos) and sends them as
//! `konnekta.screenshot` with a payload and its SHA-256. The computer saves it
//! in a configurable folder, skips duplicates and can put the image on the
//! clipboard so it can be pasted immediately.

use crate::store::JsonStore;
use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::{Packet, types};
use konnekta_core::util::{sanitize_filename, unique_path};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
use konnekta_platform::{DesktopNotification, Platform};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

pub const MAX_SCREENSHOT: i64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScreenshotConfig {
    /// Save received screenshots to `folder`.
    pub save: bool,
    pub folder: Option<PathBuf>,
    /// Copy the image to the clipboard.
    pub copy_to_clipboard: bool,
    /// Ask the phone to send screenshots automatically.
    pub auto_transfer: bool,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self { save: true, folder: None, copy_to_clipboard: true, auto_transfer: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Seen {
    hashes: VecDeque<String>,
}

pub struct ScreenshotPlugin {
    platform: Platform,
    core: OnceLock<CoreHandle>,
    pub config: JsonStore<ScreenshotConfig>,
    seen: JsonStore<Seen>,
}

impl ScreenshotPlugin {
    pub fn new(platform: Platform) -> Self {
        Self { platform, core: OnceLock::new(), config: JsonStore::default(), seen: JsonStore::default() }
    }

    fn folder(&self) -> PathBuf {
        self.config.with(|c| c.folder.clone()).unwrap_or_else(|| {
            directories::UserDirs::new()
                .and_then(|d| d.picture_dir().map(|p| p.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Konnekta Screenshots")
        })
    }

    /// Returns false if already received.
    fn remember(&self, hash: &str) -> bool {
        self.seen.update(|s| {
            if s.hashes.iter().any(|h| h == hash) {
                return false;
            }
            s.hashes.push_back(hash.to_owned());
            while s.hashes.len() > 1000 {
                s.hashes.pop_front();
            }
            true
        })
    }

    async fn send_config(&self, dev: &DeviceHandle) {
        let auto = self.config.with(|c| c.auto_transfer);
        let _ = dev
            .send_if_supported(Packet::new(types::K_SCREENSHOT, json!({ "config": { "autoTransfer": auto } })))
            .await;
    }
}

fn is_png(b: &[u8]) -> bool {
    b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
}

#[async_trait]
impl Plugin for ScreenshotPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "screenshot",
            name: "Screenshots",
            description: "Receive phone screenshots automatically and paste them right away",
            origin: PluginOrigin::Konnekta,
            permission: Permission::Screenshots,
            incoming: &[types::K_SCREENSHOT],
            outgoing: &[types::K_SCREENSHOT],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "screenshot", "config.json");
        self.seen.attach(&core, "screenshot", "seen.json");
        let _ = self.core.set(core);
    }

    async fn on_connected(&self, dev: &DeviceHandle) {
        self.send_config(dev).await;
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        if !p.has_payload() {
            return Ok(());
        }
        let size = p.payload_size.unwrap_or(-1);
        if !(0..=MAX_SCREENSHOT).contains(&size) {
            bail!("screenshot too large or of unknown size");
        }
        // Dedupe early when the phone provides the hash.
        let announced = p.get_str("sha256").map(str::to_lowercase);
        if let Some(h) = &announced {
            if self.seen.with(|s| s.hashes.iter().any(|x| x == h)) {
                tracing::debug!("duplicate screenshot skipped");
                return Ok(());
            }
        }
        let mut data = Vec::with_capacity(size as usize);
        if let Err(e) = dev.fetch_payload(&p, &mut data, None, None).await {
            dev.emit("screenshot", "failed", json!({ "error": e.to_string() }));
            return Err(e);
        }
        let hash = hex::encode(Sha256::digest(&data));
        if announced.as_ref().is_some_and(|h| *h != hash) {
            dev.emit("screenshot", "failed", json!({ "error": "integrity check failed" }));
            bail!("screenshot hash mismatch");
        }
        if !self.remember(&hash) {
            return Ok(());
        }
        let cfg = self.config.get();
        let mut saved = None;
        if cfg.save {
            let dir = self.folder();
            tokio::fs::create_dir_all(&dir).await?;
            let name = sanitize_filename(p.get_str("filename").unwrap_or("screenshot.png"));
            let path = unique_path(&dir, &name);
            tokio::fs::write(&path, &data).await?;
            saved = Some(path);
        }
        let mut copied = false;
        if cfg.copy_to_clipboard {
            if is_png(&data) {
                copied = self.platform.clipboard.set_png(&data).await.is_ok();
            } else {
                tracing::info!("screenshot is not PNG; not copied to clipboard");
            }
        }
        dev.emit("screenshot", "received", json!({ "path": saved, "copied": copied, "sha256": hash }));
        let _ = self.platform.notifier.show(
            DesktopNotification {
                id: format!("screenshot-{hash}"),
                app_name: dev.name().into(),
                title: "Screenshot received".into(),
                body: if copied { "Copied to the clipboard — paste it anywhere".into() } else { saved.map(|p| p.display().to_string()).unwrap_or_default() },
                ..Default::default()
            },
            None,
        );
        Ok(())
    }

    async fn handle_action(&self, _: Option<&DeviceHandle>, action: &str, args: Value) -> Result<Value> {
        match action {
            "getConfig" => {
                let mut v = serde_json::to_value(self.config.get())?;
                v["effectiveFolder"] = json!(self.folder());
                Ok(v)
            }
            "setConfig" => {
                let c: ScreenshotConfig = crate::args(args)?;
                self.config.update(|v| *v = c);
                if let Some(core) = self.core.get() {
                    for d in core.devices() {
                        if d.has_permission(Permission::Screenshots) {
                            self.send_config(&d).await;
                        }
                    }
                }
                Ok(Value::Null)
            }
            _ => bail!("unknown action {action}"),
        }
    }
}
