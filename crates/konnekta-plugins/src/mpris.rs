//! Media control in both directions (KDE Connect `mpris`):
//! * the phone controls players on this computer;
//! * this computer controls media playing on the phone.

use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::bodies::{MprisBody, MprisRequestBody};
use konnekta_core::proto::{Packet, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
use konnekta_platform::{MediaAction, MediaStatus, Platform};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct MprisPlugin {
    platform: Platform,
    /// Devices that asked for updates, with the player they watch.
    subscribers: Mutex<HashMap<String, Option<String>>>,
    last_sent: Mutex<HashMap<String, MprisBody>>,
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
        can_pause: Some(true),
        can_play: Some(true),
        can_go_next: Some(true),
        can_go_previous: Some(true),
        ..Default::default()
    }
}

impl MprisPlugin {
    pub fn new(platform: Platform) -> Self {
        Self {
            platform,
            subscribers: Mutex::default(),
            last_sent: Mutex::default(),
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
            types::MPRIS,
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
                    .insert(dev.id().to_owned(), b.clone());
                dev.send(Packet::new(types::MPRIS, b)).await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Plugin for MprisPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "mpris",
            name: "Media control",
            description: "Control media playback and volume between devices",
            origin: PluginOrigin::KdeConnect,
            permission: Permission::MediaControl,
            incoming: &[types::MPRIS_REQUEST, types::MPRIS],
            outgoing: &[types::MPRIS, types::MPRIS_REQUEST],
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
                    let changed = {
                        let last = self.last_sent.lock().unwrap();
                        // Position always moves while playing: compare without it.
                        let strip = |x: &MprisBody| MprisBody {
                            pos: None,
                            ..x.clone()
                        };
                        last.get(&id).map(|l| strip(l) != strip(&b)).unwrap_or(true)
                    };
                    if changed {
                        self.last_sent.lock().unwrap().insert(id.clone(), b.clone());
                        b.player = b.player.or(player);
                        let _ = dev.send(Packet::new(types::MPRIS, b)).await;
                    }
                }
            }
        });
    }

    async fn on_disconnected(&self, id: &str) {
        self.subscribers.lock().unwrap().remove(id);
        self.last_sent.lock().unwrap().remove(id);
        self.remote.lock().unwrap().remove(id);
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        match p.packet_type.as_str() {
            types::MPRIS_REQUEST => self.handle_request(dev, p.body_as()?).await,
            types::MPRIS => {
                // Phone media state.
                let b: MprisBody = p.body_as()?;
                let key = b.player.clone().unwrap_or_default();
                dev.emit("mpris", "remoteStatus", Value::Object(p.body.clone()));
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
                        types::MPRIS_REQUEST,
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
                        types::MPRIS_REQUEST,
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
                        types::MPRIS_REQUEST,
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
