//! `konnekta waybar`: one JSON line per change for a Waybar custom module.
//!
//! ```jsonc
//! "custom/konnekta": {
//!   "exec": "konnekta waybar",
//!   "return-type": "json",
//!   "format": "{}",
//!   "on-click": "konnekta-desktop"
//! }
//! ```
//!
//! Classes: `connected`, `low` (battery ≤ 15% and not charging),
//! `disconnected` (paired devices, none connected), `none`, `off` (daemon
//! not running). Never exits on its own: it waits for the daemon to come back.

use anyhow::Result;
use konnekta_core::ipc;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

const RETRY: Duration = Duration::from_secs(10);
const LOW: i64 = 15;

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Waybar line from the `devices` list and the battery of connected devices.
pub fn line(devices: &Value, batteries: &HashMap<String, Value>) -> Value {
    let paired: Vec<&Value> = devices
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["paired"].as_bool() == Some(true))
        .collect();
    if paired.is_empty() {
        return json!({ "text": "", "class": "none", "tooltip": "Konnekta: nessun dispositivo associato" });
    }
    let mut tooltip = Vec::new();
    let mut text = None;
    let mut class = "disconnected";
    for d in &paired {
        let id = d["id"].as_str().unwrap_or_default();
        let name = escape(d["name"].as_str().unwrap_or("?"));
        if d["connected"].as_bool() != Some(true) {
            tooltip.push(format!("{name} — non connesso"));
            continue;
        }
        let bat = batteries.get(id).filter(|b| !b.is_null());
        let level = bat
            .and_then(|b| b["currentCharge"].as_i64())
            .filter(|l| *l >= 0);
        let charging = bat.and_then(|b| b["isCharging"].as_bool()).unwrap_or(false);
        let detail = match level {
            Some(l) if charging => format!("{l}% in carica"),
            Some(l) => format!("{l}%"),
            None => "connesso".into(),
        };
        tooltip.push(format!("{name} — {detail}"));
        // The first connected device is shown in the bar.
        if text.is_none() {
            text = Some(match level {
                Some(l) => format!("{}{l}%", if charging { "󰂄 " } else { "󰄜 " }),
                None => "󰄜".into(),
            });
            class = if level.is_some_and(|l| l <= LOW) && !charging {
                "low"
            } else {
                "connected"
            };
        }
    }
    json!({ "text": text.unwrap_or_else(|| "󰥐".into()), "class": class, "tooltip": tooltip.join("\n") })
}

fn emit(v: &Value) {
    println!("{v}");
}

async fn render(c: &mut ipc::Client) -> Result<Value> {
    let devices = c.call("devices", Value::Null).await?;
    let mut batteries = HashMap::new();
    for d in devices.as_array().into_iter().flatten() {
        if d["paired"].as_bool() == Some(true) && d["connected"].as_bool() == Some(true) {
            let id = d["id"].as_str().unwrap_or_default().to_owned();
            let b = c
                .call(
                    "pluginAction",
                    json!({ "deviceId": id, "plugin": "battery", "action": "get", "args": null }),
                )
                .await
                .unwrap_or(Value::Null);
            batteries.insert(id, b);
        }
    }
    Ok(line(&devices, &batteries))
}

/// Events that may change the module.
fn relevant(e: &Value) -> bool {
    match e["event"].as_str() {
        Some("plugin") => e["plugin"] == "battery",
        Some(k) => k.starts_with("device") || k == "paired" || k == "unpaired",
        None => false,
    }
}

pub async fn run(dir: PathBuf) -> Result<()> {
    let endpoint = ipc::endpoint(&dir);
    let mut last = Value::Null;
    let mut show = |v: Value| {
        if v != last {
            emit(&v);
            last = v;
        }
    };
    loop {
        let (Ok(mut c), Ok(events)) = (
            ipc::Client::connect(&endpoint).await,
            ipc::Client::connect(&endpoint).await,
        ) else {
            show(json!({ "text": "", "class": "off", "tooltip": "Konnekta non è in esecuzione" }));
            tokio::time::sleep(RETRY).await;
            continue;
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let sub = tokio::spawn(events.subscribe(move |e| {
            if relevant(&e) {
                let _ = tx.send(());
            }
        }));
        loop {
            match render(&mut c).await {
                Ok(v) => show(v),
                Err(_) => break,
            }
            // Wait for a change, then let bursts settle.
            if rx.recv().await.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            while rx.try_recv().is_ok() {}
        }
        sub.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states() {
        let none = line(&json!([]), &HashMap::new());
        assert_eq!(none["class"], "none");
        let devs = json!([
            { "id": "a", "name": "Pixel <8>", "paired": true, "connected": true },
            { "id": "b", "name": "iPad", "paired": true, "connected": false },
            { "id": "c", "name": "Stranger", "paired": false, "connected": true }
        ]);
        let mut bat = HashMap::new();
        bat.insert(
            "a".to_string(),
            json!({ "currentCharge": 12, "isCharging": false }),
        );
        let v = line(&devs, &bat);
        assert_eq!(v["class"], "low");
        assert_eq!(v["text"], "󰄜 12%");
        assert_eq!(v["tooltip"], "Pixel &lt;8&gt; — 12%\niPad — non connesso");
        let off = line(
            &json!([{ "id": "b", "name": "iPad", "paired": true, "connected": false }]),
            &HashMap::new(),
        );
        assert_eq!(off["class"], "disconnected");
    }
}
