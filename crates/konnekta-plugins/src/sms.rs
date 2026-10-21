//! Text messages of the phone: read conversations and send SMS from the
//! computer. Same packets as KDE Connect (`sms.*`), so it also works with the
//! KDE Connect Android app in compatibility mode.
//!
//! Messages live in memory only and are dropped when the phone disconnects;
//! at most [`MAX_PER_THREAD`] per conversation and [`MAX_THREADS`] threads.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::{Packet, types};
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

pub const MAX_PER_THREAD: usize = 500;
pub const MAX_THREADS: usize = 300;
pub const MAX_BODY: usize = 5000;
const PAGE: i64 = 50;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Address {
    pub address: String,
}

/// One message, KDE Connect field names. Unknown fields are ignored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    #[serde(rename = "_id", default)]
    pub id: i64,
    #[serde(rename = "thread_id", default)]
    pub thread_id: i64,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub addresses: Vec<Address>,
    #[serde(default)]
    pub date: i64,
    /// 1 received, 2 sent (Android `Telephony.TextBasedSmsColumns`).
    #[serde(rename = "type", default)]
    pub kind: i32,
    #[serde(default)]
    pub read: i32,
}

impl Message {
    pub fn outgoing(&self) -> bool {
        self.kind == 2
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub thread_id: i64,
    pub addresses: Vec<String>,
    pub last: Message,
    pub unread: bool,
}

type Threads = HashMap<i64, BTreeMap<(i64, i64), Message>>;

#[derive(Default)]
pub struct SmsPlugin {
    /// device id -> thread id -> (date, id) -> message
    store: Mutex<HashMap<String, Threads>>,
}

impl SmsPlugin {
    /// Merge messages from the phone; returns the thread ids that changed.
    pub fn merge(&self, device: &str, msgs: Vec<Message>) -> Vec<i64> {
        let mut s = self.store.lock().unwrap();
        let threads = s.entry(device.to_owned()).or_default();
        let mut changed = Vec::new();
        for mut m in msgs {
            if m.thread_id == 0 && m.id == 0 {
                continue;
            }
            m.body = m.body.chars().take(MAX_BODY).collect();
            let t = threads.entry(m.thread_id).or_default();
            if t.insert((m.date, m.id), m.clone()).as_ref() != Some(&m)
                && !changed.contains(&m.thread_id)
            {
                changed.push(m.thread_id);
            }
            while t.len() > MAX_PER_THREAD {
                t.pop_first();
            }
        }
        // Too many threads: forget the oldest ones.
        while threads.len() > MAX_THREADS {
            let oldest = threads
                .iter()
                .min_by_key(|(_, t)| t.last_key_value().map(|(k, _)| k.0).unwrap_or(0))
                .map(|(id, _)| *id);
            match oldest {
                Some(id) => threads.remove(&id),
                None => break,
            };
        }
        changed
    }

    pub fn conversations(&self, device: &str) -> Vec<Conversation> {
        let s = self.store.lock().unwrap();
        let mut out: Vec<Conversation> = s
            .get(device)
            .into_iter()
            .flatten()
            .filter_map(|(id, t)| {
                let last = t.last_key_value()?.1.clone();
                let mut addresses: Vec<String> = Vec::new();
                for m in t.values() {
                    for a in &m.addresses {
                        if !addresses.contains(&a.address) {
                            addresses.push(a.address.clone());
                        }
                    }
                }
                Some(Conversation {
                    thread_id: *id,
                    addresses,
                    unread: t.values().any(|m| !m.outgoing() && m.read == 0),
                    last,
                })
            })
            .collect();
        out.sort_by_key(|c| std::cmp::Reverse(c.last.date));
        out
    }

