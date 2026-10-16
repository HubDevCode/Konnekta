//! Phone notifications on the computer (KDE Connect `notification`).
//!
//! Supports dismissing, replying (when the phone app offers a reply action)
//! and triggering notification actions, with per-app filters, keyword muting
//! and a privacy mode that hides the content on the desktop.

use crate::store::JsonStore;
use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::{Packet, bodies::NotificationBody, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
use konnekta_platform::{DesktopNotification, Platform};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationFilter {
    /// Apps never shown.
    pub blocked_apps: BTreeSet<String>,
    /// If non-empty, only these apps are shown.
    pub allowed_apps: BTreeSet<String>,
    /// Notifications containing any of these words (case-insensitive) are hidden.
    pub muted_keywords: Vec<String>,
    /// Ignore notifications the phone marks as silent.
    pub hide_silent: bool,
    /// Show "New notification from X" without title/body.
    pub privacy_mode: bool,
    /// Show desktop popups (otherwise only the in-app list).
    pub show_popups: bool,
}

impl Default for NotificationFilter {
    fn default() -> Self {
        Self {
            blocked_apps: BTreeSet::new(),
            allowed_apps: BTreeSet::new(),
            muted_keywords: Vec::new(),
            hide_silent: true,
            privacy_mode: false,
            show_popups: true,
        }
    }
}

impl NotificationFilter {
    pub fn accepts(&self, n: &NotificationBody) -> bool {
        if self.blocked_apps.contains(&n.app_name) {
            return false;
        }
        if !self.allowed_apps.is_empty() && !self.allowed_apps.contains(&n.app_name) {
            return false;
        }
        if self.hide_silent && n.silent {
            return false;
        }
        let text = format!("{} {}", n.title, n.text).to_lowercase();
        !self
            .muted_keywords
            .iter()
            .any(|k| !k.is_empty() && text.contains(&k.to_lowercase()))
    }
}

pub struct NotificationsPlugin {
    platform: Platform,
    core: OnceLock<CoreHandle>,
    pub filter: JsonStore<NotificationFilter>,
    /// device -> id -> notification
    active: Mutex<HashMap<String, HashMap<String, NotificationBody>>>,
}

impl NotificationsPlugin {
    pub fn new(platform: Platform) -> Self {
        Self {
            platform,
            core: OnceLock::new(),
            filter: JsonStore::default(),
            active: Mutex::default(),
        }
    }

    fn sanitize(mut n: NotificationBody) -> NotificationBody {
        let cut = |s: &str, max: usize| {
            s.chars()
                .filter(|c| !c.is_control() || *c == '\n')
                .take(max)
                .collect::<String>()
        };
        n.app_name = cut(&n.app_name, 64);
        n.title = cut(&n.title, 256);
        n.text = cut(&n.text, 4096);
        n.id = cut(&n.id, 256);
        if let Some(a) = n.actions.as_mut() {
            a.truncate(8);
        }
        n
    }
}

#[async_trait]
impl Plugin for NotificationsPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "notifications",
            name: "Notifications",
            description: "Show phone notifications on the computer, reply and dismiss",
            origin: PluginOrigin::KdeConnect,
            permission: Permission::Notifications,
            incoming: &[types::NOTIFICATION],
            outgoing: &[
                types::NOTIFICATION_REQUEST,
                types::NOTIFICATION_REPLY,
                types::NOTIFICATION_ACTION,
            ],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.filter.attach(&core, "notifications", "filter.json");
        let _ = self.core.set(core);
    }

    async fn on_connected(&self, dev: &DeviceHandle) {
        // Ask for the notifications already present on the phone.
        let _ = dev
            .send_if_supported(Packet::new(
                types::NOTIFICATION_REQUEST,
                json!({ "request": true }),
            ))
            .await;
    }

    async fn on_disconnected(&self, id: &str) {
        if let Some(list) = self.active.lock().unwrap().remove(id) {
            for nid in list.keys() {
                self.platform.notifier.close(&format!("{id}:{nid}"));
            }
        }
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        let n = Self::sanitize(p.body_as::<NotificationBody>()?);
        let key = format!("{}:{}", dev.id(), n.id);
        if n.is_cancel {
            if let Some(m) = self.active.lock().unwrap().get_mut(dev.id()) {
                m.remove(&n.id);
            }
            self.platform.notifier.close(&key);
            dev.emit("notifications", "removed", json!({ "id": n.id }));
            return Ok(());
        }
        let filter = self.filter.get();
        if !filter.accepts(&n) {
            return Ok(());
        }
        let is_new = self
            .active
            .lock()
            .unwrap()
            .entry(dev.id().to_owned())
            .or_default()
            .insert(n.id.clone(), n.clone())
            .is_none();
        dev.emit("notifications", "posted", serde_json::to_value(&n)?);
        if filter.show_popups && (is_new || !n.only_once) {
            let (title, body) = if filter.privacy_mode {
                (
                    format!("New notification from {}", n.app_name),
                    String::new(),
                )
            } else {
                (n.title.clone(), n.text.clone())
            };
            let mut actions: Vec<(String, String)> = Vec::new();
            if n.is_clearable {
                actions.push(("dismiss".into(), "Dismiss".into()));
            }
            if n.request_reply_id.is_some() {
                actions.push(("reply".into(), "Reply…".into()));
            }
            for a in n.actions.clone().unwrap_or_default() {
                actions.push((format!("action:{a}"), a));
            }
            let core = self.core.get().cloned();
            let (dev_id, nid) = (dev.id().to_owned(), n.id.clone());
            self.platform.notifier.show(
                DesktopNotification {
                    id: key,
                    app_name: n.app_name.clone(),
                    title,
                    body,
                    actions,
                    urgent: false,
                },
                Some(Box::new(move |a| {
                    let Some(core) = core else { return };
                    let Ok(rt) = tokio::runtime::Handle::try_current() else {
                        return;
                    };
                    rt.spawn(async move {
                        let r = match a.as_str() {
                            "dismiss" => {
                                core.plugin_action(
                                    Some(&dev_id),
                                    "notifications",
                                    "dismiss",
                                    json!({ "id": nid }),
                                )
                                .await
                            }
                            "reply" => {
                                core.emit_plugin(
                                    Some(&dev_id),
                                    "notifications",
                                    "replyRequested",
                                    json!({ "id": nid }),
                                );
                                Ok(Value::Null)
                            }
                            other => match other.strip_prefix("action:") {
                                Some(name) => {
                                    core.plugin_action(
                                        Some(&dev_id),
                                        "notifications",
                                        "action",
                                        json!({ "id": nid, "action": name }),
                                    )
                                    .await
                                }
                                None => Ok(Value::Null),
                            },
                        };
                        if let Err(e) = r {
                            tracing::warn!("notification action failed: {e}");
                        }
                    });
                })),
            )?;
        }
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match action {
            "list" => {
                let d = crate::need_dev(dev)?;
                let v: Vec<NotificationBody> = self
                    .active
                    .lock()
                    .unwrap()
                    .get(d.id())
                    .map(|m| m.values().cloned().collect())
                    .unwrap_or_default();
                Ok(serde_json::to_value(v)?)
            }
            "dismiss" => {
                let d = crate::need_dev(dev)?;
                d.send(Packet::new(
                    types::NOTIFICATION_REQUEST,
                    json!({ "cancel": id }),
                ))
                .await?;
                if let Some(m) = self.active.lock().unwrap().get_mut(d.id()) {
                    m.remove(&id);
                }
                self.platform.notifier.close(&format!("{}:{id}", d.id()));
                Ok(Value::Null)
            }
            "reply" => {
                let d = crate::need_dev(dev)?;
                let message = args
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let reply_id = self
                    .active
                    .lock()
                    .unwrap()
                    .get(d.id())
                    .and_then(|m| m.get(&id))
                    .and_then(|n| n.request_reply_id.clone());
                let Some(reply_id) = reply_id else {
                    bail!("this notification does not support replies")
                };
                d.send(Packet::new(
                    types::NOTIFICATION_REPLY,
                    json!({ "requestReplyId": reply_id, "message": message }),
                ))
                .await?;
                Ok(Value::Null)
            }
            "action" => {
                let d = crate::need_dev(dev)?;
                let name = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                d.send(Packet::new(
                    types::NOTIFICATION_ACTION,
                    json!({ "key": id, "action": name }),
                ))
                .await?;
                Ok(Value::Null)
            }
            "getFilter" => Ok(serde_json::to_value(self.filter.get())?),
            "setFilter" => {
                let f: NotificationFilter = crate::args(args)?;
                self.filter.update(|v| *v = f);
                Ok(Value::Null)
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(app: &str, title: &str) -> NotificationBody {
        NotificationBody {
            id: "1".into(),
            app_name: app.into(),
            title: title.into(),
            ..Default::default()
        }
    }

    #[test]
    fn filters() {
        let mut f = NotificationFilter::default();
        assert!(f.accepts(&n("WhatsApp", "hi")));
        f.blocked_apps.insert("Facebook".into());
        assert!(!f.accepts(&n("Facebook", "hi")));
        f.muted_keywords.push("PROMO".into());
        assert!(!f.accepts(&n("Shop", "big promo today")));
        f.allowed_apps.insert("Signal".into());
        assert!(!f.accepts(&n("WhatsApp", "hi")));
        assert!(f.accepts(&n("Signal", "hi")));
        let mut silent = n("Signal", "x");
        silent.silent = true;
        assert!(!f.accepts(&silent));
    }
}
