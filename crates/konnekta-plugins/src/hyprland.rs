//! Hyprland integration (Linux, local only: no packets).
//!
//! * `theme`: accent colours and rounding from the running compositor plus
//!   the Waybar palette (`@define-color`), so the app matches the desktop;
//! * `workspaces` and `dispatch`: list workspaces and run `hyprctl dispatch`
//!   (argv, no shell). Deck buttons use them through a `plugin` action, e.g.
//!   `{ "type": "plugin", "plugin": "hyprland", "action": "dispatch",
//!      "args": { "dispatcher": "workspace", "args": "3" } }`.

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

/// Colours of a gradient option (`"ee33ccff ee00ff99 45deg"`).
pub fn gradient(option: &Value) -> Vec<String> {
    let s = option
        .get("custom")
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

pub fn running() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

pub(crate) async fn hyprctl(args: &[&str]) -> Result<String> {
    let out = tokio::time::timeout(
        HYPRCTL_TIMEOUT,
        tokio::process::Command::new("hyprctl")
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
                let mut argv = vec!["dispatch", d];
                if !a.is_empty() {
                    argv.push(a);
                }
                let out = hyprctl(&argv).await?;
                if out.trim() != "ok" {
                    bail!("hyprctl: {}", out.trim());
                }
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
    fn dispatchers_are_identifiers() {
        assert!(valid_dispatcher("movetoworkspace"));
        assert!(!valid_dispatcher("exec; rm"));
        assert!(!valid_dispatcher(""));
    }
}
