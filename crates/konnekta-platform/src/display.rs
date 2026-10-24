//! Monitors of the desktop: the virtual one the phone shows when it extends
//! the screen, and where an absolute pointer position lands.
//!
//! Virtual monitors are made by the compositor itself: Hyprland
//! (`hyprctl output create headless`) and Sway (`swaymsg create_output`).
//! They behave as any monitor, so the system's display settings arrange,
//! scale or rotate them, and they go away when the phone stops.

use anyhow::{Context, Result, bail};
use std::process::Command;
use std::sync::Mutex;

/// A monitor in layout (logical) coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Region {
    /// The point at `(fx, fy)` (0..=1) of the region.
    pub fn at(&self, fx: f64, fy: f64) -> (i32, i32) {
        let fx = fx.clamp(0.0, 1.0);
        let fy = fy.clamp(0.0, 1.0);
        (
            (self.x + fx * (self.w - 1.0).max(0.0)).round() as i32,
            (self.y + fy * (self.h - 1.0).max(0.0)).round() as i32,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Compositor {
    Hyprland,
    Sway,
}

fn compositor() -> Option<Compositor> {
    if cfg!(not(target_os = "linux")) {
        return None;
    }
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        Some(Compositor::Hyprland)
    } else if std::env::var_os("SWAYSOCK").is_some() {
        Some(Compositor::Sway)
    } else {
        None
    }
}

fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("{program} not found"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        bail!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    // hyprctl answers errors on stdout with a zero exit code.
    if program == "hyprctl" && !args.contains(&"-j") && text.trim() != "ok" && !text.is_empty() {
        bail!("hyprctl {}: {}", args.join(" "), text.trim());
    }
    Ok(text)
}

fn monitors_json() -> Vec<serde_json::Value> {
    let out = match compositor() {
        Some(Compositor::Hyprland) => run("hyprctl", &["monitors", "-j"]),
        Some(Compositor::Sway) => run("swaymsg", &["-t", "get_outputs", "-r"]),
        None => return Vec::new(),
    };
    out.ok()
        .and_then(|o| serde_json::from_str(&o).ok())
        .unwrap_or_default()
}

/// Layout region of a monitor, by name. Kept for a second: the pointer
/// asks for it at every move.
pub fn monitor_region(name: &str) -> Option<Region> {
    static CACHE: Mutex<Option<(String, Region, std::time::Instant)>> = Mutex::new(None);
    if let Some((n, r, at)) = CACHE.lock().unwrap().as_ref()
        && n == name
        && at.elapsed() < std::time::Duration::from_secs(1)
    {
        return Some(*r);
    }
    let r = monitor_region_now(name)?;
    *CACHE.lock().unwrap() = Some((name.to_owned(), r, std::time::Instant::now()));
    Some(r)
}

fn monitor_region_now(name: &str) -> Option<Region> {
    let ms = monitors_json();
    let m = ms.iter().find(|m| m["name"].as_str() == Some(name))?;
    region_of(m)
}

fn region_of(m: &serde_json::Value) -> Option<Region> {
    match compositor()? {
        Compositor::Hyprland => {
            let scale = m["scale"].as_f64().unwrap_or(1.0).max(0.1);
            let (mut w, mut h) = (m["width"].as_f64()?, m["height"].as_f64()?);
            if m["transform"].as_u64().unwrap_or(0) % 2 == 1 {
                std::mem::swap(&mut w, &mut h);
            }
            Some(Region {
                x: m["x"].as_f64()?,
                y: m["y"].as_f64()?,
                w: w / scale,
                h: h / scale,
            })
        }
        Compositor::Sway => {
            let r = &m["rect"];
            Some(Region {
                x: r["x"].as_f64()?,
                y: r["y"].as_f64()?,
                w: r["width"].as_f64()?,
                h: r["height"].as_f64()?,
            })
        }
    }
}

/// The monitor screen capture and remote desktop use: the focused one,
/// never a phone's virtual monitor.
pub fn focused_monitor() -> Option<String> {
    let ms = monitors_json();
    let real = |m: &&serde_json::Value| {
        !m["name"]
            .as_str()
            .is_some_and(|n| n.starts_with(VIRTUAL_PREFIX))
    };
    let m = ms
        .iter()
        .filter(real)
        .find(|m| m["focused"].as_bool() == Some(true))
        .or_else(|| ms.iter().find(real))?;
    m["name"].as_str().map(str::to_owned)
}

/// Move the pointer to a point of the layout (any monitor).
pub fn move_cursor(x: i32, y: i32) -> Result<()> {
    match compositor() {
        Some(Compositor::Hyprland) => {
            run(
                "hyprctl",
                &["dispatch", "movecursor", &x.to_string(), &y.to_string()],
            )?;
        }
        Some(Compositor::Sway) => {
            run(
                "swaymsg",
                &["seat", "-", "cursor", "set", &x.to_string(), &y.to_string()],
            )?;
        }
        None => bail!("no compositor to place the pointer"),
    }
    Ok(())
}

/// Whether the pointer can be placed through the compositor.
pub fn can_move_cursor() -> bool {
    compositor().is_some()
}

/// Whether this desktop can make a virtual monitor for the phone.
pub fn supports_virtual_monitor() -> bool {
    compositor().is_some()
}

const VIRTUAL_PREFIX: &str = "KONNEKTA-";

/// Monitor the phone shows while it extends the screen.
static EXTEND: Mutex<Option<String>> = Mutex::new(None);
/// Monitor remote desktop captures.
static CAPTURED: Mutex<Option<String>> = Mutex::new(None);

/// Region an absolute pointer position from the phone refers to: the
/// virtual monitor (`extend`) or the monitor remote desktop shows.
pub fn pointer_region(extend: bool) -> Option<Region> {
    let name = if extend {
        EXTEND.lock().unwrap().clone()
    } else {
        CAPTURED.lock().unwrap().clone()
    }?;
    monitor_region(&name)
}

/// Remote desktop captures this monitor (None when it ends).
pub fn set_captured_monitor(name: Option<String>) {
    *CAPTURED.lock().unwrap() = name;
}

/// A virtual monitor, removed when dropped.
pub struct VirtualMonitor {
    pub name: String,
    compositor: Compositor,
}

impl VirtualMonitor {
    /// A new monitor of `width` x `height` pixels at the right of the others.
    pub fn create(width: u32, height: u32, refresh: u32) -> Result<Self> {
        let comp = compositor().context(
            "estendere lo schermo richiede Hyprland o Sway (altri desktop non permettono ancora di creare un monitor virtuale)",
        )?;
        let (w, h) = (
            width.clamp(320, 7680) / 2 * 2,
            height.clamp(240, 4320) / 2 * 2,
        );
        let name = match comp {
            Compositor::Hyprland => {
                let name = format!("{VIRTUAL_PREFIX}1");
                // A monitor left by a crash.
                let _ = run("hyprctl", &["output", "remove", &name]);
                run("hyprctl", &["output", "create", "headless", &name])?;
                run(
                    "hyprctl",
                    &[
                        "keyword",
                        "monitor",
                        &format!("{name},{w}x{h}@{refresh},auto-right,1"),
                    ],
                )?;
                name
            }
            Compositor::Sway => {
                let before: Vec<String> = monitors_json()
                    .iter()
                    .filter_map(|m| m["name"].as_str().map(str::to_owned))
                    .collect();
                run("swaymsg", &["create_output"])?;
                let ms = monitors_json();
                let name = ms
                    .iter()
                    .filter_map(|m| m["name"].as_str())
                    .find(|n| !before.iter().any(|b| b == n))
                    .context("sway made no new output")?
                    .to_owned();
                let right = ms
                    .iter()
                    .filter(|m| m["name"].as_str() != Some(name.as_str()))
                    .filter_map(region_of)
                    .map(|r| (r.x + r.w) as i64)
                    .max()
                    .unwrap_or(0);
                run(
                    "swaymsg",
                    &[
                        "output",
                        &name,
                        "mode",
                        "--custom",
                        &format!("{w}x{h}@{refresh}Hz"),
                        "position",
                        &right.to_string(),
                        "0",
                    ],
                )?;
                name
            }
        };
        *EXTEND.lock().unwrap() = Some(name.clone());
        tracing::info!(monitor = %name, "virtual monitor {w}x{h} for the phone");
        Ok(Self {
            name,
            compositor: comp,
        })
    }
}

impl Drop for VirtualMonitor {
    fn drop(&mut self) {
        let mut ext = EXTEND.lock().unwrap();
        if ext.as_deref() == Some(self.name.as_str()) {
            *ext = None;
        }
        let r = match self.compositor {
            Compositor::Hyprland => run("hyprctl", &["output", "remove", &self.name]),
            Compositor::Sway => run("swaymsg", &["output", &self.name, "unplug"]),
        };
        if let Err(e) = r {
            tracing::warn!("virtual monitor not removed: {e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_land_inside_the_region() {
        let r = Region {
            x: 2048.0,
            y: 0.0,
            w: 1600.0,
            h: 720.0,
        };
        assert_eq!(r.at(0.0, 0.0), (2048, 0));
        assert_eq!(r.at(1.0, 1.0), (3647, 719));
        assert_eq!(r.at(2.0, -1.0), (3647, 0));
    }
}
