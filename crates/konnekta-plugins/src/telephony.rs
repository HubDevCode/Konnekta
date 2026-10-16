//! Incoming call notifications and call control.
//!
//! KDE Connect only supports "mute ringer". Konnekta adds accept / reject /
//! hang up (`konnekta.telephony.action`) where the phone OS allows it:
//! Android ≥ 9 with ANSWER_PHONE_CALLS; iOS does not allow third-party apps to
//! control calls, so the iOS app does not advertise this capability.

use crate::store::JsonStore;
use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::bodies::{CallAction, CallActionBody, TelephonyBody};
use konnekta_core::proto::{Packet, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
use konnekta_platform::{DesktopNotification, MediaAction, Platform};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TelephonyConfig {
    /// Pause local media while a call is ringing / active.
    pub pause_media: bool,
    /// Show the caller's number (may be private).
    pub show_number: bool,
}

impl Default for TelephonyConfig {
    fn default() -> Self {
        Self {
            pause_media: true,
            show_number: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallState {
    pub event: String,
    pub caller: String,
    pub active: bool,
}

pub struct TelephonyPlugin {
    platform: Platform,
    core: OnceLock<CoreHandle>,
    pub config: JsonStore<TelephonyConfig>,
    calls: Mutex<HashMap<String, CallState>>,
    paused_by_us: Mutex<bool>,
}

impl TelephonyPlugin {
    pub fn new(platform: Platform) -> Self {
        Self {
            platform,
            core: OnceLock::new(),
            config: JsonStore::default(),
            calls: Mutex::default(),
            paused_by_us: Mutex::new(false),
        }
    }

    async fn pause_media(&self) {
        if !self.config.with(|c| c.pause_media) {
            return;
        }
        if let Ok(Some(s)) = self.platform.media.status(None).await
            && s.is_playing
            && self
                .platform
                .media
                .action(None, MediaAction::Pause)
                .await
                .is_ok()
        {
            *self.paused_by_us.lock().unwrap() = true;
        }
    }

    async fn resume_media(&self) {
        let was = std::mem::take(&mut *self.paused_by_us.lock().unwrap());
        if was {
            let _ = self.platform.media.action(None, MediaAction::Play).await;
        }
    }
}

#[async_trait]
impl Plugin for TelephonyPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "telephony",
            name: "Calls",
            description: "Incoming call alerts, mute, answer and reject",
            origin: PluginOrigin::KdeConnect,
            permission: Permission::Telephony,
            incoming: &[types::TELEPHONY],
            outgoing: &[types::TELEPHONY_REQUEST_MUTE, types::K_CALL_ACTION],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "telephony", "config.json");
        let _ = self.core.set(core);
    }

    async fn on_disconnected(&self, id: &str) {
        if self.calls.lock().unwrap().remove(id).is_some() {
            self.resume_media().await;
        }
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        let b: TelephonyBody = p.body_as()?;
        let cfg = self.config.get();
        let caller = b
            .contact_name
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| b.phone_number.clone().filter(|_| cfg.show_number))
            .unwrap_or_else(|| "Unknown caller".into());
        let caller: String = caller.chars().take(64).collect();
        let key = format!("call-{}", dev.id());
        if b.is_cancel {
            self.calls.lock().unwrap().remove(dev.id());
            self.platform.notifier.close(&key);
            dev.emit("telephony", "ended", json!({}));
            self.resume_media().await;
            return Ok(());
        }
        let state = CallState {
            event: b.event.clone(),
            caller: caller.clone(),
            active: b.event != "missedCall",
        };
        dev.emit("telephony", "call", serde_json::to_value(&state)?);
        match b.event.as_str() {
            "ringing" => {
                self.calls
                    .lock()
                    .unwrap()
                    .insert(dev.id().to_owned(), state);
                self.pause_media().await;
                let mut actions = vec![("mute".to_string(), "Mute".to_string())];
                if dev.supports(types::K_CALL_ACTION) {
                    actions.insert(0, ("accept".into(), "Answer".into()));
                    actions.insert(1, ("reject".into(), "Reject".into()));
                }
                let core = self.core.get().cloned();
                let dev_id = dev.id().to_owned();
                self.platform.notifier.show(
                    DesktopNotification {
                        id: key,
                        app_name: dev.name().into(),
                        title: "Incoming call".into(),
                        body: caller,
                        actions,
                        urgent: true,
                    },
                    Some(Box::new(move |a| {
                        let (Some(core), Ok(rt)) = (core, tokio::runtime::Handle::try_current())
                        else {
                            return;
                        };
                        rt.spawn(async move {
                            let _ = core
                                .plugin_action(Some(&dev_id), "telephony", &a, Value::Null)
                                .await;
                        });
                    })),
                )?;
            }
            "talking" => {
                self.calls
                    .lock()
                    .unwrap()
                    .insert(dev.id().to_owned(), state);
                self.platform.notifier.close(&key);
                self.pause_media().await;
            }
            "missedCall" => {
                self.calls.lock().unwrap().remove(dev.id());
                self.platform.notifier.show(
                    DesktopNotification {
                        id: key,
                        app_name: dev.name().into(),
                        title: "Missed call".into(),
                        body: caller,
                        ..Default::default()
                    },
                    None,
                )?;
                self.resume_media().await;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        let call = |a| Packet::new(types::K_CALL_ACTION, CallActionBody { action: a });
        match action {
            "mute" => {
                crate::need_dev(dev)?
                    .send(Packet::empty(types::TELEPHONY_REQUEST_MUTE))
                    .await?
            }
            "accept" | "reject" | "hangup" => {
                let d = crate::need_dev(dev)?;
                if !d.supports(types::K_CALL_ACTION) {
                    bail!("the phone does not allow controlling calls (iOS or missing permission)");
                }
                let a = match action {
                    "accept" => CallAction::Accept,
                    "reject" => CallAction::Reject,
                    _ => CallAction::Hangup,
                };
                d.send(call(a)).await?;
            }
            "state" => {
                let d = crate::need_dev(dev)?;
                return Ok(serde_json::to_value(
                    self.calls.lock().unwrap().get(d.id()),
                )?);
            }
            "getConfig" => return Ok(serde_json::to_value(self.config.get())?),
            "setConfig" => {
                let c: TelephonyConfig = crate::args(args)?;
                self.config.update(|v| *v = c);
            }
            _ => bail!("unknown action {action}"),
        }
        Ok(Value::Null)
    }
}
