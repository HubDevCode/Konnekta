//! Plugin system.
//!
//! A plugin declares which packet types it accepts / emits and the
//! [`Permission`] guarding it. The core enforces, for every packet:
//! paired device -> plugin globally enabled -> plugin enabled for the device ->
//! permission granted. Plugins therefore never see unauthorised input.

use crate::core::Inner;
use crate::device::DeviceHandle;
use crate::event::Event;
use crate::permissions::Permission;
use anyhow::{Result, bail};
use async_trait::async_trait;
use konnekta_proto::Packet;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginOrigin {
    /// Implements a standard KDE Connect plugin (interoperable).
    KdeConnect,
    /// Konnekta-only extension (`konnekta.*` packets).
    Konnekta,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub origin: PluginOrigin,
    pub permission: Permission,
    pub incoming: &'static [&'static str],
    pub outgoing: &'static [&'static str],
}

#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn info(&self) -> PluginInfo;

    /// Called once when the core starts. Background tasks may keep the
    /// `Arc` and should exit once [`CoreHandle::is_alive`] returns false.
    async fn start(self: Arc<Self>, _core: CoreHandle) {}

    /// A paired device became reachable (or was just paired).
    async fn on_connected(&self, _dev: &DeviceHandle) {}

    /// The device disconnected or was unpaired: stop any session with it.
    async fn on_disconnected(&self, _device_id: &str) {}

    /// Handle an authorised packet.
    async fn handle_packet(&self, dev: &DeviceHandle, packet: Packet) -> Result<()>;

    /// Local action requested by the user interface (not by the network).
    async fn handle_action(
        &self,
        _dev: Option<&DeviceHandle>,
        action: &str,
        _args: Value,
    ) -> Result<Value> {
        bail!("unsupported action '{action}'")
    }
}

/// Weak handle to the core given to plugins.
#[derive(Clone)]
pub struct CoreHandle {
    pub(crate) inner: Weak<Inner>,
}

impl CoreHandle {
    pub(crate) fn upgrade(&self) -> Option<Arc<Inner>> {
        self.inner.upgrade()
    }

    /// False once the core has been dropped: background tasks must stop.
    pub fn is_alive(&self) -> bool {
        self.inner.strong_count() > 0
    }

    pub fn emit(&self, event: Event) {
        if let Some(i) = self.upgrade() {
            let _ = i.events.send(event);
        }
    }

    pub fn emit_plugin(&self, device_id: Option<&str>, plugin: &str, kind: &str, data: Value) {
        self.emit(Event::Plugin {
            device_id: device_id.map(str::to_owned),
            plugin: plugin.to_owned(),
            kind: kind.to_owned(),
            data,
        });
    }

    /// Connected **and paired** device.
    pub fn device(&self, id: &str) -> Option<DeviceHandle> {
        self.upgrade()?.paired_handle(id)
    }

    /// All connected and paired devices.
    pub fn devices(&self) -> Vec<DeviceHandle> {
        self.upgrade()
            .map(|i| i.paired_handles())
            .unwrap_or_default()
    }

    pub fn download_dir(&self) -> PathBuf {
        self.upgrade()
            .map(|i| i.config.read().unwrap().download_dir.clone())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Private per-plugin storage directory (created on demand).
    pub fn plugin_data_dir(&self, plugin_id: &str) -> Option<PathBuf> {
        let dir = self
            .upgrade()?
            .data_dir
            .as_ref()?
            .join("plugins")
            .join(plugin_id);
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    pub fn local_device_id(&self) -> String {
        self.upgrade()
            .map(|i| i.identity.device_id.clone())
            .unwrap_or_default()
    }

    /// Invoke another plugin's local action (used by the Deck and macros).
    pub async fn plugin_action(
        &self,
        device_id: Option<&str>,
        plugin: &str,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        let Some(inner) = self.upgrade() else {
            bail!("core stopped")
        };
        inner.plugin_action(device_id, plugin, action, args).await
    }

    pub fn is_plugin_enabled(&self, id: &str) -> bool {
        self.upgrade()
            .map(|i| !i.config.read().unwrap().disabled_plugins.contains(id))
            .unwrap_or(false)
    }
}
