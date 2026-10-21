//! Use the phone as the computer's camera and scanner.
//!
//! The computer asks (`photo.request` with `mode`): the phone opens its
//! camera and answers with a photo (payload), or with the recognised text of
//! a document scan (OCR on the phone) or a QR code. Text goes straight to the
//! clipboard; photos are saved in the download folder.
//!
//! Answers are accepted only while a request is pending (5 minutes), so a
//! phone cannot push photos nobody asked for. Same packets as the KDE Connect
//! photo plugin, which only knows `photo`.

use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::{Packet, types};
use konnekta_core::util::{enough_space, sanitize_filename, unique_path};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo};
use konnekta_platform::{DesktopNotification, Platform};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const MAX_PHOTO: i64 = 64 * 1024 * 1024;
pub const MAX_TEXT: usize = 20_000;
pub const PENDING_TIME: Duration = Duration::from_secs(300);
const MODES: &[&str] = &["photo", "scan", "qr"];

pub struct PhotoPlugin {
    platform: Platform,
    core: OnceLock<CoreHandle>,
    /// device id -> (mode, asked at)
    pending: Mutex<HashMap<String, (String, Instant)>>,
}

impl PhotoPlugin {
    pub fn new(platform: Platform) -> Self {
        Self {
            platform,
            core: OnceLock::new(),
            pending: Mutex::default(),
        }
    }

    /// Consume the pending request of this device, if still valid.
    fn take_pending(&self, device: &str) -> Option<String> {
        let (mode, at) = self.pending.lock().unwrap().remove(device)?;
        (at.elapsed() < PENDING_TIME).then_some(mode)
    }

    fn notify(&self, dev: &DeviceHandle, title: &str, body: String) {
        let _ = self.platform.notifier.show(
            DesktopNotification {
                id: format!("photo-{}", dev.id()),
                app_name: dev.name().into(),
                title: title.into(),
                body,
                ..Default::default()
            },
            None,
        );
    }
}

#[async_trait]
impl Plugin for PhotoPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "photo",
            name: "Camera and scanner",
            description: "Take a photo, scan a document (OCR) or read a QR code with the phone",
            permission: Permission::Files,
            incoming: &[types::PHOTO],
            outgoing: &[types::PHOTO_REQUEST],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        let _ = self.core.set(core);
    }

    async fn on_disconnected(&self, id: &str) {
        self.pending.lock().unwrap().remove(id);
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        // The phone cancelled (closed the camera).
        if p.body.get("cancelled").and_then(Value::as_bool) == Some(true) {
            self.pending.lock().unwrap().remove(dev.id());
            dev.emit("photo", "cancelled", json!({}));
            return Ok(());
        }
        let Some(mode) = self.take_pending(dev.id()) else {
            bail!("photo not requested");
        };
        if let Some(text) = p.get_str("text").filter(|_| !p.has_payload()) {
            let text: String = text.chars().take(MAX_TEXT).collect();
            let copied = self.platform.clipboard.set_text(&text).await.is_ok();
            dev.emit(
                "photo",
                "text",
                json!({ "mode": mode, "text": text, "copied": copied }),
            );
            let what = if mode == "qr" {
                "Codice QR letto"
            } else {
                "Testo riconosciuto"
            };
            self.notify(
                dev,
                what,
                if copied {
                    "Copiato negli appunti".into()
                } else {
                    text
                },
            );
            return Ok(());
        }
        if !p.has_payload() {
            bail!("empty photo");
        }
        let size = p.payload_size.unwrap_or(-1);
        if !(0..=MAX_PHOTO).contains(&size) {
            bail!("photo too large or of unknown size");
        }
        let dir = self
            .core
            .get()
            .map(CoreHandle::download_dir)
            .unwrap_or_default();
        tokio::fs::create_dir_all(&dir).await?;
        if !enough_space(&dir, size as u64) {
            dev.emit("photo", "failed", json!({ "error": "disk full" }));
            bail!("not enough disk space");
        }
        let name = sanitize_filename(p.get_str("filename").unwrap_or("foto.jpg"));
        let path = unique_path(&dir, &name);
        let part = path.with_extension("part");
        let mut f = tokio::fs::File::create(&part).await?;
        if let Err(e) = dev.fetch_payload(&p, &mut f, None, None).await {
            drop(f);
            let _ = tokio::fs::remove_file(&part).await;
            dev.emit("photo", "failed", json!({ "error": e.to_string() }));
            return Err(e);
        }
        drop(f);
        tokio::fs::rename(&part, &path).await?;
        dev.emit("photo", "received", json!({ "mode": mode, "path": path }));
        self.notify(dev, "Foto ricevuta", path.display().to_string());
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        let dev = crate::need_dev(dev)?;
        match action {
            "take" => {
                let mode = args.get("mode").and_then(Value::as_str).unwrap_or("photo");
                if !MODES.contains(&mode) {
                    bail!("unknown mode {mode}");
                }
                if mode != "photo" && dev.protocol == konnekta_core::proto::Protocol::KdeConnect {
                    bail!("KDE Connect can only take photos");
                }
                self.pending
                    .lock()
                    .unwrap()
                    .insert(dev.id().to_owned(), (mode.to_owned(), Instant::now()));
                dev.send(Packet::new(types::PHOTO_REQUEST, json!({ "mode": mode })))
                    .await?;
                Ok(Value::Null)
            }
            "cancel" => {
                self.pending.lock().unwrap().remove(dev.id());
                Ok(Value::Null)
            }
            _ => bail!("unknown action {action}"),
        }
    }
}
