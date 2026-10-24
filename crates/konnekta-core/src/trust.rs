//! Persistent store of paired (trusted) devices and their pinned certificates.

use crate::permissions::Permission;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use konnekta_proto::{DeviceType, Protocol};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrustedDevice {
    pub device_id: String,
    pub name: String,
    #[serde(default)]
    pub device_type: DeviceType,
    /// Pinned certificate (DER, base64).
    pub certificate: String,
    pub permissions: BTreeSet<Permission>,
    /// Wall clock ms of the last change of `permissions` (here or synced
    /// from the device); 0 = never changed since pairing.
    #[serde(default)]
    pub permissions_changed_at: i64,
    #[serde(default)]
    pub disabled_plugins: BTreeSet<String>,
    pub paired_at: i64,
    #[serde(default)]
    pub last_address: Option<IpAddr>,
    /// TCP port of the device's listener (native protocol).
    #[serde(default)]
    pub last_port: Option<u16>,
    /// Protocol the device was paired with.
    #[serde(default = "native")]
    pub protocol: Protocol,
    /// Extra addresses dialed when the device is not on the local network,
    /// e.g. its Tailscale name or IP (`host` or `host:port`).
    #[serde(default)]
    pub extra_addresses: Vec<String>,
    /// The way the pair chose to connect (synced on both devices); none =
    /// the fastest available.
    #[serde(default)]
    pub preferred_transport: Option<crate::Transport>,
    /// Wall clock ms of the last change of `preferred_transport`.
    #[serde(default)]
    pub transport_changed_at: i64,
}

fn native() -> Protocol {
    Protocol::Konnekta
}

impl TrustedDevice {
    pub fn cert_der(&self) -> Vec<u8> {
        B64.decode(&self.certificate).unwrap_or_default()
    }

    pub fn encode_cert(der: &[u8]) -> String {
        B64.encode(der)
    }
}

pub struct TrustStore {
    path: Option<PathBuf>,
    devices: BTreeMap<String, TrustedDevice>,
}

impl TrustStore {
    pub fn in_memory() -> Self {
        Self {
            path: None,
            devices: BTreeMap::new(),
        }
    }

    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        let devices: BTreeMap<String, TrustedDevice> = match std::fs::read(&path) {
            Ok(raw) => match serde_json::from_slice(&raw) {
                Ok(d) => d,
                Err(e) => {
                    // Never start with a half-understood trust store: keep the
                    // file for inspection and start without paired devices.
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or_default();
                    let moved = path.with_extension(format!("json.broken-{ts}"));
                    tracing::error!(
                        "trusted devices file is corrupt ({e}); moved to {}",
                        moved.display()
                    );
                    std::fs::rename(&path, &moved)?;
                    BTreeMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        // A pinned certificate that does not parse counts as not paired.
        let devices = devices
            .into_iter()
            .filter(|(id, d)| {
                let ok = konnekta_crypto::cert_common_name(&d.cert_der()).is_ok_and(|cn| cn == *id);
                if !ok {
                    tracing::error!(device = %id, "ignoring trusted device with an unreadable certificate");
                }
                ok
            })
            .collect();
        Ok(Self {
            path: Some(path),
            devices,
        })
    }

    fn persist(&self) {
        if let Some(path) = &self.path {
            let data = serde_json::to_vec_pretty(&self.devices).expect("serializable");
            if let Err(e) = konnekta_crypto::write_private_file(path, &data) {
                tracing::error!("failed to save trusted devices: {e}");
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&TrustedDevice> {
        self.devices.get(id)
    }

    pub fn is_trusted(&self, id: &str) -> bool {
        self.devices.contains_key(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &TrustedDevice> {
        self.devices.values()
    }

    pub fn insert(&mut self, dev: TrustedDevice) {
        self.devices.insert(dev.device_id.clone(), dev);
        self.persist();
    }

    pub fn remove(&mut self, id: &str) -> Option<TrustedDevice> {
        let r = self.devices.remove(id);
        self.persist();
        r
    }

    pub fn update<F: FnOnce(&mut TrustedDevice)>(&mut self, id: &str, f: F) -> bool {
        match self.devices.get_mut(id) {
            Some(d) => {
                f(d);
                self.persist();
                true
            }
            None => false,
        }
    }
}
