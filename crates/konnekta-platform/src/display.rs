//! Monitors of the desktop: the virtual one the phone shows when it extends
//! the screen, and where an absolute pointer position lands.
//!
//! Virtual monitors are made by the compositor itself: Hyprland
//! (`hyprctl output create headless`) and Sway (`swaymsg create_output`).
//! They behave as any monitor, so the system's display settings arrange,
//! scale or rotate them, and they go away when the phone stops.
//!
//! Hyprland is driven through its request socket (what `hyprctl` talks to),
//! in the classic syntax or, from Hyprland 0.57 with a Lua configuration,
//! in Lua (`dispatch` and `keyword` answer errors there): see
//! [`hyprland_call`].

use anyhow::{Context, Result, bail};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

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

/// Run a program of the system (Sway's `swaymsg`); its standard output.
fn run(program: &str, args: &[&str]) -> Result<String> {
    let out = crate::host::command(program)
        .args(args)
        .output()
        .with_context(|| format!("{program} not found"))?;
    if !out.status.success() {
        bail!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A request to Hyprland's socket, as `hyprctl` sends it (`j/monitors`,
/// `dispatch …`, `eval …`); the answer.
#[cfg(unix)]
pub fn hyprland_request(msg: &str) -> Result<String> {
    use std::io::{Read, Write};
    let path = std::path::Path::new(
        &std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR not set")?,
    )
    .join("hypr")
    .join(std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").context("Hyprland is not running")?)
    .join(".socket.sock");
    let mut s = std::os::unix::net::UnixStream::connect(&path)
        .with_context(|| format!("cannot reach Hyprland ({})", path.display()))?;
    let limit = Some(std::time::Duration::from_secs(3));
    s.set_read_timeout(limit)?;
    s.set_write_timeout(limit)?;
    s.write_all(msg.as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}

#[cfg(not(unix))]
pub fn hyprland_request(_: &str) -> Result<String> {
    bail!("Hyprland is not available here")
}

const SYNTAX_UNKNOWN: u8 = 0;
const SYNTAX_CLASSIC: u8 = 1;
const SYNTAX_LUA: u8 = 2;
/// Which syntax this Hyprland took last.
static HYPRLAND_SYNTAX: AtomicU8 = AtomicU8::new(SYNTAX_UNKNOWN);

fn first_line(s: &str) -> &str {
    s.trim().lines().next().unwrap_or_default()
}

/// A Hyprland request in the classic syntax (`dispatch movecursor 10 10`,
/// `keyword monitor …`) or, where Hyprland runs a Lua configuration (0.57
/// and later, where those answer `unknown request` or a Lua error), the Lua
/// code that does the same (`hl.dispatch(hl.dsp.cursor.move({ x = 10, y = 10 }))`).
/// The syntax that worked is tried first next time.
pub fn hyprland_call(classic: &str, lua: Option<&str>) -> Result<()> {
    let ok = |r: &str| r.trim() == "ok";
    let mode = HYPRLAND_SYNTAX.load(Ordering::Relaxed);
    let mut classic_err = None;
    if mode != SYNTAX_LUA || lua.is_none() {
        let r = hyprland_request(classic)?;
        if ok(&r) {
            HYPRLAND_SYNTAX.store(SYNTAX_CLASSIC, Ordering::Relaxed);
            return Ok(());
        }
        classic_err = Some(format!("{classic}: {}", first_line(&r)));
    }
    if let Some(code) = lua {
        let r = hyprland_request(&format!("eval {code}"))?;
        if ok(&r) {
            if mode != SYNTAX_LUA {
                tracing::info!("Hyprland takes Lua requests (Lua configuration)");
            }
            HYPRLAND_SYNTAX.store(SYNTAX_LUA, Ordering::Relaxed);
            return Ok(());
        }
        // In Lua mode the classic answer is only noise.
        bail!("Hyprland: {code}: {}", first_line(&r));
    }
    bail!("Hyprland: {}", classic_err.unwrap_or_default())
}

/// A Lua string literal of `s`, for [`hyprland_call`].
pub fn lua_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => out.push_str(&format!("\\{:03}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn monitors_json() -> Vec<serde_json::Value> {
    let out = match compositor() {
        Some(Compositor::Hyprland) => hyprland_request("j/monitors"),
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
        Some(Compositor::Hyprland) => hyprland_call(
            &format!("dispatch movecursor {x} {y}"),
            Some(&format!(
                "hl.dispatch(hl.dsp.cursor.move({{ x = {x}, y = {y} }}))"
            )),
        )?,
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

/// Where the pointer is, in layout coordinates. Hyprland only (through its
/// socket, cheap enough to ask many times a second): Sway does not tell.
#[cfg(unix)]
pub fn cursor_position() -> Option<(f64, f64)> {
    if compositor()? != Compositor::Hyprland {
        return None;
    }
    let out = hyprland_request("j/cursorpos").ok()?;
    let v: serde_json::Value = serde_json::from_str(out.trim()).ok()?;
    Some((v["x"].as_f64()?, v["y"].as_f64()?))
}

#[cfg(not(unix))]
pub fn cursor_position() -> Option<(f64, f64)> {
    None
}

/// Whether [`cursor_position`] can answer here.
pub fn can_report_cursor() -> bool {
    compositor() == Some(Compositor::Hyprland)
}

/// The pointer as a share of the monitor `name` (outside 0..=1 when it is
/// on another monitor).
pub fn cursor_on(name: &str) -> Option<(f64, f64)> {
    let r = monitor_region(name)?;
    let (x, y) = cursor_position()?;
    Some(((x - r.x) / r.w.max(1.0), (y - r.y) / r.h.max(1.0)))
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
                let _ = hyprland_request(&format!("output remove {name}"));
                let r = hyprland_request(&format!("output create headless {name}"))?;
                if r.trim() != "ok" {
                    bail!(
                        "Hyprland: output create headless {name}: {}",
                        first_line(&r)
                    );
                }
                let mode = format!("{w}x{h}@{refresh}");
                let set = hyprland_call(
                    &format!("keyword monitor {name},{mode},auto-right,1"),
                    Some(&format!(
                        "hl.monitor({{ output = {}, mode = {}, position = \"auto-right\", scale = 1 }})",
                        lua_string(&name),
                        lua_string(&mode)
                    )),
                );
                if let Err(e) = set {
                    // Not left behind holding a workspace.
                    let _ = hyprland_request(&format!("output remove {name}"));
                    return Err(e);
                }
                wait_for_size(&name, w, h);
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

/// Hyprland applies a monitor's mode a moment later: the capture starts
/// once the monitor has its size (a capture started before would stop when
/// the size changes).
fn wait_for_size(name: &str, w: u32, h: u32) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let sized = monitors_json().iter().any(|m| {
            m["name"].as_str() == Some(name)
                && m["width"].as_u64() == Some(u64::from(w))
                && m["height"].as_u64() == Some(u64::from(h))
        });
        if sized {
            return;
        }
        if std::time::Instant::now() > deadline {
            tracing::warn!(monitor = %name, "virtual monitor still without its {w}x{h} mode");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

impl Drop for VirtualMonitor {
    fn drop(&mut self) {
        let mut ext = EXTEND.lock().unwrap();
        if ext.as_deref() == Some(self.name.as_str()) {
            *ext = None;
        }
        let r = match self.compositor {
            Compositor::Hyprland => hyprland_request(&format!("output remove {}", self.name))
                .and_then(|r| match r.trim() {
                    "ok" => Ok(String::new()),
                    other => bail!("{other}"),
                }),
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

    #[test]
    fn lua_strings_are_escaped() {
        assert_eq!(lua_string("KONNEKTA-1"), r#""KONNEKTA-1""#);
        assert_eq!(lua_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(lua_string("x\ny\u{1}"), r#""x\ny\001""#);
    }

    /// Needs a Hyprland session: `cargo test -p konnekta-platform -- --ignored hyprland_live`.
    #[test]
    #[ignore]
    fn hyprland_live_cursor() {
        let (x0, y0) = cursor_position().expect("Hyprland cursor");
        move_cursor(x0 as i32 + 40, y0 as i32 + 30).unwrap();
        let (x, y) = cursor_position().unwrap();
        move_cursor(x0 as i32, y0 as i32).unwrap();
        assert_eq!((x as i32, y as i32), (x0 as i32 + 40, y0 as i32 + 30));
    }

    /// Needs a Hyprland session (adds and removes a monitor for a moment).
    #[test]
    #[ignore]
    fn hyprland_live_virtual_monitor() {
        let m = VirtualMonitor::create(1080, 2400, 60).unwrap();
        let name = m.name.clone();
        let r = monitor_region(&name).expect("the new monitor");
        assert_eq!((r.w as u32, r.h as u32), (1080, 2400));
        drop(m);
        assert!(
            !monitors_json()
                .iter()
                .any(|x| x["name"].as_str() == Some(name.as_str()))
        );
    }
}
