//! File / text / URL sharing (`konnekta.share`) with a transfer manager.
//!
//! Files travel over a dedicated pinned-TLS socket. Between Konnekta peers
//! transfers carry a SHA-256 (integrity check) and can resume: the partial
//! file is kept as `<name>.<hash>.part` and the receiver asks the sender to
//! continue from its size.

use crate::store::JsonStore;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use konnekta_core::payload::{PayloadSource, Progress};
use konnekta_core::proto::{Packet, bodies::ShareBody, packet::now_millis, types};
use konnekta_core::util::{sanitize_filename, unique_path};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo};
use konnekta_platform::{DesktopNotification, Platform};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferState {
    InProgress,
    Completed,
    /// Interrupted (connection lost): may be resumed / retried.
    Interrupted,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub device_id: String,
    pub direction: Direction,
    pub name: String,
    pub size: u64,
    pub transferred: u64,
    pub state: TransferState,
    #[serde(default)]
    pub error: Option<String>,
    /// Local path (destination for incoming, source for outgoing).
    pub path: Option<PathBuf>,
    pub started_at: i64,
    #[serde(default)]
    pub resumed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShareConfig {
    /// Destination folder; default: the core download dir.
    pub destination: Option<PathBuf>,
    /// Put received text on the clipboard.
    pub text_to_clipboard: bool,
    /// Refuse files bigger than this (0 = unlimited).
    pub max_file_size: u64,
}

impl Default for ShareConfig {
    fn default() -> Self {
        Self {
            destination: None,
            text_to_clipboard: true,
            max_file_size: 0,
        }
    }
}

/// Something to deliver when the device is reachable again.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OutboxContent {
    Files { paths: Vec<PathBuf> },
    Folder { path: PathBuf },
    Text { text: String },
    Url { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutboxItem {
    pub id: String,
    pub device_id: String,
    pub content: OutboxContent,
    pub queued_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Outbox {
    pub items: Vec<OutboxItem>,
}

pub struct SharePlugin {
    platform: Platform,
    me: std::sync::Weak<SharePlugin>,
    core: OnceLock<CoreHandle>,
    pub config: JsonStore<ShareConfig>,
    pub outbox: JsonStore<Outbox>,
    transfers: Arc<Mutex<Vec<Transfer>>>,
}

/// Only `http(s)://host...` links are shared as links.
pub fn is_web_url(s: &str) -> bool {
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"));
    rest.is_some_and(|r| {
        let host = r.split(['/', '?', '#']).next().unwrap_or("");
        !host.is_empty() && !host.chars().any(|c| c.is_whitespace() || c.is_control())
    })
}

/// Pack a folder into `<out_dir>/<name>.tar.gz` (symlinks are stored as
/// links, never followed outside the folder).
pub fn archive_folder(dir: &Path, out_dir: &Path) -> Result<PathBuf> {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "cartella".into());
    std::fs::create_dir_all(out_dir)?;
    let out = unique_path(out_dir, &format!("{name}.tar.gz"));
    let file = std::fs::File::create(&out)?;
    let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(enc);
    tar.follow_symlinks(false);
    tar.append_dir_all(&name, dir)?;
    tar.into_inner()?.finish()?;
    Ok(out)
}

pub async fn sha256_file(path: &Path) -> Result<String> {
    let p = path.to_owned();
    tokio::task::spawn_blocking(move || -> Result<String> {
        use std::io::Read;
        let mut f = std::fs::File::open(p)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Ok(hex::encode(h.finalize()))
    })
    .await?
}

fn valid_sha(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

impl SharePlugin {
    pub fn new(platform: Platform) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            platform,
            me: me.clone(),
            core: OnceLock::new(),
            config: JsonStore::default(),
            outbox: JsonStore::default(),
            transfers: Arc::default(),
        })
    }

    async fn send_text(&self, dev: &DeviceHandle, text: String) -> Result<()> {
        dev.send(Packet::new(
            types::SHARE,
            ShareBody {
                text: Some(text.clone()),
                ..Default::default()
            },
        ))
        .await?;
        dev.emit(
            "share",
            "sent",
            json!({ "kind": "text", "text": text.chars().take(400).collect::<String>() }),
        );
        Ok(())
    }

    async fn send_url(&self, dev: &DeviceHandle, url: String) -> Result<()> {
        if !is_web_url(&url) {
            bail!("only http and https links with a host can be shared as links");
        }
        dev.send(Packet::new(
            types::SHARE,
            ShareBody {
                url: Some(url.clone()),
                ..Default::default()
            },
        ))
        .await?;
        dev.emit("share", "sent", json!({ "kind": "url", "text": url }));
        Ok(())
    }

    fn temp_dir(&self) -> PathBuf {
        self.core
            .get()
            .and_then(|c| c.plugin_data_dir("share"))
            .unwrap_or_else(std::env::temp_dir)
            .join("archives")
    }

    async fn send_folder(self: &Arc<Self>, dev: DeviceHandle, path: PathBuf) -> Result<()> {
        if !tokio::fs::metadata(&path).await?.is_dir() {
            bail!("{} is not a folder", path.display());
        }
        let out_dir = self.temp_dir();
        let archive =
            tokio::task::spawn_blocking(move || archive_folder(&path, &out_dir)).await??;
        let me = self.clone();
        tokio::spawn(async move {
            if let Err(e) = me.send_file(&dev, archive.clone(), 1, 0).await {
                tracing::warn!("folder send failed: {e:#}");
            }
            let _ = tokio::fs::remove_file(&archive).await;
        });
        Ok(())
    }

    async fn deliver(self: &Arc<Self>, dev: DeviceHandle, content: OutboxContent) -> Result<()> {
        match content {
            OutboxContent::Files { paths } => self.send_files(dev, paths).await,
            OutboxContent::Folder { path } => self.send_folder(dev, path).await,
            OutboxContent::Text { text } => self.send_text(&dev, text).await,
            OutboxContent::Url { url } => self.send_url(&dev, url).await,
        }
    }

    async fn flush_outbox(self: Arc<Self>, dev: DeviceHandle) {
        let items: Vec<OutboxItem> = self.outbox.with(|o| {
            o.items
                .iter()
                .filter(|i| i.device_id == dev.id())
                .cloned()
                .collect()
        });
        for item in items {
            match self.deliver(dev.clone(), item.content.clone()).await {
                Ok(()) => {
                    self.outbox.update(|o| o.items.retain(|i| i.id != item.id));
                    dev.emit("share", "outboxSent", json!({ "id": item.id }));
                }
                Err(e) => {
                    tracing::warn!("outbox delivery failed: {e:#}");
                    break;
                }
            }
        }
    }

    fn destination(&self) -> PathBuf {
        self.config
            .with(|c| c.destination.clone())
            .or_else(|| self.core.get().map(CoreHandle::download_dir))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn add(&self, t: Transfer) {
        self.emit(&t);
        let mut v = self.transfers.lock().unwrap();
        v.push(t);
        // Keep history bounded.
        if v.len() > 500 {
            let drop_n = v.len() - 500;
            v.retain({
                let mut i = 0;
                move |t| {
                    i += 1;
                    t.state == TransferState::InProgress || i > drop_n
                }
            });
        }
    }

    fn emit(&self, t: &Transfer) {
        if let Some(c) = self.core.get() {
            c.emit_plugin(
                Some(&t.device_id),
                "share",
                "transfer",
                serde_json::to_value(t).unwrap_or_default(),
            );
        }
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut Transfer)) {
        let t = {
            let mut v = self.transfers.lock().unwrap();
            let Some(t) = v.iter_mut().find(|t| t.id == id) else {
                return;
            };
            f(t);
            t.clone()
        };
        self.emit(&t);
    }

    fn progress(&self, id: &str) -> Progress {
        let transfers = self.transfers.clone();
        let core = self.core.get().cloned();
        let id = id.to_owned();
        let last = Mutex::new(Instant::now() - Duration::from_secs(1));
        Arc::new(move |done, total| {
            let snapshot = {
                let mut v = transfers.lock().unwrap();
                let Some(t) = v.iter_mut().find(|t| t.id == id) else {
                    return;
                };
                t.transferred = done;
                t.size = total;
                let mut l = last.lock().unwrap();
                if l.elapsed() < Duration::from_millis(250) && done < total {
                    return;
                }
                *l = Instant::now();
                t.clone()
            };
            if let Some(c) = &core {
                c.emit_plugin(
                    Some(&snapshot.device_id),
                    "share",
                    "transfer",
                    serde_json::to_value(&snapshot).unwrap_or_default(),
                );
            }
        })
    }

    async fn receive_file(&self, dev: &DeviceHandle, p: &Packet, body: ShareBody) -> Result<()> {
        let name = sanitize_filename(body.filename.as_deref().unwrap_or("file"));
        let size = p.payload_size.unwrap_or(-1);
        if size < 0 {
            bail!("unknown payload size");
        }
        let size = size as u64;
        let max = self.config.with(|c| c.max_file_size);
        if max > 0 && size > max {
            bail!("file {name} exceeds the configured size limit");
        }
        let dir = self.destination();
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("cannot create {}", dir.display()))?;

        let sha = body
            .sha256
            .as_deref()
            .filter(|s| valid_sha(s))
            .map(str::to_lowercase);
        let resumable = sha.is_some() && dev.resume_supported();
        let part = match &sha {
            Some(h) if resumable => dir.join(format!(".{name}.{}.part", &h[..16])),
            _ => unique_path(&dir, &format!(".{name}.part")),
        };
        let offset = if resumable {
            tokio::fs::metadata(&part)
                .await
                .map(|m| m.len().min(size))
                .unwrap_or(0)
        } else {
            0
        };
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.add(Transfer {
            id: id.clone(),
            device_id: dev.id().to_owned(),
            direction: Direction::Incoming,
            name: name.clone(),
            size,
            transferred: offset,
            state: TransferState::InProgress,
            error: None,
            path: None,
            started_at: now_millis(),
            resumed: offset > 0,
        });

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(offset > 0)
            .truncate(offset == 0)
            .open(&part)
            .await?;
        let resume = if resumable { Some(offset) } else { None };
        let r = dev
            .fetch_payload(p, &mut file, resume, Some(self.progress(&id)))
            .await;
        file.flush().await.ok();
        drop(file);
        if let Err(e) = r {
            let keep = resumable;
            if !keep {
                let _ = tokio::fs::remove_file(&part).await;
            }
            self.update(&id, |t| {
                t.state = if keep {
                    TransferState::Interrupted
                } else {
                    TransferState::Failed
                };
                t.error = Some(e.to_string());
            });
            return Err(e);
        }
        if let Some(expected) = &sha {
            let got = sha256_file(&part).await?;
            if &got != expected {
                let _ = tokio::fs::remove_file(&part).await;
                self.update(&id, |t| {
                    t.state = TransferState::Failed;
                    t.error = Some("integrity check failed (SHA-256 mismatch)".into());
                });
                bail!("integrity check failed for {name}");
            }
        }
        let dest = unique_path(&dir, &name);
        tokio::fs::rename(&part, &dest).await?;
        if let Some(ms) = body.last_modified.filter(|m| *m > 0) {
            let f = std::fs::File::options().write(true).open(&dest);
            if let Ok(f) = f {
                let _ = f.set_modified(std::time::UNIX_EPOCH + Duration::from_millis(ms as u64));
            }
        }
        self.update(&id, |t| {
            t.state = TransferState::Completed;
            t.transferred = size;
            t.path = Some(dest.clone());
        });
        // A click on the notification (or "Apri") opens the file; received
        // files are never opened without the user asking.
        let launcher = self.platform.launcher.clone();
        let (file, folder) = (dest.clone(), dir.clone());
        let _ = self.platform.notifier.show(
            DesktopNotification {
                id: format!("share-{id}"),
                app_name: dev.name().into(),
                title: "File ricevuto".into(),
                body: name,
                actions: vec![
                    ("default".into(), "Apri".into()),
                    ("open".into(), "Apri".into()),
                    ("folder".into(), "Mostra nella cartella".into()),
                ],
                ..Default::default()
            },
            Some(Box::new(move |a| {
                let target = match a.as_str() {
                    "default" | "open" => file.clone(),
                    "folder" => folder.clone(),
                    _ => return,
                };
                if let Ok(h) = tokio::runtime::Handle::try_current() {
                    h.spawn(async move {
                        let _ = launcher.open(&target.to_string_lossy()).await;
                    });
                }
            })),
        );
        if body.open == Some(true) {
            // Only reveal the folder: never auto-execute received files.
            let _ = self.platform.launcher.open(&dir.to_string_lossy()).await;
        }
        Ok(())
    }

    async fn send_file(
        &self,
        dev: &DeviceHandle,
        path: PathBuf,
        total_files: u32,
        total_size: u64,
    ) -> Result<String> {
        let meta = tokio::fs::metadata(&path)
            .await
            .with_context(|| format!("cannot read {}", path.display()))?;
        if !meta.is_file() {
            bail!("{} is not a file", path.display());
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.add(Transfer {
            id: id.clone(),
            device_id: dev.id().to_owned(),
            direction: Direction::Outgoing,
            name: name.clone(),
            size: meta.len(),
            transferred: 0,
            state: TransferState::InProgress,
            error: None,
            path: Some(path.clone()),
            started_at: now_millis(),
            resumed: false,
        });
        let sha = if dev.resume_supported() {
            Some(sha256_file(&path).await?)
        } else {
            None
        };
        let ms = |t: std::io::Result<std::time::SystemTime>| {
            t.ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
        };
        let body = ShareBody {
            filename: Some(name),
            last_modified: ms(meta.modified()),
            creation_time: ms(meta.created()),
            number_of_files: Some(total_files),
            total_payload_size: Some(total_size as i64),
            sha256: sha,
            ..Default::default()
        };
        let task = dev
            .send_with_payload(
                Packet::new(types::SHARE, body),
                PayloadSource::File(path),
                Some(self.progress(&id)),
            )
            .await;
        let result = match task {
            Ok(task) => task.await.map_err(anyhow::Error::from).and_then(|r| r),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => self.update(&id, |t| {
                t.state = TransferState::Completed;
                t.transferred = t.size;
            }),
            Err(e) => {
                self.update(&id, |t| {
                    t.state = if t.transferred > 0 {
                        TransferState::Interrupted
                    } else {
                        TransferState::Failed
                    };
                    t.error = Some(e.to_string());
                });
                return Err(e);
            }
        }
        Ok(id)
    }

    async fn send_files(self: &Arc<Self>, dev: DeviceHandle, paths: Vec<PathBuf>) -> Result<()> {
        let mut total = 0u64;
        for p in &paths {
            total += tokio::fs::metadata(p)
                .await
                .with_context(|| format!("cannot read {}", p.display()))?
                .len();
        }
        let n = paths.len() as u32;
        if n > 1 {
            let _ = dev
                .send_if_supported(Packet::new(
                    types::SHARE_UPDATE,
                    json!({ "numberOfFiles": n, "totalPayloadSize": total }),
                ))
                .await;
        }
        let me = self.clone();
        tokio::spawn(async move {
            for p in paths {
                if let Err(e) = me.send_file(&dev, p, n, total).await {
                    tracing::warn!("send failed: {e:#}");
                }
            }
        });
        Ok(())
    }
}

