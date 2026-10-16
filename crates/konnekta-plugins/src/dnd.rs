//! Do Not Disturb synchronisation between phone and computer.

use crate::store::JsonStore;
use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::{Packet, bodies::DndBody, packet::now_millis, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
use konnekta_platform::Platform;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Priority {
    /// The most recent change wins.
    Latest,
    /// On simultaneous changes the phone wins.
    Phone,
    /// On simultaneous changes the computer wins.
    Computer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DndConfig {
    pub sync: bool,
    pub priority: Priority,
    /// Two changes closer than this are "simultaneous".
    pub conflict_window_ms: i64,
}

impl Default for DndConfig {
    fn default() -> Self {
        Self { sync: true, priority: Priority::Latest, conflict_window_ms: 5000 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Apply the remote state locally.
    Apply,
    /// Keep ours and tell the peer.
    KeepLocal,
    /// Nothing to do.
    Ignore,
}

/// Conflict resolution (pure function, unit tested).
pub fn resolve(cfg: &DndConfig, local: Option<(bool, i64)>, remote: &DndBody) -> Decision {
    let Some((local_on, local_at)) = local else { return Decision::Apply };
    if local_on == remote.enabled {
        return Decision::Ignore;
    }
    if !remote.user_initiated {
        // A state report (e.g. on connect): newer state wins.
        return if remote.changed_at > local_at { Decision::Apply } else { Decision::KeepLocal };
    }
    let simultaneous = (remote.changed_at - local_at).abs() <= cfg.conflict_window_ms;
    match (cfg.priority, simultaneous) {
        (Priority::Computer, true) => Decision::KeepLocal,
        (Priority::Phone, true) => Decision::Apply,
        _ if remote.changed_at >= local_at => Decision::Apply,
        _ => Decision::KeepLocal,
    }
}

pub struct DndPlugin {
    platform: Platform,
    pub config: JsonStore<DndConfig>,
    /// Last known local state and when it changed.
    local: Mutex<Option<(bool, i64)>>,
    core: std::sync::OnceLock<CoreHandle>,
}

impl DndPlugin {
    pub fn new(platform: Platform) -> Self {
        Self { platform, config: JsonStore::default(), local: Mutex::new(None), core: Default::default() }
    }

    fn body(&self, user: bool) -> Option<DndBody> {
        self.local.lock().unwrap().map(|(enabled, at)| DndBody { enabled, changed_at: at, user_initiated: user })
    }

    async fn broadcast(&self, user: bool) {
        let (Some(core), Some(b)) = (self.core.get(), self.body(user)) else { return };
        for d in core.devices() {
            if d.has_permission(Permission::DoNotDisturb) {
                let _ = d.send_if_supported(Packet::new(types::K_DND, b.clone())).await;
            }
        }
    }

    async fn set_local(&self, enabled: bool, at: i64) -> Result<()> {
        self.platform.dnd.set(enabled).await?;
        *self.local.lock().unwrap() = Some((enabled, at));
        Ok(())
    }
}

#[async_trait]
impl Plugin for DndPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "dnd",
            name: "Do Not Disturb sync",
            description: "Keep Do Not Disturb in sync between phone and computer",
            origin: PluginOrigin::Konnekta,
            permission: Permission::DoNotDisturb,
            incoming: &[types::K_DND],
            outgoing: &[types::K_DND],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "dnd", "config.json");
        let _ = self.core.set(core.clone());
        if !self.platform.dnd.supported() {
            return;
        }
        if let Ok(Some(v)) = self.platform.dnd.get().await {
            *self.local.lock().unwrap() = Some((v, now_millis()));
        }
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            while core.is_alive() {
                tick.tick().await;
                let Ok(Some(v)) = self.platform.dnd.get().await else { continue };
                let changed = {
                    let mut l = self.local.lock().unwrap();
                    if l.map(|(x, _)| x) != Some(v) {
                        *l = Some((v, now_millis()));
                        true
                    } else {
                        false
                    }
                };
                if changed && self.config.with(|c| c.sync) {
                    self.broadcast(true).await;
                }
            }
        });
    }

    async fn on_connected(&self, dev: &DeviceHandle) {
        if let Some(b) = self.body(false) {
            let _ = dev.send_if_supported(Packet::new(types::K_DND, b)).await;
        }
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        let remote: DndBody = p.body_as()?;
        dev.emit("dnd", "remoteState", json!({ "enabled": remote.enabled }));
        let cfg = self.config.get();
        if !cfg.sync || !self.platform.dnd.supported() {
            return Ok(());
        }
        let local = *self.local.lock().unwrap();
        match resolve(&cfg, local, &remote) {
            Decision::Apply => self.set_local(remote.enabled, remote.changed_at).await?,
            Decision::KeepLocal => {
                if let Some(b) = self.body(true) {
                    dev.send(Packet::new(types::K_DND, b)).await?;
                }
            }
            Decision::Ignore => {}
        }
        Ok(())
    }

    async fn handle_action(&self, _: Option<&DeviceHandle>, action: &str, args: Value) -> Result<Value> {
        match action {
            "get" => Ok(json!({ "enabled": self.local.lock().unwrap().map(|(v, _)| v), "supported": self.platform.dnd.supported() })),
            "set" | "toggle" => {
                let target = match action {
                    "set" => args.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                    _ => !self.local.lock().unwrap().map(|(v, _)| v).unwrap_or(false),
                };
                if !self.platform.dnd.supported() {
                    bail!("Do Not Disturb cannot be controlled on this desktop");
                }
                self.set_local(target, now_millis()).await?;
                if self.config.with(|c| c.sync) {
                    self.broadcast(true).await;
                }
                Ok(json!({ "enabled": target }))
            }
            "getConfig" => Ok(serde_json::to_value(self.config.get())?),
            "setConfig" => {
                let c: DndConfig = crate::args(args)?;
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

    fn r(enabled: bool, at: i64, user: bool) -> DndBody {
        DndBody { enabled, changed_at: at, user_initiated: user }
    }

    #[test]
    fn conflict_resolution() {
        let mut c = DndConfig::default();
        assert_eq!(resolve(&c, None, &r(true, 1, true)), Decision::Apply);
        assert_eq!(resolve(&c, Some((true, 1)), &r(true, 9, true)), Decision::Ignore);
        // Latest wins.
        assert_eq!(resolve(&c, Some((false, 1000)), &r(true, 2000, true)), Decision::Apply);
        assert_eq!(resolve(&c, Some((false, 9000)), &r(true, 2000, true)), Decision::KeepLocal);
        // Simultaneous with explicit priority.
        c.priority = Priority::Computer;
        assert_eq!(resolve(&c, Some((false, 1000)), &r(true, 2000, true)), Decision::KeepLocal);
        c.priority = Priority::Phone;
        assert_eq!(resolve(&c, Some((false, 3000)), &r(true, 2000, true)), Decision::Apply);
        // State reports: newer wins regardless of priority.
        assert_eq!(resolve(&c, Some((false, 3000)), &r(true, 2000, false)), Decision::KeepLocal);
    }
}
