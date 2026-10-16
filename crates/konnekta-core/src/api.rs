//! JSON API over the core, shared by every front-end: the daemon's local IPC,
//! the mobile FFI bridge and the desktop app. Method names are camelCase.

use crate::{Konnekta, Permission};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};

fn s<'a>(p: &'a Value, k: &str) -> Result<&'a str> {
    p.get(k).and_then(Value::as_str).with_context(|| format!("missing '{k}'"))
}

fn b(p: &Value, k: &str) -> Result<bool> {
    p.get(k).and_then(Value::as_bool).with_context(|| format!("missing '{k}'"))
}

pub fn parse_address(addr: &str) -> Result<SocketAddr> {
    if let Ok(sa) = addr.parse::<SocketAddr>() {
        return Ok(sa);
    }
    let ip: IpAddr = addr.trim().parse().context("invalid IP address")?;
    Ok(SocketAddr::new(ip, konnekta_proto::DEFAULT_PORT))
}

pub async fn call(k: &Konnekta, method: &str, p: Value) -> Result<Value> {
    Ok(match method {
        "info" => {
            let c = k.config();
            json!({
                "deviceId": k.device_id(), "deviceName": c.device_name, "deviceType": c.device_type,
                "fingerprint": k.fingerprint(), "tcpPort": k.tcp_port(), "udpPort": k.udp_port(),
                "protocolVersion": konnekta_proto::PROTOCOL_VERSION, "version": env!("CARGO_PKG_VERSION"),
            })
        }
        "devices" => serde_json::to_value(k.devices())?,
        "device" => serde_json::to_value(k.device(s(&p, "deviceId")?))?,
        "connect" => {
            k.connect_to(parse_address(s(&p, "address")?)?).await?;
            Value::Null
        }
        "refresh" => {
            k.refresh().await;
            Value::Null
        }
        "pair" => json!({ "verificationKey": k.request_pair(s(&p, "deviceId")?).await? }),
        "acceptPair" => {
            k.accept_pair(s(&p, "deviceId")?).await?;
            Value::Null
        }
        "rejectPair" => {
            k.reject_pair(s(&p, "deviceId")?).await?;
            Value::Null
        }
        "unpair" => {
            k.unpair(s(&p, "deviceId")?).await?;
            Value::Null
        }
        "permissions" => Value::Array(
            Permission::ALL
                .iter()
                .map(|x| json!({ "id": x, "label": x.label(), "sensitive": x.is_sensitive() }))
                .collect(),
        ),
        "setPermission" => {
            let perm: Permission = serde_json::from_value(p.get("permission").cloned().unwrap_or_default())
                .context("unknown permission")?;
            k.set_permission(s(&p, "deviceId")?, perm, b(&p, "granted")?)?;
            Value::Null
        }
        "setDevicePlugin" => {
            k.set_device_plugin_enabled(s(&p, "deviceId")?, s(&p, "plugin")?, b(&p, "enabled")?)?;
            Value::Null
        }
        "plugins" => Value::Array(
            k.plugins()
                .into_iter()
                .map(|(i, enabled)| {
                    let mut v = serde_json::to_value(i).unwrap_or_default();
                    v["enabled"] = json!(enabled);
                    v
                })
                .collect(),
        ),
        "setPluginEnabled" => {
            k.set_plugin_enabled(s(&p, "plugin")?, b(&p, "enabled")?)?;
            Value::Null
        }
        "pluginAction" => {
            let dev = p.get("deviceId").and_then(Value::as_str);
            k.plugin_action(dev, s(&p, "plugin")?, s(&p, "action")?, p.get("args").cloned().unwrap_or(Value::Null))
                .await?
        }
        "getConfig" => serde_json::to_value(k.config())?,
        "setConfig" => {
            let name = p.get("deviceName").and_then(Value::as_str).map(str::to_owned);
            if let Some(n) = &name {
                if n.trim().is_empty() || n.chars().count() > 32 || n.chars().any(char::is_control) {
                    bail!("invalid device name");
                }
            }
            k.update_config(|c| {
                if let Some(n) = name {
                    c.device_name = n;
                }
                if let Some(d) = p.get("downloadDir").and_then(Value::as_str) {
                    c.download_dir = d.into();
                }
                if let Some(a) = p.get("announce").and_then(Value::as_bool) {
                    c.announce = a;
                }
            })?;
            Value::Null
        }
        _ => bail!("unknown method '{method}'"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse_address("192.168.1.5").unwrap().port(), 1716);
        assert_eq!(parse_address("10.0.0.2:1800").unwrap().port(), 1800);
        assert!(parse_address("evil.example").is_err());
    }
}
