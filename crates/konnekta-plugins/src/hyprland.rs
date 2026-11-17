//! Hyprland integration (Linux, local only: no packets).
//!
//! * `theme`: accent colours and rounding from the running compositor plus
//!   the Waybar palette (`@define-color`), so the app matches the desktop;
//! * `workspaces` and `dispatch`: list workspaces and run `hyprctl dispatch`
//!   (argv, no shell). Deck buttons use them through a `plugin` action, e.g.
//!   `{ "type": "plugin", "plugin": "hyprland", "action": "dispatch",
//!      "args": { "dispatcher": "workspace", "args": "3" } }`.
//!   On Hyprland 0.57+ with a Lua configuration the common dispatchers are
//!   sent as their Lua equivalent (see [`lua_dispatcher`]).

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::Packet;
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

const HYPRCTL_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    /// CSS colours (`#rrggbb` or `rgba(...)`).
    pub accent: Option<String>,
    pub accent_alt: Option<String>,
    pub inactive: Option<String>,
    pub rounding: Option<i64>,
    /// Named colours from the Waybar stylesheet.
    pub palette: BTreeMap<String, String>,
}

/// Hyprland colour `AARRGGBB` (or `0xAARRGGBB`) -> CSS.
pub fn css_color(argb: &str) -> Option<String> {
    let h = argb.trim().trim_start_matches("0x");
    if h.len() != 8 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let (a, rgb) = h.split_at(2);
    let a = u8::from_str_radix(a, 16).ok()?;
    Some(if a == 0xff {
        format!("#{}", rgb.to_lowercase())
    } else {
        let v = |i: usize| u8::from_str_radix(&rgb[i..i + 2], 16).unwrap_or(0);
        format!(
            "rgba({}, {}, {}, {:.2})",
            v(0),
            v(2),
            v(4),
            a as f32 / 255.0
        )
    })
}

/// Colours of a gradient option (`"ee33ccff ee00ff99 45deg"`): `custom`
/// up to Hyprland 0.56, `gradient` from 0.57.
pub fn gradient(option: &Value) -> Vec<String> {
    let s = option
        .get("custom")
        .or_else(|| option.get("gradient"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    s.split_whitespace().filter_map(css_color).collect()
}

/// `@define-color name #hex;` lines of a GTK stylesheet.
pub fn define_colors(css: &str) -> BTreeMap<String, String> {
    css.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("@define-color")?;
            let mut it = rest.split_whitespace();
            let name = it.next()?;
            let value = it.collect::<Vec<_>>().join(" ");
            let value = value.trim_end_matches(';').trim();
            let ok = name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && (value.starts_with('#') || value.starts_with("rgb"))
                && value.len() <= 32
                && !value.contains(['{', '}', ';', '"', '\'']);
            ok.then(|| (name.to_owned(), value.to_owned()))
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    pub windows: i64,
    pub active: bool,
}

pub fn workspaces(list: &Value, active_id: Option<i64>) -> Vec<Workspace> {
    let mut out: Vec<Workspace> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let id = w.get("id")?.as_i64()?;
            // Special workspaces (scratchpads) have negative ids.
            (id > 0).then(|| Workspace {
                id,
                name: w
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                windows: w.get("windows").and_then(Value::as_i64).unwrap_or(0),
                active: Some(id) == active_id,
            })
        })
        .collect();
    out.sort_by_key(|w| w.id);
    out
}

