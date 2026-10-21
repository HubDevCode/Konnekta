//! Media control in both directions (`konnekta.media`):
//! * the phone controls players on this computer;
//! * this computer controls media playing on the phone.

use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::bodies::{MprisBody, MprisRequestBody};
use konnekta_core::proto::{Packet, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo};
use konnekta_platform::{MediaAction, MediaStatus, Platform};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct MediaPlugin {
    platform: Platform,
    /// Devices that asked for updates, with the player they watch.
    subscribers: Mutex<HashMap<String, Option<String>>>,
    last_sent: Mutex<HashMap<String, (MprisBody, Instant)>>,
    /// Cover last sent to each device (its URL).
    art_sent: Mutex<HashMap<String, String>>,
    /// Media state reported by phones.
    remote: Mutex<HashMap<String, HashMap<String, MprisBody>>>,
}

pub fn status_body(s: &MediaStatus, volume: Option<u8>) -> MprisBody {
    MprisBody {
        player: Some(s.player.clone()),
        title: Some(s.title.clone()),
        artist: Some(s.artist.clone()),
        album: Some(s.album.clone()),
        is_playing: Some(s.is_playing),
        pos: Some(s.position_ms),
        length: Some(s.length_ms),
        volume: volume.map(i32::from),
        album_art_url: (!s.art_url.is_empty()).then(|| s.art_url.clone()),
        can_pause: Some(true),
        can_play: Some(true),
        can_go_next: Some(true),
        can_go_previous: Some(true),
        ..Default::default()
    }
}

/// Worth telling the device: anything but the position moving on by
/// itself (a jump, like a seek on the computer, counts).
fn changed(last: &MprisBody, at: Instant, now: &MprisBody) -> bool {
    let strip = |x: &MprisBody| MprisBody {
        pos: None,
        ..x.clone()
    };
    if strip(last) != strip(now) {
        return true;
    }
    let (Some(p0), Some(p1)) = (last.pos, now.pos) else {
        return false;
    };
    let expected = if now.is_playing == Some(true) {
        p0 + at.elapsed().as_millis() as i64
    } else {
        p0
    };
    (p1 - expected).abs() > 2500
}

