use anyhow::Result;
use async_trait::async_trait;
use konnekta_core::proto::{Packet, bodies::PingBody, types};
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
use konnekta_platform::{DesktopNotification, Platform};
use serde_json::{Value, json};

pub struct PingPlugin {
    platform: Platform,
}

impl PingPlugin {
    pub fn new(platform: Platform) -> Self {
        Self { platform }
    }
}

#[async_trait]
impl Plugin for PingPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "ping",
            name: "Ping",
            description: "Send and receive pings",
            origin: PluginOrigin::KdeConnect,
            permission: Permission::Ping,
            incoming: &[types::PING],
            outgoing: &[types::PING],
        }
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        let body: PingBody = p.body_as().unwrap_or_default();
        let msg = body.message.unwrap_or_else(|| "Ping!".into());
        let msg: String = msg.chars().take(500).collect();
        self.platform.notifier.show(
            DesktopNotification {
                id: format!("ping-{}", dev.id()),
                app_name: dev.name().into(),
                title: "Ping".into(),
                body: msg.clone(),
                ..Default::default()
            },
            None,
        )?;
        dev.emit("ping", "received", json!({ "message": msg }));
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        match action {
            "send" => {
                let message = args
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                crate::need_dev(dev)?
                    .send(Packet::new(types::PING, PingBody { message }))
                    .await?;
                Ok(Value::Null)
            }
            _ => anyhow::bail!("unknown action {action}"),
        }
    }
}
