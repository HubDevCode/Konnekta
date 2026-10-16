use konnekta_proto::DeviceType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device_name: String,
    pub device_type: DeviceType,
    /// Where received files go.
    pub download_dir: PathBuf,
    /// Announce ourselves with UDP broadcasts. Disable on untrusted networks
    /// to avoid exposing the device name (manual connect still works).
    pub announce: bool,
    /// Listen for UDP announcements of other devices.
    pub listen_udp: bool,
    pub bind_addr: IpAddr,
    pub udp_port: u16,
    pub tcp_port: u16,
    /// Extra unicast/broadcast targets for announcements.
    pub broadcast_targets: Vec<SocketAddr>,
    /// Re-announce interval in seconds (0 = only at start / on demand).
    pub announce_interval_secs: u64,
    /// Globally disabled plugins (by id).
    pub disabled_plugins: BTreeSet<String>,
    /// Maximum simultaneous connections from devices that are not paired.
    pub max_unpaired_links: usize,
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
            listen_udp: true,
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            udp_port: konnekta_proto::DEFAULT_PORT,
            tcp_port: konnekta_proto::DEFAULT_PORT,
            broadcast_targets: vec![SocketAddr::new(
                IpAddr::V4(Ipv4Addr::BROADCAST),
                konnekta_proto::DEFAULT_PORT,
            )],
            announce_interval_secs: 60,
            disabled_plugins: BTreeSet::new(),
            max_unpaired_links: 8,
        }
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
