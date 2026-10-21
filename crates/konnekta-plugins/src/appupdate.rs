//! Push a new build of the Android app from the computer.
//!
//! The file must be an `.apk` (a ZIP) of at most 300 MiB; it travels with
//! its SHA-256 and the phone verifies it before handing it to the system
//! installer, which always asks the user to confirm. Only the Konnekta
//! Android app advertises `konnekta.app.update`.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use konnekta_core::payload::PayloadSource;
use konnekta_core::proto::{Packet, types};
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_APK: u64 = 300 * 1024 * 1024;

/// Check the file and return (size, sha256 hex).
pub fn check_apk(path: &Path) -> Result<(u64, String)> {
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("apk"))
    {
        bail!("not an .apk file");
    }
    let meta = std::fs::metadata(path).context("file not found")?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_APK {
        bail!("invalid size");
    }
    let mut f = std::fs::File::open(path)?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)?;
    if magic != *b"PK\x03\x04" {
        bail!("not an Android package");
    }
    let mut h = Sha256::new();
    h.update(magic);
    std::io::copy(&mut f, &mut h)?;
    Ok((meta.len(), hex::encode(h.finalize())))
}

pub struct AppUpdatePlugin;

#[async_trait]
impl Plugin for AppUpdatePlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "appupdate",
            name: "App update",
            description: "Send a new version of the Android app from this computer",
            permission: Permission::Files,
            incoming: &[],
            outgoing: &[types::APP_UPDATE],
        }
    }

    async fn handle_packet(&self, _: &DeviceHandle, _: Packet) -> Result<()> {
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        let dev = crate::need_dev(dev)?;
        match action {
            "push" => {
                if !dev.supports(types::APP_UPDATE) {
                    bail!("this device does not accept app updates");
                }
                let path: PathBuf = args
                    .get("path")
                    .and_then(Value::as_str)
                    .context("missing path")?
                    .into();
                let p = path.clone();
                let (size, sha256) = tokio::task::spawn_blocking(move || check_apk(&p)).await??;
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let packet = Packet::new(
                    types::APP_UPDATE,
                    json!({ "filename": name, "sha256": sha256 }),
                );
                dev.send_with_payload(packet, PayloadSource::File(path), None)
                    .await?;
                dev.emit(
                    "appupdate",
                    "sent",
                    json!({ "filename": name, "size": size }),
                );
                Ok(json!({ "sha256": sha256 }))
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_apk_zips_are_accepted() {
        let d = tempfile::tempdir().unwrap();
        let ok = d.path().join("konnekta.apk");
        std::fs::write(&ok, b"PK\x03\x04rest").unwrap();
        let (size, sha) = check_apk(&ok).unwrap();
        assert_eq!(size, 8);
        assert_eq!(sha, hex::encode(Sha256::digest(b"PK\x03\x04rest")));
        let fake = d.path().join("evil.apk");
        std::fs::write(&fake, b"#!/bin/sh").unwrap();
        assert!(check_apk(&fake).is_err());
        let wrong_ext = d.path().join("konnekta.zip");
        std::fs::write(&wrong_ext, b"PK\x03\x04").unwrap();
        assert!(check_apk(&wrong_ext).is_err());
    }
}