    pub fn thread(&self, device: &str, thread_id: i64) -> Vec<Message> {
        let s = self.store.lock().unwrap();
        s.get(device)
            .and_then(|t| t.get(&thread_id))
            .map(|t| t.values().cloned().collect())
            .unwrap_or_default()
    }
}

#[async_trait]
impl Plugin for SmsPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "sms",
            name: "Messages",
            description: "Read and send the phone's text messages",
            permission: Permission::Messages,
            incoming: &[types::SMS_MESSAGES],
            outgoing: &[
                types::SMS_SEND,
                types::SMS_CONVERSATIONS_REQUEST,
                types::SMS_THREAD_REQUEST,
            ],
        }
    }

    async fn on_disconnected(&self, id: &str) {
        self.store.lock().unwrap().remove(id);
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        let msgs: Vec<Message> = p
            .body
            .get("messages")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| serde_json::from_value(m.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        let changed = self.merge(dev.id(), msgs);
        if !changed.is_empty() {
            dev.emit("sms", "updated", json!({ "threads": changed }));
        }
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        let dev = crate::need_dev(dev)?;
        let thread_id = || {
            args.get("threadId")
                .and_then(Value::as_i64)
                .context("missing threadId")
        };
        match action {
            // Cached list now; the phone's answer arrives as an "updated" event.
            "conversations" => {
                dev.send(Packet::new(types::SMS_CONVERSATIONS_REQUEST, json!({})))
                    .await?;
                Ok(serde_json::to_value(self.conversations(dev.id()))?)
            }
            "conversation" => {
                let id = thread_id()?;
                let cached = self.thread(dev.id(), id);
                // Older page: start from the oldest message we have.
                let mut body = json!({ "threadID": id, "numberToRequest": PAGE });
                if args.get("older").and_then(Value::as_bool) == Some(true)
                    && let Some(first) = cached.first()
                {
                    body["rangeStartTimestamp"] = json!(first.date);
                }
                dev.send(Packet::new(types::SMS_THREAD_REQUEST, body))
                    .await?;
                Ok(serde_json::to_value(cached)?)
            }
            "send" => {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                if text.is_empty() {
                    bail!("empty message");
                }
                if text.chars().count() > MAX_BODY {
                    bail!("message too long");
                }
                // Recipients: explicit numbers, or those of an existing thread.
                let mut addresses: Vec<String> = args
                    .get("addresses")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                if addresses.is_empty() {
                    let id = thread_id()?;
                    addresses = self
                        .conversations(dev.id())
                        .into_iter()
                        .find(|c| c.thread_id == id)
                        .map(|c| c.addresses)
                        .context("unknown conversation")?;
                }
                if addresses.is_empty() || addresses.len() > 20 {
                    bail!("invalid recipients");
                }
                let addrs: Vec<Value> = addresses.iter().map(|a| json!({ "address": a })).collect();
                dev.send(Packet::new(
                    types::SMS_SEND,
                    json!({ "version": 2, "addresses": addrs, "messageBody": text }),
                ))
                .await?;
                Ok(Value::Null)
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: i64, thread: i64, date: i64, kind: i32, body: &str) -> Message {
        serde_json::from_value(json!({
            "_id": id, "thread_id": thread, "date": date, "type": kind, "read": 0,
            "body": body, "addresses": [{ "address": "+39 333 0000000" }], "event": 1, "sub_id": 1
        }))
        .unwrap()
    }

    #[test]
    fn merge_dedups_and_orders_conversations() {
        let p = SmsPlugin::default();
        assert_eq!(
            p.merge(
                "d",
                vec![msg(1, 7, 100, 1, "ciao"), msg(2, 8, 300, 2, "ok")]
            ),
            vec![7, 8]
        );
        // Same message again: nothing changed.
        assert!(p.merge("d", vec![msg(1, 7, 100, 1, "ciao")]).is_empty());
        p.merge("d", vec![msg(3, 7, 400, 1, "a dopo")]);
        let c = p.conversations("d");
        assert_eq!(c.iter().map(|c| c.thread_id).collect::<Vec<_>>(), [7, 8]);
        assert_eq!(c[0].last.body, "a dopo");
        assert!(c[0].unread && !c[1].unread);
        assert_eq!(
            p.thread("d", 7).iter().map(|m| m.id).collect::<Vec<_>>(),
            [1, 3]
        );
    }

    #[test]
    fn bounded_per_thread() {
        let p = SmsPlugin::default();
        p.merge(
            "d",
            (0..(MAX_PER_THREAD as i64 + 20))
                .map(|i| msg(i, 1, i, 1, "x"))
                .collect(),
        );
        let t = p.thread("d", 1);
        assert_eq!(t.len(), MAX_PER_THREAD);
        assert_eq!(t[0].id, 20, "oldest dropped");
    }
}
