use anyhow::Result;
use async_trait::async_trait;
use konnekta_core::proto::{Packet, bodies::BatteryBody, types};
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

/// Shows the phone battery level.
#[derive(Default)]
pub struct BatteryPlugin {
    levels: Mutex<HashMap<String, BatteryBody>>,
}

#[async_trait]
impl Plugin for BatteryPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "battery",
            name: "Battery",
            description: "Show the battery level of connected devices",
            origin: PluginOrigin::KdeConnect,
            permission: Permission::Battery,
            incoming: &[types::BATTERY],
            outgoing: &[],
        }
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        let mut b: BatteryBody = p.body_as()?;
        b.current_charge = b.current_charge.clamp(-1, 100);
        self.levels
            .lock()
            .unwrap()
            .insert(dev.id().to_owned(), b.clone());
        dev.emit("battery", "status", serde_json::to_value(&b)?);
        Ok(())
    }

    async fn on_disconnected(&self, id: &str) {
        self.levels.lock().unwrap().remove(id);
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        _: Value,
    ) -> Result<Value> {
        match action {
            "get" => {
                let id = crate::need_dev(dev)?.id();
                Ok(serde_json::to_value(self.levels.lock().unwrap().get(id))?)
            }
            _ => anyhow::bail!("unknown action {action}"),
        }
    }
}
