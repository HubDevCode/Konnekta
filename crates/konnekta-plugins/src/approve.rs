//! Carry `sudo` / polkit approvals and key enrollments between the local
//! helper (`konnekta-approve`, `konnekta approve enroll`) and the phone.
//!
//! This plugin decides nothing: the helper builds the request and verifies
//! the phone's signature with the root-owned key; the phone shows the
//! request and signs after a biometric check. Here we only validate the
//! fields, allow one open request per phone, and close it on timeout,
//! cancel or disconnect.
//!
//! Packets: `konnekta.approve.request` (computer -> phone)
//! `{"kind": "approve"|"enroll"|"cancel", "id", <fields>, "timeout"}` and
//! `konnekta.approve` (phone -> computer) `{"id", "result":
//! "approved"|"denied"|"error", "signature", "publicKey", "error"}`.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use konnekta_approve::message::{Approval, Enrollment};
use konnekta_core::proto::{Packet, types};
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

pub const MIN_TIMEOUT: u64 = 5;
pub const MAX_TIMEOUT: u64 = 120;
/// Signatures and keys are small; anything larger is refused.
const MAX_B64: usize = 2048;

struct Pending {
    id: String,
    deadline: Instant,
    tx: Option<oneshot::Sender<Value>>,
    rx: Option<oneshot::Receiver<Value>>,
}

#[derive(Default)]
pub struct ApprovePlugin {
    /// device id -> its one open request
    pending: Mutex<HashMap<String, Pending>>,
}

fn s<'a>(a: &'a Value, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).unwrap_or_default()
}

/// Only the expected answer fields, with bounded sizes.
fn clean_answer(body: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    // Only the three known results; anything else counts as an error.
    let result = body
        .get("result")
        .and_then(Value::as_str)
        .filter(|r| matches!(*r, "approved" | "denied" | "error"))
        .unwrap_or("error");
    out.insert("result".into(), json!(result));
    for k in ["signature", "publicKey"] {
        if let Some(v) = body
            .get(k)
            .and_then(Value::as_str)
            .filter(|v| v.len() <= MAX_B64)
        {
            out.insert(k.into(), json!(v));
        }
    }
    if let Some(e) = body.get("error").and_then(Value::as_str) {
        let e: String = e.chars().filter(|c| !c.is_control()).take(200).collect();
        out.insert("error".into(), json!(e));
    }
    Value::Object(out)
}

impl ApprovePlugin {
    /// Validated request body for the phone.
    fn request_body(kind: &str, id: &str, a: &Value, timeout: u64) -> Result<Value> {
        let time = a
            .get("time")
            .and_then(Value::as_u64)
            .context("missing time")?;
        let mut body = match kind {
            "approve" => {
                let m = Approval {
                    host: s(a, "host").into(),
                    user: s(a, "user").into(),
                    service: s(a, "service").into(),
                    tty: s(a, "tty").into(),
                    rhost: s(a, "rhost").into(),
                    time,
                    nonce: s(a, "nonce").into(),
                };
                m.validate()?;
                json!({ "host": m.host, "user": m.user, "service": m.service, "tty": m.tty, "rhost": m.rhost, "time": m.time, "nonce": m.nonce })
            }
            "enroll" => {
                // The phone fills in `key` with its new key; check the rest.
                let m = Enrollment {
                    host: s(a, "host").into(),
                    user: s(a, "user").into(),
                    key: "0".repeat(64),
                    time,
                    nonce: s(a, "nonce").into(),
                };
                m.validate()?;
                json!({ "host": m.host, "user": m.user, "time": m.time, "nonce": m.nonce })
            }
            other => bail!("unknown kind {other}"),
        };
        body["kind"] = json!(kind);
        body["id"] = json!(id);
        body["timeout"] = json!(timeout);
        Ok(body)
    }

    async fn cancel_on_phone(dev: &DeviceHandle, id: &str) {
        let _ = dev
            .send(Packet::new(
                types::APPROVE_REQUEST,
                json!({ "kind": "cancel", "id": id }),
            ))
            .await;
    }

    /// Close the open request of `device` with `answer`.
    fn finish(&self, device: &str, id: Option<&str>, answer: Value) -> bool {
        let mut p = self.pending.lock().unwrap();
        let Some(entry) = p.get_mut(device) else {
            return false;
        };
        if id.is_some_and(|id| id != entry.id) {
            return false;
        }
        if let Some(tx) = entry.tx.take() {
            let _ = tx.send(answer);
        }
        // Keep the entry until `wait` collects it (or it expires).
        true
    }
}

