//! Audio mixer of the computer (`konnekta.mixer`): the phone sees every
//! output, input and playing app, changes their volume, mutes them, picks the
//! default devices and moves apps between outputs.
//!
//! A `subscribe` request sends the state at once and keeps pushing changes
//! for [`SUBSCRIPTION`]; the phone renews it while the mixer is on screen.

use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::bodies::MixerRequestBody;
use konnekta_core::proto::{Packet, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo};
use konnekta_platform::{MixerKind, MixerState, Platform};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a device keeps receiving changes after its last request.
const SUBSCRIPTION: Duration = Duration::from_secs(60);

pub struct MixerPlugin {
    platform: Platform,
    /// Devices watching the mixer, until when.
    subscribers: Mutex<HashMap<String, Instant>>,
    last_sent: Mutex<HashMap<String, MixerState>>,
    /// Mixer state reported by other devices.
    remote: Mutex<HashMap<String, Value>>,
}

fn kind(s: Option<&str>) -> Result<MixerKind> {
    Ok(match s {
        Some("output") => MixerKind::Output,
        Some("input") => MixerKind::Input,
        Some("app") => MixerKind::App,
        other => bail!("unknown mixer channel kind {other:?}"),
    })
}

impl MixerPlugin {
    pub fn new(platform: Platform) -> Self {
        Self {
            platform,
            subscribers: Mutex::default(),
            last_sent: Mutex::default(),
            remote: Mutex::default(),
        }
    }

    /// Apply the change carried by a request, if any.
    async fn apply(&self, req: &MixerRequestBody) -> Result<()> {
        let Some(id) = req.id else {
            return Ok(());
        };
        let k = kind(req.kind.as_deref())?;
        let m = &self.platform.mixer;
        if let Some(v) = req.volume {
            m.set_volume(k, id, v.min(konnekta_platform::mixer::MAX_VOLUME))
                .await?;
        }
        if let Some(mute) = req.mute {
            m.set_mute(k, id, mute).await?;
        }
        if req.make_default == Some(true) {
            m.set_default(k, id).await?;
        }
        if let Some(out) = req.output {
            if k != MixerKind::App {
                bail!("only apps can move to another output");
            }
            m.move_app(id, out).await?;
        }
        Ok(())
    }

    async fn send_state(&self, dev: &DeviceHandle) -> Result<()> {
        let Some(state) = self.platform.mixer.state().await? else {
            return Ok(());
        };
        self.last_sent
            .lock()
            .unwrap()
            .insert(dev.id().to_owned(), state.clone());
        dev.send(Packet::new(types::MIXER, state)).await
    }

    async fn handle_request(&self, dev: &DeviceHandle, req: MixerRequestBody) -> Result<()> {
        let changed = self.apply(&req).await;
        if req.subscribe == Some(true) || req.id.is_some() {
            self.subscribers
                .lock()
                .unwrap()
                .insert(dev.id().to_owned(), Instant::now() + SUBSCRIPTION);
            // Answer even after a failed change, so the phone shows the truth.
            self.send_state(dev).await?;
        }
        changed
    }

    async fn push_changes(&self, core: &CoreHandle) {
        let now = Instant::now();
        let ids: Vec<String> = {
            let mut subs = self.subscribers.lock().unwrap();
            subs.retain(|_, until| *until > now);
            subs.keys().cloned().collect()
        };
        if ids.is_empty() {
            return;
        }
        let Ok(Some(state)) = self.platform.mixer.state().await else {
            return;
        };
        for id in ids {
            let Some(dev) = core.device(&id) else {
                continue;
            };
            let changed = self.last_sent.lock().unwrap().get(&id) != Some(&state);
            if changed {
                self.last_sent
                    .lock()
                    .unwrap()
                    .insert(id.clone(), state.clone());
                let _ = dev.send(Packet::new(types::MIXER, state.clone())).await;
            }
        }
    }
}

#[async_trait]
impl Plugin for MixerPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "mixer",
            name: "Audio mixer",
            description: "Volume of every output, input and app; default audio devices",
            permission: Permission::MediaControl,
            incoming: &[types::MIXER_REQUEST, types::MIXER],
            outgoing: &[types::MIXER, types::MIXER_REQUEST],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            while core.is_alive() {
                tick.tick().await;
                self.push_changes(&core).await;
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
            types::MIXER_REQUEST => self.handle_request(dev, p.body_as()?).await,
            types::MIXER => {
                let state = Value::Object(p.body.clone());
                dev.emit("mixer", "remoteState", state.clone());
                self.remote
                    .lock()
                    .unwrap()
                    .insert(dev.id().to_owned(), state);
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
        match action {
            // This computer's mixer (for the desktop UI).
            "localState" => Ok(serde_json::to_value(self.platform.mixer.state().await?)?),
            "localSet" => {
                self.apply(&crate::args(args)?).await?;
                Ok(serde_json::to_value(self.platform.mixer.state().await?)?)
            }
            // Mixer of another device.
            "remoteRefresh" | "remoteSet" => {
                let mut req: MixerRequestBody = if action == "remoteSet" {
                    crate::args(args)?
                } else {
                    MixerRequestBody::default()
                };
                req.subscribe = Some(true);
                crate::need_dev(dev)?
                    .send(Packet::new(types::MIXER_REQUEST, req))
                    .await?;
                Ok(Value::Null)
            }
            "remoteState" => {
                let d = crate::need_dev(dev)?;
                Ok(self
                    .remote
                    .lock()
                    .unwrap()
                    .get(d.id())
                    .cloned()
                    .unwrap_or(Value::Null))
            }
            _ => bail!("unknown action {action}"),
        }
    }
}
