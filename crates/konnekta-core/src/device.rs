use crate::core::Inner;
use crate::event::Event;
use crate::payload::{self, PayloadSource, Progress};
use crate::permissions::Permission;
use crate::usb;
use anyhow::{Context, Result, bail};
use konnekta_proto::{DeviceType, IdentityBody, Packet, Protocol, types};
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
    /// The current link runs over a USB cable.
    pub usb: bool,
    pub fingerprint: Option<String>,
    pub permissions: BTreeSet<Permission>,
    pub disabled_plugins: BTreeSet<String>,
    pub incoming_capabilities: Vec<String>,
    pub outgoing_capabilities: Vec<String>,
    /// Protocol of the current link (or of the pairing when offline).
    pub protocol: Protocol,
    /// Key of the open pairing, to show while it waits for the user.
    pub pair_key: Option<String>,
    /// Extra addresses dialed when the device is not on the local network.
    pub extra_addresses: Vec<String>,
    /// Program and version reported by the device.
    pub app: Option<String>,
}

/// Handle to a connected device, given to plugins.
#[derive(Clone)]
pub struct DeviceHandle {
    pub(crate) inner: Weak<Inner>,
    pub(crate) link_id: u64,
    pub(crate) tx: mpsc::Sender<Packet>,
    pub identity: Arc<IdentityBody>,
    pub addr: SocketAddr,
    pub protocol: Protocol,
    pub(crate) peer_cert: Arc<Vec<u8>>,
    /// True if we dialed the device.
    pub(crate) outgoing: bool,
    /// The link runs over a USB cable (adb).
    pub usb: bool,
    /// Serial of the phone when we drive the cable with adb: side
    /// connections are then mapped through it.
    pub(crate) adb: Option<Arc<str>>,
}

impl std::fmt::Debug for DeviceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceHandle")
            .field("id", &self.identity.device_id)
            .field("name", &self.identity.device_name)
            .field("addr", &self.addr)
            .field("usb", &self.usb)
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
            if crate::link::to_wire(packet.clone(), self.protocol).is_none() {
                bail!(
                    "{} is not available over {}",
                    packet.packet_type,
                    self.protocol.label()
                );
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

    /// Address to dial for a port the device listens on. Over USB it is an
    /// adb forward, which must live as long as the connection.
    pub(crate) async fn peer_port(&self, port: u16) -> Result<(SocketAddr, Option<usb::Mapping>)> {
        match &self.adb {
            Some(serial) => {
                let (addr, m) = usb::forward(serial, port).await?;
                Ok((addr, Some(m)))
            }
            None => Ok((SocketAddr::new(self.addr.ip(), port), None)),
        }
    }

    /// Port to announce for a local listener. Over USB it is an adb reverse,
    /// which must live as long as the connection.
    pub(crate) async fn announced_port(&self, local: u16) -> Result<(u16, Option<usb::Mapping>)> {
        match &self.adb {
            Some(serial) => {
                let (port, m) = usb::reverse(serial, local).await?;
                Ok((port, Some(m)))
            }
            None => Ok((local, None)),
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
            self.protocol.payload_ports(),
            source,
            self.resume_supported(),
            progress,
        )
        .await?;
        let (port, mapping) = self.announced_port(port).await?;
        self.send(packet.with_payload(size as i64, port)).await?;
        Ok(match mapping {
            Some(m) => tokio::spawn(async move {
                let r = task.await;
                drop(m);
                r?
            }),
            None => task,
        })
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
        let (addr, _mapping) = self.peer_port(info.port).await?;
        payload::fetch(
            &core.identity,
            (*self.peer_cert).clone(),
            self.id().to_owned(),
            addr,
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
