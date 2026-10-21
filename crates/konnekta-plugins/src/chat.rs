//! Chat view: one thread per device with what was exchanged in this session
//! (texts, links, clipboard, files, images), newest last. Memory only; each
//! text is cut to 400 characters. Typing in the chat sends a link when the
//! text is an http(s) URL, otherwise text (it lands on the device clipboard).

use crate::share::is_web_url;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::{Packet, packet::now_millis};
use konnekta_core::{CoreHandle, DeviceHandle, Event, Permission, Plugin, PluginInfo};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub const MAX_ENTRIES: usize = 200;
const MAX_TEXT: usize = 400;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatEntry {
    pub id: u64,
    /// "in" (from the device) or "out" (from this computer).
    pub dir: &'static str,
    /// text, url, clip, file, image, notice
    pub kind: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    pub at: i64,
}

#[derive(Default)]
pub struct ChatPlugin {
    threads: Mutex<HashMap<String, VecDeque<ChatEntry>>>,
    next: AtomicU64,
    core: OnceLock<CoreHandle>,
}

fn cut(s: &str) -> String {
    s.chars().take(MAX_TEXT).collect()
}

impl ChatPlugin {
    fn push(&self, device: &str, mut e: ChatEntry) -> ChatEntry {
        e.id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        e.text = cut(&e.text);
        let mut t = self.threads.lock().unwrap();
        let th = t.entry(device.to_owned()).or_default();
        // A transfer updates its own entry instead of adding a new one.
        if let Some(tid) = &e.transfer_id
            && let Some(old) = th.iter_mut().find(|x| x.transfer_id.as_ref() == Some(tid))
        {
            old.state = e.state.clone();
            return old.clone();
        }
        th.push_back(e.clone());
        while th.len() > MAX_ENTRIES {
            th.pop_front();
        }
        e
    }

    /// Map a plugin event to a chat entry.
    pub fn entry_for(plugin: &str, kind: &str, data: &Value) -> Option<ChatEntry> {
        let s = |k: &str| {
            data.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let e = |dir, kind, text: String| ChatEntry {
            id: 0,
            dir,
            kind,
            text,
            transfer_id: None,
            state: None,
            at: now_millis(),
        };
        Some(match (plugin, kind) {
            ("share", "text") => e("in", "text", s("text")),
            ("share", "url") => e("in", "url", s("url")),
            ("share", "sent") => {
                let k = if s("kind") == "url" { "url" } else { "text" };
                e("out", k, s("text"))
            }
            ("share", "transfer") => {
                let dir = if s("direction") == "incoming" {
                    "in"
                } else {
                    "out"
                };
                ChatEntry {
                    transfer_id: Some(s("id")),
                    state: Some(s("state")),
                    ..e(dir, "file", s("name"))
                }
            }
            ("clipboard", "received") => e("in", "clip", s("text")),
            ("clipboard", "image") => e("in", "image", "Immagine negli appunti".into()),
            ("screenshot", "received") => e("in", "image", "Screenshot".into()),
            ("ping", "received") => e("in", "notice", s("message")),
            _ => return None,
        })
    }
}

#[async_trait]
impl Plugin for ChatPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "chat",
            name: "Chat",
            description: "Thread of texts, links, clips and files exchanged with each device",
            permission: Permission::Ping,
            incoming: &[],
            outgoing: &[],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        let _ = self.core.set(core.clone());
        let Some(mut rx) = core.subscribe() else {
            return;
        };
        tokio::spawn(async move {
            loop {
                let ev = match rx.recv().await {
                    Ok(ev) => ev,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                match ev {
                    Event::Plugin {
                        device_id: Some(d),
                        plugin,
                        kind,
                        data,
                    } if plugin != "chat" => {
                        if let Some(e) = Self::entry_for(&plugin, &kind, &data) {
                            let e = self.push(&d, e);
                            core.emit_plugin(
                                Some(&d),
                                "chat",
                                "entry",
                                serde_json::to_value(&e).unwrap_or_default(),
                            );
                        }
                    }
                    Event::Unpaired { device_id } => {
                        self.threads.lock().unwrap().remove(&device_id);
                    }
                    _ => {}
                }
            }
        });
    }

    async fn handle_packet(&self, _: &DeviceHandle, _: Packet) -> Result<()> {
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        let device_id = dev
            .map(|d| d.id().to_owned())
            .or_else(|| {
                args.get("deviceId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .context("missing device")?;
        match action {
            "thread" => {
                let t = self.threads.lock().unwrap();
                Ok(serde_json::to_value(
                    t.get(&device_id).cloned().unwrap_or_default(),
                )?)
            }
            "clear" => {
                self.threads.lock().unwrap().remove(&device_id);
                Ok(Value::Null)
            }
            "send" => {
                let mut text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if text.is_empty() {
                    bail!("empty message");
                }
                // "omarchy.org/flux" -> "https://omarchy.org/flux"
                let looks_like_host = !text.contains(char::is_whitespace)
                    && !text.contains("://")
                    && text.split('/').next().is_some_and(|h| {
                        h.contains('.') && !h.starts_with('.') && !h.ends_with('.')
                    });
                if looks_like_host {
                    text = format!("https://{text}");
                }
                let core = self.core.get().context("not started")?;
                // Delivered now, or queued until the device is back.
                let content = if is_web_url(&text) {
                    json!({ "kind": "url", "url": text })
                } else {
                    json!({ "kind": "text", "text": text })
                };
                core.plugin_action(
                    None,
                    "share",
                    "queue",
                    json!({ "deviceId": device_id, "content": content }),
                )
                .await
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_events_and_updates_transfers() {
        let chat = ChatPlugin::default();
        let e = ChatPlugin::entry_for("share", "url", &json!({"url": "https://x.org"})).unwrap();
        assert_eq!((e.dir, e.kind), ("in", "url"));
        let t1 = ChatPlugin::entry_for(
            "share",
            "transfer",
            &json!({"id": "t", "name": "a.pdf", "state": "inProgress", "direction": "outgoing"}),
        )
        .unwrap();
        let t2 = ChatPlugin::entry_for(
            "share",
            "transfer",
            &json!({"id": "t", "name": "a.pdf", "state": "completed", "direction": "outgoing"}),
        )
        .unwrap();
        chat.push("d", t1);
        chat.push("d", t2);
        let th = chat.threads.lock().unwrap();
        assert_eq!(th["d"].len(), 1);
        assert_eq!(th["d"][0].state.as_deref(), Some("completed"));
        assert!(ChatPlugin::entry_for("media", "remoteStatus", &json!({})).is_none());
    }

    #[test]
    fn text_is_cut_and_thread_bounded() {
        let chat = ChatPlugin::default();
        for _ in 0..(MAX_ENTRIES + 10) {
            chat.push(
                "d",
                ChatPlugin::entry_for("share", "text", &json!({"text": "x".repeat(1000)})).unwrap(),
            );
        }
        let th = chat.threads.lock().unwrap();
        assert_eq!(th["d"].len(), MAX_ENTRIES);
        assert_eq!(th["d"][0].text.chars().count(), MAX_TEXT);
    }
}
