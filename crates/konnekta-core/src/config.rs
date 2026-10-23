use konnekta_proto::{DeviceType, Protocol};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

/// Anti-abuse limits for devices that are not paired.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    /// Links of unpaired devices in total; a new one closes the oldest.
    pub unpaired_links: usize,
    pub unpaired_links_per_addr: usize,
    /// Unpaired link closed after this time without a pair request.
    pub unpaired_idle_secs: u64,
    /// Connections in handshake (before the link exists).
    pub pending_connections: usize,
    pub pending_connections_per_addr: usize,
    /// Open incoming pair requests.
    pub open_pair_requests: usize,
    pub open_pair_requests_per_addr: usize,
    /// After a request ends without pairing, ignore the device this long.
    pub pair_cooldown_secs: u64,
    /// Devices from discovery remembered while not paired.
    pub discovered_devices: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            unpaired_links: 8,
            unpaired_links_per_addr: 2,
            unpaired_idle_secs: 120,
            pending_connections: 32,
            pending_connections_per_addr: 4,
            open_pair_requests: 4,
            open_pair_requests_per_addr: 2,
            pair_cooldown_secs: 30,
            discovered_devices: 64,
        }
    }
}

/// Optional interop adapter for another ecosystem (KDE Connect, Flux). Off
/// by default; ports 0 = ephemeral (tests).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Adapter {
    pub enabled: bool,
    pub udp_port: u16,
    pub tcp_port: u16,
}

impl Adapter {
    pub fn off(p: Protocol) -> Self {
        Self {
            enabled: false,
            udp_port: p.udp_port(),
            tcp_port: *p.tcp_ports().start(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device_name: String,
    pub device_type: DeviceType,
    /// Where received files go.
    pub download_dir: PathBuf,
    /// Announce ourselves (UDP broadcast and mDNS). Disable on untrusted
    /// networks: paired devices are still dialed directly.
    pub announce: bool,
    /// Advertise and browse `_konnekta._tcp` with mDNS.
    pub mdns: bool,
    /// Listen for UDP announcements of other devices.
    pub listen_udp: bool,
    /// Connect to Android phones attached by USB cable, through adb.
    pub usb: bool,
    pub bind_addr: IpAddr,
    /// Native protocol ports (0 = ephemeral, used by tests).
    pub udp_port: u16,
    pub tcp_port: u16,
    /// Lets KDE Connect / GSConnect apps connect (off by default).
    pub kde_connect: Adapter,
    /// Lets Flux apps connect (off by default).
    pub flux: Adapter,
    /// Broadcast / unicast targets for announcements (port added per protocol).
    pub broadcast_targets: Vec<IpAddr>,
    /// Re-announce interval in seconds (0 = only at start / on demand).
    pub announce_interval_secs: u64,
    /// How often paired devices that are offline are dialed.
    pub reconnect_interval_secs: u64,
    /// Globally disabled plugins (by id).
    pub disabled_plugins: BTreeSet<String>,
    /// Keep the permissions of each pair the same on both devices.
    pub sync_permissions: bool,
    pub limits: Limits,
}

impl Default for Config {
    fn default() -> Self {
        let download_dir = directories::UserDirs::new()
            .and_then(|d| d.download_dir().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Konnekta");
        Self {
            device_name: default_device_name(),
            device_type: DeviceType::Desktop,
            download_dir,
            announce: true,
            mdns: true,
            listen_udp: true,
            usb: true,
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            udp_port: Protocol::Konnekta.udp_port(),
            tcp_port: *Protocol::Konnekta.tcp_ports().start(),
            kde_connect: Adapter::off(Protocol::KdeConnect),
            flux: Adapter::off(Protocol::Flux),
            broadcast_targets: vec![IpAddr::V4(Ipv4Addr::BROADCAST)],
            announce_interval_secs: 60,
            reconnect_interval_secs: 10,
            disabled_plugins: BTreeSet::new(),
            sync_permissions: true,
            limits: Limits::default(),
        }
    }
}

impl Config {
    /// Settings of an interop adapter (`None` for the native protocol).
    pub fn adapter(&self, p: Protocol) -> Option<&Adapter> {
        match p {
            Protocol::Konnekta => None,
            Protocol::KdeConnect => Some(&self.kde_connect),
            Protocol::Flux => Some(&self.flux),
        }
    }

    pub fn adapter_mut(&mut self, p: Protocol) -> Option<&mut Adapter> {
        match p {
            Protocol::Konnekta => None,
            Protocol::KdeConnect => Some(&mut self.kde_connect),
            Protocol::Flux => Some(&mut self.flux),
        }
    }

    /// The native protocol is always on; adapters only when enabled.
    pub fn speaks(&self, p: Protocol) -> bool {
        self.adapter(p).is_none_or(|a| a.enabled)
    }
}

fn default_device_name() -> String {
    let name = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "Konnekta".into());
    name.chars().filter(|c| !c.is_control()).take(32).collect()
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(raw) => Ok(serde_json::from_slice(&raw)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        konnekta_crypto::write_private_file(path, &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    /// Default data directory (`~/.config/konnekta`, `%APPDATA%\konnekta`, ...).
    pub fn default_data_dir() -> PathBuf {
        directories::ProjectDirs::from("org", "Konnekta", "konnekta")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".konnekta"))
    }
}
