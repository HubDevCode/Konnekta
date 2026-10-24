//! Links over a USB cable, through adb (Android with USB debugging on).
//!
//! The computer runs `adb`: it forwards a local port to the phone's
//! listener and dials it like any other address, so TLS, pinning and pairing
//! are unchanged. Side connections (payloads, stream channels) are mapped the
//! same way: the computer forwards the ports the phone listens on, and
//! reverses its own so the phone reaches them on its 127.0.0.1. The phone
//! needs nothing but its usual listener.

use anyhow::{Context, Result, bail};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const ADB_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// Same machine as far as a connection check goes. Through adb every
/// connection comes from the loopback interface, v4 or v6.
pub(crate) fn same_host(a: IpAddr, b: IpAddr) -> bool {
    a == b || (a.is_loopback() && b.is_loopback())
}

/// The adb program: `PATH`, then the usual Android SDK folders.
pub fn find_adb() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(d) = std::env::var_os(var) {
            dirs.push(PathBuf::from(d).join("platform-tools"));
        }
    }
    if let Some(home) = directories::BaseDirs::new() {
        let h = home.home_dir();
        dirs.push(h.join("Android/Sdk/platform-tools"));
        dirs.push(h.join("Library/Android/sdk/platform-tools"));
        dirs.push(home.data_local_dir().join("Android/Sdk/platform-tools"));
    }
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

async fn adb(args: &[&str]) -> Result<String> {
    let path = find_adb().context("adb not found")?;
    let mut cmd = tokio::process::Command::new(path);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(ADB_TIMEOUT, cmd.output())
        .await
        .context("adb timeout")??;
    if !out.status.success() {
        bail!(
            "adb {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Serials of the attached devices that authorized this computer.
pub async fn devices() -> Vec<String> {
    let Ok(out) = adb(&["devices"]).await else {
        return Vec::new();
    };
    out.lines()
        .filter_map(|l| {
            let (serial, state) = l.split_once('\t')?;
            (state.trim() == "device").then(|| serial.trim().to_owned())
        })
        .collect()
}

/// `adb forward|reverse tcp:0 ...` prints the port it picked.
fn reported_port(out: &str) -> Result<u16> {
    out.trim().parse().context("adb did not report the port")
}

/// A port mapping that is removed when dropped: an adb one, or a stream
/// listener of a Bluetooth session.
pub enum Mapping {
    Adb(AdbMapping),
    Bluetooth(crate::bluetooth::Guard),
}

pub struct AdbMapping {
    serial: Arc<str>,
    kind: &'static str,
    port: u16,
}

impl Drop for AdbMapping {
    fn drop(&mut self) {
        let (serial, kind, port) = (self.serial.clone(), self.kind, self.port);
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                let spec = format!("tcp:{port}");
                let _ = adb(&["-s", &serial, kind, "--remove", &spec]).await;
            });
        }
    }
}

/// Forward a free local port to `remote` on the phone: dial the returned
/// address to reach it.
pub async fn forward(serial: &Arc<str>, remote: u16) -> Result<(SocketAddr, Mapping)> {
    let out = adb(&["-s", serial, "forward", "tcp:0", &format!("tcp:{remote}")]).await?;
    let port = reported_port(&out)?;
    Ok((
        SocketAddr::new(LOOPBACK, port),
        Mapping::Adb(AdbMapping {
            serial: serial.clone(),
            kind: "forward",
            port,
        }),
    ))
}

/// Make the local port `local` reachable from the phone: it connects to the
/// returned port on its own 127.0.0.1.
pub async fn reverse(serial: &Arc<str>, local: u16) -> Result<(u16, Mapping)> {
    let out = adb(&["-s", serial, "reverse", "tcp:0", &format!("tcp:{local}")]).await?;
    let port = reported_port(&out)?;
    Ok((
        port,
        Mapping::Adb(AdbMapping {
            serial: serial.clone(),
            kind: "reverse",
            port,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_is_one_host() {
        assert!(same_host(LOOPBACK, "::1".parse().unwrap()));
        assert!(!same_host(LOOPBACK, "192.168.1.2".parse().unwrap()));
        assert_eq!(reported_port("41234\n").unwrap(), 41234);
        assert!(reported_port("").is_err());
    }
}
