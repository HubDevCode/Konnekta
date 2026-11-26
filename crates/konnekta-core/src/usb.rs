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
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ADB_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// Same machine as far as a connection check goes. Through adb every
/// connection comes from the loopback interface, v4 or v6.
pub(crate) fn same_host(a: IpAddr, b: IpAddr) -> bool {
    a == b || (a.is_loopback() && b.is_loopback())
}

/// The adb program: `PATH`, then the usual Android SDK folders. Looked up
/// again at most once a minute (every adb call asks).
pub fn find_adb() -> Option<PathBuf> {
    static FOUND: Mutex<Option<(std::time::Instant, Option<PathBuf>)>> = Mutex::new(None);
    if let Some((at, p)) = FOUND.lock().unwrap().as_ref()
        && at.elapsed() < Duration::from_secs(60)
    {
        return p.clone();
    }
    let p = look_for_adb();
    *FOUND.lock().unwrap() = Some((std::time::Instant::now(), p.clone()));
    p
}

fn look_for_adb() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
    let mut dirs: Vec<PathBuf> = konnekta_platform::host::path()
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
    let mut cmd = konnekta_platform::host::command_async(path);
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
///
/// From `adb track-devices`, which stays connected to the adb server and
/// prints the list only when it changes; `adb devices` while it cannot run.
pub async fn devices() -> Vec<String> {
    if let Some(list) = TRACKER.list() {
        return list;
    }
    let Ok(out) = adb(&["devices"]).await else {
        return Vec::new();
    };
    parse_devices(&out)
}

fn parse_devices(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            let (serial, state) = l.split_once('\t')?;
            (state.trim() == "device").then(|| serial.trim().to_owned())
        })
        .collect()
}

/// The tracker stops when nobody asked for this long (USB turned off).
const TRACK_IDLE: Duration = Duration::from_secs(60);

struct Tracker {
    state: Mutex<TrackState>,
}

#[derive(Default)]
struct TrackState {
    running: bool,
    /// Last list adb sent (None until the first one).
    list: Option<Vec<String>>,
    last_used: Option<std::time::Instant>,
    retry_at: Option<std::time::Instant>,
}

static TRACKER: Tracker = Tracker {
    state: Mutex::new(TrackState {
        running: false,
        list: None,
        last_used: None,
        retry_at: None,
    }),
};

impl Tracker {
    /// The current list, starting the tracker if needed; `None` while it
    /// has no list (starting, or adb too old / missing).
    fn list(&'static self) -> Option<Vec<String>> {
        let now = std::time::Instant::now();
        let mut s = self.state.lock().unwrap();
        s.last_used = Some(now);
        if s.running {
            return s.list.clone();
        }
        if s.retry_at.is_some_and(|t| now < t) {
            return None;
        }
        let path = find_adb()?;
        let mut cmd = konnekta_platform::host::command_async(path);
        cmd.arg("track-devices")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let Ok(mut child) = cmd.spawn() else {
            s.retry_at = Some(now + Duration::from_secs(60));
            return None;
        };
        let Some(out) = child.stdout.take() else {
            s.retry_at = Some(now + Duration::from_secs(60));
            return None;
        };
        s.running = true;
        s.list = None;
        tokio::spawn(self.follow(child, out, now));
        None
    }

    async fn follow(
        &'static self,
        child: tokio::process::Child,
        mut out: tokio::process::ChildStdout,
        started: std::time::Instant,
    ) {
        use tokio::io::AsyncReadExt;
        let _child = child;
        let read = async {
            loop {
                // Each list: 4 hex digits of length, then `serial\tstate\n`...
                let mut len = [0u8; 4];
                out.read_exact(&mut len).await.ok()?;
                let len = usize::from_str_radix(std::str::from_utf8(&len).ok()?, 16).ok()?;
                let mut body = vec![0u8; len.min(64 * 1024)];
                out.read_exact(&mut body).await.ok()?;
                let list = parse_devices(&String::from_utf8_lossy(&body));
                self.state.lock().unwrap().list = Some(list);
            }
            #[allow(unreachable_code)]
            Some(())
        };
        let idle = async {
            loop {
                tokio::time::sleep(TRACK_IDLE / 4).await;
                let last = self.state.lock().unwrap().last_used;
                if last.is_none_or(|t| t.elapsed() > TRACK_IDLE) {
                    return;
                }
            }
        };
        let stopped_idle = tokio::select! {
            _ = read => false,
            _ = idle => true,
        };
        let mut s = self.state.lock().unwrap();
        s.running = false;
        s.list = None;
        if !stopped_idle {
            // adb restarted or too old for track-devices: poll meanwhile.
            let wait = if started.elapsed() < Duration::from_secs(5) {
                300
            } else {
                2
            };
            s.retry_at = Some(std::time::Instant::now() + Duration::from_secs(wait));
        }
    }
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

/// Phones whose adbd cannot pick the port of a reverse mapping (`tcp:0`
/// support came with Android 7): a port is chosen here for them.
static NO_REVERSE_TCP0: Mutex<Vec<Arc<str>>> = Mutex::new(Vec::new());

/// Ports tried on the phone when its adbd cannot pick one: below Linux's
/// ephemeral range, so they never clash with the phone's outgoing sockets.
const REVERSE_PORTS: std::ops::RangeInclusive<u16> = 30000..=32767;
const REVERSE_TRIES: u16 = 8;

/// Make the local port `local` reachable from the phone: it connects to the
/// returned port on its own 127.0.0.1.
pub async fn reverse(serial: &Arc<str>, local: u16) -> Result<(u16, Mapping)> {
    let target = format!("tcp:{local}");
    let mapping = |port| {
        Mapping::Adb(AdbMapping {
            serial: serial.clone(),
            kind: "reverse",
            port,
        })
    };
    let picks_port = !NO_REVERSE_TCP0.lock().unwrap().contains(serial);
    if picks_port {
        match adb(&["-s", serial, "reverse", "tcp:0", &target]).await {
            Ok(out) if reported_port(&out).is_ok() => {
                let port = reported_port(&out)?;
                return Ok((port, mapping(port)));
            }
            Ok(_) => {
                // Android 6: adbd accepts tcp:0 and listens on some port, but
                // never says which. Drop that listener and choose the port,
                // now and for the rest of this run.
                tracing::info!(%serial, "adb reverse tcp:0 unsupported, choosing the port");
                let _ = adb(&["-s", serial, "reverse", "--remove", "tcp:0"]).await;
                NO_REVERSE_TCP0.lock().unwrap().push(serial.clone());
            }
            Err(e) => {
                // Maybe only a passing failure (device offline, adb restarting):
                // choose the port this time, try tcp:0 again next time.
                tracing::info!(%serial, error = %e, "adb reverse tcp:0 failed, choosing the port this time");
                let _ = adb(&["-s", serial, "reverse", "--remove", "tcp:0"]).await;
            }
        }
    }
    let span = REVERSE_PORTS.end() - REVERSE_PORTS.start() + 1;
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let first = (seed ^ u32::from(local)) % u32::from(span);
    let mut last = None;
    for i in 0..REVERSE_TRIES {
        let port = REVERSE_PORTS.start() + ((first + u32::from(i) * 389) % u32::from(span)) as u16;
        // --no-rebind: a port another mapping holds is skipped, not stolen.
        let spec = format!("tcp:{port}");
        match adb(&["-s", serial, "reverse", "--no-rebind", &spec, &target]).await {
            Ok(_) => return Ok((port, mapping(port))),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("adb reverse failed")))
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
