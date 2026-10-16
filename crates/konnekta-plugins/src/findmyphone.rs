use anyhow::Result;
use async_trait::async_trait;
use konnekta_core::proto::{Packet, types};
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
use serde_json::Value;

/// Makes the phone ring loudly.
pub struct FindMyPhonePlugin;

#[async_trait]
impl Plugin for FindMyPhonePlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "findmyphone",
            name: "Find my phone",
            description: "Make the phone ring",
            origin: PluginOrigin::KdeConnect,
            permission: Permission::FindMyPhone,
            incoming: &[],
            outgoing: &[types::FINDMYPHONE_REQUEST],
        }
    }

    async fn handle_packet(&self, _: &DeviceHandle, _: Packet) -> Result<()> {
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        _: Value,
    ) -> Result<Value> {
        match action {
            "ring" => {
                crate::need_dev(dev)?
                    .send(Packet::empty(types::FINDMYPHONE_REQUEST))
                    .await?;
                Ok(Value::Null)
            }
            _ => anyhow::bail!("unknown action {action}"),
        }
    }
}