#[async_trait]
impl Plugin for SharePlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "share",
            name: "Share",
            description: "Send and receive files, text and links",
            permission: Permission::Files,
            incoming: &[types::SHARE, types::SHARE_UPDATE],
            outgoing: &[types::SHARE, types::SHARE_UPDATE],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "share", "config.json");
        self.outbox.attach(&core, "share", "outbox.json");
        let _ = self.core.set(core);
    }

    async fn on_connected(&self, dev: &DeviceHandle) {
        if let Some(me) = self.me.upgrade() {
            tokio::spawn(me.flush_outbox(dev.clone()));
        }
    }

    async fn on_disconnected(&self, id: &str) {
        let ids: Vec<String> = self
            .transfers
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.device_id == id && t.state == TransferState::InProgress)
            .map(|t| t.id.clone())
            .collect();
        for t in ids {
            self.update(&t, |t| {
                t.state = TransferState::Interrupted;
                t.error = Some("device disconnected".into());
            });
        }
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        if p.packet_type == types::SHARE_UPDATE {
            dev.emit("share", "incomingBatch", Value::Object(p.body.clone()));
            return Ok(());
        }
        let body: ShareBody = p.body_as()?;
        if p.has_payload() {
            return self.receive_file(dev, &p, body).await;
        }
        if let Some(text) = body.text {
            let text: String = text.chars().take(1_000_000).collect();
            if self.config.with(|c| c.text_to_clipboard) {
                self.platform.clipboard.set_text(&text).await?;
            }
            dev.emit("share", "text", json!({ "text": text }));
            let _ = self.platform.notifier.show(
                DesktopNotification {
                    id: format!("share-text-{}", dev.id()),
                    app_name: dev.name().into(),
                    title: "Text received".into(),
                    body: text.chars().take(200).collect(),
                    ..Default::default()
                },
                None,
            );
        } else if let Some(url) = body.url {
            // URLs are never opened automatically: the user decides.
            let safe = url.starts_with("https://") || url.starts_with("http://");
            dev.emit("share", "url", json!({ "url": url, "safeScheme": safe }));
            let launcher = self.platform.launcher.clone();
            let open_url = url.clone();
            let actions = if safe {
                vec![("open".to_string(), "Open".to_string())]
            } else {
                vec![]
            };
            let _ = self.platform.notifier.show(
                DesktopNotification {
                    id: format!("share-url-{}", dev.id()),
                    app_name: dev.name().into(),
                    title: "Link received".into(),
                    body: url,
                    actions,
                    ..Default::default()
                },
                Some(Box::new(move |a| {
                    if a == "open" {
                        tokio::runtime::Handle::try_current()
                            .map(|h| {
                                h.spawn(async move {
                                    let _ = launcher.open(&open_url).await;
                                })
                            })
                            .ok();
                    }
                })),
            );
        }
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        match action {
            "sendFiles" => {
                let paths: Vec<PathBuf> =
                    crate::args(args.get("paths").cloned().unwrap_or_default())?;
                if paths.is_empty() {
                    bail!("no files selected");
                }
                let dev = crate::need_dev(dev)?.clone();
                let shared = self.me.upgrade().context("plugin dropped")?;
                shared.send_files(dev, paths).await?;
                Ok(Value::Null)
            }
            "sendText" => {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                self.send_text(crate::need_dev(dev)?, text).await?;
                Ok(Value::Null)
            }
            "sendUrl" => {
                let url = args
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                self.send_url(crate::need_dev(dev)?, url).await?;
                Ok(Value::Null)
            }
            "sendFolder" => {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .context("missing path")?;
                let shared = self.me.upgrade().context("plugin dropped")?;
                shared
                    .send_folder(crate::need_dev(dev)?.clone(), PathBuf::from(path))
                    .await?;
                Ok(Value::Null)
            }
            // Deliver now if the device is connected, otherwise keep it in
            // the outbox until it comes back.
            "queue" => {
                let device_id = args
                    .get("deviceId")
                    .and_then(Value::as_str)
                    .context("missing deviceId")?
                    .to_owned();
                let content: OutboxContent =
                    crate::args(args.get("content").cloned().unwrap_or_default())?;
                if let OutboxContent::Url { url } = &content
                    && !is_web_url(url)
                {
                    bail!("only http and https links with a host can be shared as links");
                }
                let me = self.me.upgrade().context("plugin dropped")?;
                if let Some(d) = self.core.get().and_then(|c| c.device(&device_id)) {
                    me.deliver(d, content).await?;
                    return Ok(json!({ "queued": false }));
                }
                let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
                self.outbox.update(|o| {
                    o.items.push(OutboxItem {
                        id: id.clone(),
                        device_id,
                        content,
                        queued_at: now_millis(),
                    })
                });
                Ok(json!({ "queued": true, "id": id }))
            }
            "outbox" => Ok(serde_json::to_value(self.outbox.get().items)?),
            "outboxRemove" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                self.outbox.update(|o| o.items.retain(|i| i.id != id));
                Ok(Value::Null)
            }
            "transfers" => Ok(serde_json::to_value(&*self.transfers.lock().unwrap())?),
            "clearFinished" => {
                self.transfers
                    .lock()
                    .unwrap()
                    .retain(|t| t.state == TransferState::InProgress);
                Ok(Value::Null)
            }
            "retry" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
                let t = self
                    .transfers
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|t| t.id == id)
                    .cloned()
                    .context("unknown transfer")?;
                if t.direction != Direction::Outgoing || t.state == TransferState::InProgress {
                    bail!("only failed or interrupted outgoing transfers can be retried");
                }
                let path = t.path.context("missing source path")?;
                let d = crate::need_dev(dev)?;
                self.send_file(d, path, 1, t.size).await?;
                Ok(Value::Null)
            }
            "getConfig" => Ok(serde_json::to_value(self.config.get())?),
            "setConfig" => {
                let c: ShareConfig = crate::args(args)?;
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

    #[test]
    fn web_urls() {
        assert!(is_web_url("https://omarchy.org/flux"));
        assert!(is_web_url("http://192.168.1.1"));
        assert!(!is_web_url("file:///etc/passwd"));
        assert!(!is_web_url("https://"));
        assert!(!is_web_url("javascript:alert(1)"));
    }

    #[test]
    fn folder_archive() {
        let src = tempfile::tempdir().unwrap();
        let dir = src.path().join("Progetto");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"ciao").unwrap();
        std::fs::write(dir.join("sub/b.txt"), b"mondo").unwrap();
        let out = tempfile::tempdir().unwrap();
        let archive = archive_folder(&dir, out.path()).unwrap();
        assert!(archive.ends_with("Progetto.tar.gz"));
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(
            std::fs::File::open(&archive).unwrap(),
        ));
        let names: Vec<String> = tar
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().any(|n| n == "Progetto/sub/b.txt"));
    }

    #[test]
    fn sha_validation() {
        assert!(valid_sha(&"a".repeat(64)));
        assert!(!valid_sha("../../x"));
    }
}
