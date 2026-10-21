use anyhow::Result;
use async_trait::async_trait;
use konnekta_core::proto::{Packet, types};
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo};
use serde_json::Value;

/// Makes the phone ring loudly.
pub struct FindDevicePlugin;

#[async_trait]
impl Plugin for FindDevicePlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "finddevice",
            name: "Find my phone",
            description: "Make the phone ring",
            permission: Permission::FindMyPhone,
            incoming: &[],
            outgoing: &[types::FIND_DEVICE],
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
                    .send(Packet::empty(types::FIND_DEVICE))
                    .await?;
                Ok(Value::Null)
            }
            _ => anyhow::bail!("unknown action {action}"),
        }
    }
}