/// Dispatcher names are plain identifiers (`workspace`, `movetoworkspace`...).
pub fn valid_dispatcher(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 40
        && d.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The Lua dispatcher (Hyprland 0.57+ with a Lua configuration) that does
/// what the classic `dispatcher args` does, for the common ones.
pub fn lua_dispatcher(dispatcher: &str, args: &str) -> Option<String> {
    use konnekta_platform::display::lua_string as q;
    let a = args.trim();
    // Workspaces are numbers or names (`e+1`, `special:magic`, `name:web`).
    let ws = || match a.parse::<i64>() {
        Ok(n) => n.to_string(),
        Err(_) => q(a),
    };
    let none = a.is_empty();
    Some(match dispatcher {
        "workspace" if !none => format!("hl.dsp.focus({{ workspace = {} }})", ws()),
        "movetoworkspace" if !none => format!("hl.dsp.window.move({{ workspace = {} }})", ws()),
        "togglespecialworkspace" => format!(
            "hl.dsp.workspace.toggle_special({})",
            if none { String::new() } else { q(a) }
        ),
        "movefocus" => {
            let dir = match a {
                "l" | "left" => "left",
                "r" | "right" => "right",
                "u" | "up" => "up",
                "d" | "down" => "down",
                _ => return None,
            };
            format!("hl.dsp.focus({{ direction = \"{dir}\" }})")
        }
        "exec" if !none => format!("hl.dsp.exec_cmd({})", q(a)),
        "killactive" if none => "hl.dsp.window.close()".into(),
        "forcekillactive" if none => "hl.dsp.window.kill()".into(),
        "togglefloating" if none => "hl.dsp.window.float({ action = \"toggle\" })".into(),
        "fullscreen" if none => "hl.dsp.window.fullscreen()".into(),
        "pseudo" if none => "hl.dsp.window.pseudo()".into(),
        "pin" if none => "hl.dsp.window.pin()".into(),
        "centerwindow" if none => "hl.dsp.window.center()".into(),
        "exit" if none => "hl.dsp.exit()".into(),
        "movecursor" => {
            let mut it = a.split_whitespace().map(str::parse::<i64>);
            let (Some(Ok(x)), Some(Ok(y)), None) = (it.next(), it.next(), it.next()) else {
                return None;
            };
            format!("hl.dsp.cursor.move({{ x = {x}, y = {y} }})")
        }
        _ => return None,
    })
    .map(|d| format!("hl.dispatch({d})"))
}

/// Run a dispatcher, in the syntax this Hyprland takes.
pub(crate) async fn dispatch(dispatcher: &str, args: &str) -> Result<()> {
    let classic = if args.is_empty() {
        format!("dispatch {dispatcher}")
    } else {
        format!("dispatch {dispatcher} {args}")
    };
    let lua = lua_dispatcher(dispatcher, args);
    tokio::task::spawn_blocking(move || {
        konnekta_platform::display::hyprland_call(&classic, lua.as_deref())
    })
    .await?
}

pub fn running() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

pub(crate) async fn hyprctl(args: &[&str]) -> Result<String> {
    let out = tokio::time::timeout(
        HYPRCTL_TIMEOUT,
        konnekta_platform::host::command_async("hyprctl")
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("hyprctl timed out")?
    .context("hyprctl not found")?;
    if !out.status.success() {
        bail!("hyprctl failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn hyprctl_json(args: &[&str]) -> Result<Value> {
    let mut a = vec!["-j"];
    a.extend_from_slice(args);
    Ok(serde_json::from_str(&hyprctl(&a).await?)?)
}

fn waybar_css() -> Option<PathBuf> {
    let p = directories::BaseDirs::new()?
        .config_dir()
        .join("waybar/style.css");
    p.is_file().then_some(p)
}

pub struct HyprlandPlugin;

impl HyprlandPlugin {
    async fn theme(&self) -> Theme {
        let mut t = Theme::default();
        if running() {
            if let Ok(v) = hyprctl_json(&["getoption", "general:col.active_border"]).await {
                let g = gradient(&v);
                t.accent = g.first().cloned();
                t.accent_alt = g.get(1).cloned();
            }
            if let Ok(v) = hyprctl_json(&["getoption", "general:col.inactive_border"]).await {
                t.inactive = gradient(&v).into_iter().next();
            }
            if let Ok(v) = hyprctl_json(&["getoption", "decoration:rounding"]).await {
                t.rounding = v.get("int").and_then(Value::as_i64);
            }
        }
        if let Some(css) = waybar_css().and_then(|p| std::fs::read_to_string(p).ok()) {
            t.palette = define_colors(&css);
        }
        t
    }
}

#[async_trait]
impl Plugin for HyprlandPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "hyprland",
            name: "Hyprland",
            description: "Theme from Hyprland and Waybar, workspaces and dispatchers for the Deck",
            permission: Permission::Deck,
            incoming: &[],
            outgoing: &[],
        }
    }

    async fn handle_packet(&self, _: &DeviceHandle, _: Packet) -> Result<()> {
        Ok(())
    }

    async fn handle_action(
        &self,
        _: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        match action {
            "status" => {
                let version = if running() {
                    hyprctl_json(&["version"])
                        .await
                        .ok()
                        .and_then(|v| v.get("tag").cloned())
                } else {
                    None
                };
                Ok(json!({ "running": running(), "version": version }))
            }
            "theme" => Ok(serde_json::to_value(self.theme().await)?),
            "workspaces" => {
                if !running() {
                    bail!("Hyprland is not running");
                }
                let list = hyprctl_json(&["workspaces"]).await?;
                let active = hyprctl_json(&["activeworkspace"])
                    .await
                    .ok()
                    .and_then(|v| v.get("id")?.as_i64());
                Ok(serde_json::to_value(workspaces(&list, active))?)
            }
            "dispatch" => {
                if !running() {
                    bail!("Hyprland is not running");
                }
                let d = args
                    .get("dispatcher")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !valid_dispatcher(d) {
                    bail!("invalid dispatcher");
                }
                let a = args.get("args").and_then(Value::as_str).unwrap_or_default();
                if a.len() > 500 || a.contains(['\n', '\r']) {
                    bail!("invalid arguments");
                }
                dispatch(d, a.trim()).await?;
                Ok(Value::Null)
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_gradients() {
        assert_eq!(css_color("ff89b4fa").as_deref(), Some("#89b4fa"));
        assert_eq!(
            css_color("ee33ccff").as_deref(),
            Some("rgba(51, 204, 255, 0.93)")
        );
        assert_eq!(css_color("45deg"), None);
        let v = json!({ "option": "general:col.active_border", "custom": "ee33ccff ee00ff99 45deg", "set": true });
        assert_eq!(gradient(&v).len(), 2);
    }

    #[test]
    fn waybar_palette_is_parsed_and_filtered() {
        let css = "@define-color base    #11111b;\n@define-color text #cdd6f4;\n@define-color evil #fff; } * { x: url(a);\n* { color: red; }";
        let p = define_colors(css);
        assert_eq!(p.get("base").map(String::as_str), Some("#11111b"));
        assert_eq!(p.get("text").map(String::as_str), Some("#cdd6f4"));
        assert!(!p.contains_key("evil"));
    }

    #[test]
    fn workspace_list_skips_special_and_marks_active() {
        let v = json!([
            { "id": 3, "name": "3", "windows": 1 },
            { "id": -98, "name": "special:magic", "windows": 2 },
            { "id": 1, "name": "1", "windows": 4 }
        ]);
        let w = workspaces(&v, Some(3));
        assert_eq!(w.iter().map(|w| w.id).collect::<Vec<_>>(), [1, 3]);
        assert!(w[1].active && !w[0].active);
    }

    #[test]
    fn gradient_of_hyprland_0_57() {
        let v = json!({ "option": "general:col.active_border", "gradient": "ee33ccff ee00ff99 45deg", "set": true });
        assert_eq!(gradient(&v).len(), 2);
    }

    #[test]
    fn classic_dispatchers_in_lua() {
        let l = |d, a| lua_dispatcher(d, a);
        assert_eq!(
            l("workspace", "3").as_deref(),
            Some("hl.dispatch(hl.dsp.focus({ workspace = 3 }))")
        );
        assert_eq!(
            l("workspace", "e+1").as_deref(),
            Some(r#"hl.dispatch(hl.dsp.focus({ workspace = "e+1" }))"#)
        );
        assert_eq!(
            l("movetoworkspace", "special:magic").as_deref(),
            Some(r#"hl.dispatch(hl.dsp.window.move({ workspace = "special:magic" }))"#)
        );
        assert_eq!(
            l("exec", r#"notify-send "hi""#).as_deref(),
            Some(r#"hl.dispatch(hl.dsp.exec_cmd("notify-send \"hi\""))"#)
        );
        assert_eq!(
            l("movecursor", "10 20").as_deref(),
            Some("hl.dispatch(hl.dsp.cursor.move({ x = 10, y = 20 }))")
        );
        assert_eq!(l("movecursor", "10"), None);
        assert_eq!(
            l("killactive", "").as_deref(),
            Some("hl.dispatch(hl.dsp.window.close())")
        );
        assert_eq!(l("someplugin", "x"), None);
    }

    #[test]
    fn dispatchers_are_identifiers() {
        assert!(valid_dispatcher("movetoworkspace"));
        assert!(!valid_dispatcher("exec; rm"));
        assert!(!valid_dispatcher(""));
    }
}
