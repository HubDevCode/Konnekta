use crate::core::Inner;
use crate::event::Event;
use crate::payload::{self, PayloadSource, Progress};
use crate::permissions::Permission;
use anyhow::{Context, Result, bail};
use konnekta_proto::{DeviceType, IdentityBody, Packet, types};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::{Arc, Weak};
use tokio::io::AsyncWrite;
use tokio::sync::mpsc;

/// Snapshot of a device for user interfaces.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub device_type: DeviceType,
    pub paired: bool,
    pub connected: bool,
    pub pair_state: String,
    pub address: Option<String>,
    pub fingerprint: Option<String>,
    pub permissions: BTreeSet<Permission>,
    pub disabled_plugins: BTreeSet<String>,
    pub incoming_capabilities: Vec<String>,
    pub outgoing_capabilities: Vec<String>,
}

/// Handle to a connected device, given to plugins.
#[derive(Clone)]
pub struct DeviceHandle {
    pub(crate) inner: Weak<Inner>,
    pub(crate) link_id: u64,
    pub(crate) tx: mpsc::Sender<Packet>,
    pub identity: Arc<IdentityBody>,
    pub addr: SocketAddr,
    pub(crate) peer_cert: Arc<Vec<u8>>,
}

impl std::fmt::Debug for DeviceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceHandle")
            .field("id", &self.identity.device_id)
            .field("name", &self.identity.device_name)
            .field("addr", &self.addr)
            .finish()
    }
}

impl DeviceHandle {
    pub fn id(&self) -> &str {
        &self.identity.device_id
    }

    pub fn name(&self) -> &str {
        &self.identity.device_name
    }

    fn core(&self) -> Result<Arc<Inner>> {
        self.inner.upgrade().context("core stopped")
    }

    /// Does the peer accept this packet type?
    pub fn supports(&self, packet_type: &str) -> bool {
        self.identity.supports_incoming(packet_type)
    }

    pub fn is_paired(&self) -> bool {
        self.inner
            .upgrade()
            .map(|i| i.is_paired(self.id()))
            .unwrap_or(false)
    }

    pub fn has_permission(&self, p: Permission) -> bool {
        self.inner
            .upgrade()
            .map(|i| i.has_permission(self.id(), p))
            .unwrap_or(false)
    }

    /// Send a packet. Fails if the device is not paired, does not support the
    /// packet type, or the permission guarding it was not granted locally.
    pub async fn send(&self, packet: Packet) -> Result<()> {
        let core = self.core()?;
        if packet.packet_type != types::PAIR {
            if !core.is_paired(self.id()) {
                bail!("device not paired");
            }
            if !self.supports(&packet.packet_type) {
                bail!("{} does not support {}", self.name(), packet.packet_type);
            }
            if let Some(p) = core.permission_for_outgoing(&packet.packet_type)
                && !core.has_permission(self.id(), p)
            {
                bail!("permission {:?} not granted for {}", p, self.name());
            }
        }
        self.tx.send(packet).await.context("link closed")
    }

    /// Like [`send`](Self::send) but silently ignores unsupported types.
    pub async fn send_if_supported(&self, packet: Packet) -> Result<()> {
        if self.supports(&packet.packet_type) {
            self.send(packet).await
        } else {
            Ok(())
        }
    }

    pub fn resume_supported(&self) -> bool {
        self.supports(payload::RESUME_CAPABILITY)
    }

    /// Send a packet with a payload (file, image...). The returned task
    /// resolves when the peer has downloaded it.
    pub async fn send_with_payload(
        &self,
        packet: Packet,
        source: PayloadSource,
        progress: Option<Progress>,
    ) -> Result<tokio::task::JoinHandle<Result<()>>> {
        let core = self.core()?;
        let size = source.size().await?;
        let bind = core.config.read().unwrap().bind_addr;
        let (port, task) = payload::serve(
            &core.identity,
            (*self.peer_cert).clone(),
            self.id().to_owned(),
            self.addr.ip(),
            bind,
            source,
            self.resume_supported(),
            progress,
        )
        .await?;
        self.send(packet.with_payload(size as i64, port)).await?;
        Ok(task)
    }

    /// Download the payload attached to a received packet.
    pub async fn fetch_payload<W: AsyncWrite + Unpin>(
        &self,
        packet: &Packet,
        out: &mut W,
        resume_from: Option<u64>,
        progress: Option<Progress>,
    ) -> Result<u64> {
        let core = self.core()?;
        let info = packet
            .payload_transfer_info
            .context("packet has no payload")?;
        let size = packet.payload_size.unwrap_or(-1);
        if size < 0 {
            bail!("streams of unknown size are not supported");
        }
        let resume = if self.resume_supported() {
            resume_from.or(Some(0))
        } else {
            None
        };
        payload::fetch(
            &core.identity,
            (*self.peer_cert).clone(),
            self.id().to_owned(),
            SocketAddr::new(self.addr.ip(), info.port),
            size as u64,
            resume,
            out,
            progress,
        )
        .await?;
        Ok(size as u64)
    }

    pub fn emit(&self, plugin: &str, kind: &str, data: Value) {
        if let Some(i) = self.inner.upgrade() {
            let _ = i.events.send(Event::Plugin {
                device_id: Some(self.id().to_owned()),
                plugin: plugin.to_owned(),
                kind: kind.to_owned(),
                data,
            });
        }
    }
}