/// Bytes of a cover picture: a local file or (with curl) a web address.
async fn load_art(url: &str) -> Option<Vec<u8>> {
    const MAX: u64 = 8 * 1024 * 1024;
    if let Some(path) = url.strip_prefix("file://") {
        let path = percent_decode(path);
        if tokio::fs::metadata(&path).await.ok()?.len() > MAX {
            return None;
        }
        return tokio::fs::read(&path).await.ok();
    }
    if url.starts_with("https://") || url.starts_with("http://") {
        let out = tokio::time::timeout(
            Duration::from_secs(8),
            tokio::process::Command::new("curl")
                .args([
                    "-sfL",
                    "--max-time",
                    "6",
                    "--max-filesize",
                    &MAX.to_string(),
                    url,
                ])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .ok()?
        .ok()?;
        return (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout);
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl MediaPlugin {
    /// The cover of the current track, once per track and device, as the
    /// payload of a `konnekta.media` packet (the phone shows it in its media
    /// controls).
    async fn send_art(&self, dev: &DeviceHandle, b: &MprisBody) {
        let Some(url) = b.album_art_url.clone() else {
            return;
        };
        if self.art_sent.lock().unwrap().get(dev.id()) == Some(&url) {
            return;
        }
        self.art_sent
            .lock()
            .unwrap()
            .insert(dev.id().to_owned(), url.clone());
        let Some(bytes) = load_art(&url).await else {
            return;
        };
        let p = Packet::new(
            types::MEDIA,
            MprisBody {
                player: b.player.clone(),
                album_art_url: Some(url),
                transferring_album_art: Some(true),
                ..Default::default()
            },
        );
        if let Err(e) = dev
            .send_with_payload(p, konnekta_core::payload::PayloadSource::Bytes(bytes), None)
            .await
        {
            tracing::debug!("album art not sent: {e:#}");
        }
    }

    pub fn new(platform: Platform) -> Self {
        Self {
            platform,
            subscribers: Mutex::default(),
            last_sent: Mutex::default(),
            art_sent: Mutex::default(),
            remote: Mutex::default(),
        }
    }

    async fn now_playing(&self, player: Option<&str>) -> Option<MprisBody> {
        let s = self.platform.media.status(player).await.ok().flatten()?;
        let v = self.platform.media.volume().await.ok().flatten();
        Some(status_body(&s, v))
    }

    async fn send_player_list(&self, dev: &DeviceHandle) -> Result<()> {
        let players = self.platform.media.players().await.unwrap_or_default();
        let mut p = Packet::new(
            types::MEDIA,
            MprisBody {
                player_list: Some(players),
                ..Default::default()
            },
        );
        p.body
            .insert("supportAlbumArtPayload".into(), Value::Bool(false));
        dev.send(p).await
    }

    async fn handle_request(&self, dev: &DeviceHandle, req: MprisRequestBody) -> Result<()> {
        let player = req.player.clone().filter(|p| !p.is_empty());
        let m = &self.platform.media;
        if let Some(a) = req.action.as_deref() {
            match MediaAction::parse(a) {
                Some(a) => m.action(player.as_deref(), a).await?,
                None => tracing::debug!("unknown mpris action {a}"),
            }
        }
        if let Some(v) = req.set_volume {
            m.set_volume(v.clamp(0, 100) as u8).await?;
        }
        if let Some(off) = req.seek {
            m.seek(player.as_deref(), off).await?;
        }
        if let Some(pos_ms) = req.set_position
            && let Ok(Some(s)) = m.status(player.as_deref()).await
        {
            m.seek(player.as_deref(), (pos_ms - s.position_ms) * 1000)
                .await?;
        }
        if req.request_player_list == Some(true) {
            self.send_player_list(dev).await?;
        }
        if req.request_now_playing == Some(true)
            || req.request_volume == Some(true)
            || req.action.is_some()
        {
            self.subscribers
                .lock()
                .unwrap()
                .insert(dev.id().to_owned(), player.clone());
            if let Some(b) = self.now_playing(player.as_deref()).await {
                self.last_sent
                    .lock()
                    .unwrap()
                    .insert(dev.id().to_owned(), (b.clone(), Instant::now()));
                dev.send(Packet::new(types::MEDIA, b.clone())).await?;
                self.send_art(dev, &b).await;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Plugin for MediaPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "media",
            name: "Media control",
            description: "Control media playback and volume between devices",
            permission: Permission::MediaControl,
            incoming: &[types::MEDIA_REQUEST, types::MEDIA],
            outgoing: &[types::MEDIA, types::MEDIA_REQUEST],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        // Push now-playing changes to subscribed devices.
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            while core.is_alive() {
                tick.tick().await;
                let subs: Vec<(String, Option<String>)> = self
                    .subscribers
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                for (id, player) in subs {
                    let Some(dev) = core.device(&id) else {
                        continue;
                    };
                    let Some(mut b) = self.now_playing(player.as_deref()).await else {
                        continue;
                    };
                    let changed = self
                        .last_sent
                        .lock()
                        .unwrap()
                        .get(&id)
                        .map(|(l, at)| changed(l, *at, &b))
                        .unwrap_or(true);
                    if changed {
                        self.last_sent
                            .lock()
                            .unwrap()
                            .insert(id.clone(), (b.clone(), Instant::now()));
                        b.player = b.player.or(player);
                        let _ = dev.send(Packet::new(types::MEDIA, b.clone())).await;
                        self.send_art(&dev, &b).await;
                    }
                }
            }
        });
    }

    async fn on_disconnected(&self, id: &str) {
        self.subscribers.lock().unwrap().remove(id);
        self.last_sent.lock().unwrap().remove(id);
        self.art_sent.lock().unwrap().remove(id);
        self.remote.lock().unwrap().remove(id);
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        match p.packet_type.as_str() {
            types::MEDIA_REQUEST => self.handle_request(dev, p.body_as()?).await,
            types::MEDIA => {
                // Phone media state.
                let b: MprisBody = p.body_as()?;
                let key = b.player.clone().unwrap_or_default();
                dev.emit("media", "remoteStatus", Value::Object(p.body.clone()));
                if b.player_list.is_none() {
                    self.remote
                        .lock()
                        .unwrap()
                        .entry(dev.id().to_owned())
                        .or_default()
                        .insert(key, b);
                }
                Ok(())
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
        let player = args
            .get("player")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match action {
            // Local player status (for the desktop UI).
            "localStatus" => Ok(serde_json::to_value(
                self.platform.media.status(player.as_deref()).await?,
            )?),
            "localPlayers" => Ok(serde_json::to_value(self.platform.media.players().await?)?),
            // Control media on the phone.
            "remoteRefresh" => {
                crate::need_dev(dev)?
                    .send(Packet::new(
                        types::MEDIA_REQUEST,
                        MprisRequestBody {
                            request_player_list: Some(true),
                            request_now_playing: Some(true),
                            player,
                            ..Default::default()
                        },
                    ))
                    .await?;
                Ok(Value::Null)
            }
            "remoteControl" => {
                let a = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if MediaAction::parse(a).is_none() {
                    bail!("unknown media action {a}");
                }
                crate::need_dev(dev)?
                    .send(Packet::new(
                        types::MEDIA_REQUEST,
                        MprisRequestBody {
                            player,
                            action: Some(a.to_owned()),
                            ..Default::default()
                        },
                    ))
                    .await?;
                Ok(Value::Null)
            }
            "remoteVolume" => {
                let v = args
                    .get("volume")
                    .and_then(Value::as_i64)
                    .unwrap_or(50)
                    .clamp(0, 100) as i32;
                crate::need_dev(dev)?
                    .send(Packet::new(
                        types::MEDIA_REQUEST,
                        MprisRequestBody {
                            player,
                            set_volume: Some(v),
                            ..Default::default()
                        },
                    ))
                    .await?;
                Ok(Value::Null)
            }
            "remoteStatus" => {
                let d = crate::need_dev(dev)?;
                Ok(serde_json::to_value(
                    self.remote
                        .lock()
                        .unwrap()
                        .get(d.id())
                        .cloned()
                        .unwrap_or_default(),
                )?)
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_jumps_are_changes() {
        let b = |pos, playing| MprisBody {
            title: Some("Song".into()),
            pos: Some(pos),
            is_playing: Some(playing),
            ..Default::default()
        };
        let at = Instant::now();
        // Moving on by itself (or paused in place) is not news.
        assert!(!changed(&b(10_000, true), at, &b(10_300, true)));
        assert!(!changed(&b(10_000, false), at, &b(10_000, false)));
        // A seek is.
        assert!(changed(&b(10_000, true), at, &b(60_000, true)));
        assert!(changed(&b(10_000, false), at, &b(4_000, false)));
        // So is a new state.
        assert!(changed(&b(10_000, true), at, &b(10_000, false)));
    }

    #[test]
    fn art_paths_are_decoded() {
        assert_eq!(
            percent_decode("/home/a/My%20Song%C3%A8.jpg"),
            "/home/a/My Songè.jpg"
        );
        assert_eq!(percent_decode("/bad%2"), "/bad%2");
    }
}