#[async_trait]
impl Plugin for ApprovePlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "approve",
            name: "Fingerprint approval",
            description: "Approve sudo and polkit on this computer with the phone's fingerprint",
            permission: Permission::Approve,
            incoming: &[types::APPROVE],
            outgoing: &[types::APPROVE_REQUEST],
        }
    }

    async fn on_disconnected(&self, id: &str) {
        self.finish(id, None, json!({ "result": "disconnected" }));
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        let id = p.get_str("id").unwrap_or_default().to_owned();
        if !self.finish(dev.id(), Some(&id), clean_answer(&p.body)) {
            tracing::debug!("approval answer for no open request");
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
        match action {
            // {"kind": "approve"|"enroll", fields..., "timeout": s} -> {"id"}
            "request" => {
                let kind = args
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("approve");
                let timeout = args
                    .get("timeout")
                    .and_then(Value::as_u64)
                    .unwrap_or(25)
                    .clamp(MIN_TIMEOUT, MAX_TIMEOUT);
                let id = uuid::Uuid::new_v4().simple().to_string();
                let body = Self::request_body(kind, &id, &args, timeout)?;
                {
                    let mut p = self.pending.lock().unwrap();
                    if p.get(dev.id()).is_some_and(|e| e.deadline > Instant::now()) {
                        bail!("another approval is waiting on this phone");
                    }
                    let (tx, rx) = oneshot::channel();
                    p.insert(
                        dev.id().to_owned(),
                        Pending {
                            id: id.clone(),
                            deadline: Instant::now() + Duration::from_secs(timeout),
                            tx: Some(tx),
                            rx: Some(rx),
                        },
                    );
                }
                if let Err(e) = dev.send(Packet::new(types::APPROVE_REQUEST, body)).await {
                    self.pending.lock().unwrap().remove(dev.id());
                    return Err(e.context("the phone cannot take approvals"));
                }
                Ok(json!({ "id": id }))
            }
            // {"id"} -> the phone's answer, or {"result": "timeout"}
            "wait" => {
                let id = s(&args, "id").to_owned();
                let (rx, deadline) = {
                    let mut p = self.pending.lock().unwrap();
                    let e = p
                        .get_mut(dev.id())
                        .filter(|e| e.id == id)
                        .context("no such request")?;
                    (e.rx.take().context("already waiting")?, e.deadline)
                };
                let left = deadline.saturating_duration_since(Instant::now());
                let answer = match tokio::time::timeout(left, rx).await {
                    Ok(Ok(a)) => a,
                    _ => {
                        Self::cancel_on_phone(dev, &id).await;
                        json!({ "result": "timeout" })
                    }
                };
                self.pending.lock().unwrap().remove(dev.id());
                Ok(answer)
            }
            "cancel" => {
                let id = s(&args, "id").to_owned();
                if self.finish(dev.id(), Some(&id), json!({ "result": "cancelled" })) {
                    Self::cancel_on_phone(dev, &id).await;
                }
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
    fn requests_are_validated() {
        let ok = json!({ "host": "arch", "user": "u", "service": "sudo", "time": 5, "nonce": "a".repeat(64) });
        let b = ApprovePlugin::request_body("approve", "x", &ok, 25).unwrap();
        assert_eq!(
            (b["kind"].as_str(), b["timeout"].as_u64()),
            (Some("approve"), Some(25))
        );
        let mut bad = ok.clone();
        bad["tty"] = json!("a\nb");
        assert!(ApprovePlugin::request_body("approve", "x", &bad, 25).is_err());
        assert!(ApprovePlugin::request_body("root", "x", &ok, 25).is_err());
        assert!(ApprovePlugin::request_body("enroll", "x", &ok, 25).is_ok());
    }

    #[test]
    fn answers_are_cleaned() {
        let mut m = Map::new();
        m.insert("result".into(), json!("approved"));
        m.insert("signature".into(), json!("MEUC"));
        m.insert("extra".into(), json!("x"));
        m.insert("publicKey".into(), json!("A".repeat(MAX_B64 + 1)));
        assert_eq!(
            clean_answer(&m),
            json!({ "result": "approved", "signature": "MEUC" })
        );
        m.insert("result".into(), json!("root"));
        assert_eq!(clean_answer(&m)["result"], "error");
    }
}
