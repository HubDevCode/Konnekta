//! Mobile Deck: the phone becomes a Stream Deck-like control surface.
//!
//! Layout and actions live on this computer. The phone receives only the
//! *appearance* (label, icon, colour, position, category) and presses buttons
//! by id; it never sees or sends the underlying commands.
//!
//! Protocol (`konnekta.deck.request` from phone):
//! * `{request: "layout"}` → `konnekta.deck {type: "layout", ...}`
//! * `{request: "press", profile, page, button}`
//! * `{request: "setProfile", profile}`
//!
//! Push from computer: `konnekta.deck {type: "layout"}` when the layout
//! changes, `{type: "state", states: {buttonId: {active, label?}}}` when a
//! dynamic button state changes.

use crate::actions::{Action, Executor, validate_all};
use crate::store::JsonStore;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::{Packet, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo};
use konnekta_platform::Platform;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StateSource {
    /// Active while local media is playing.
    MediaPlaying,
    /// Active while Do Not Disturb is on.
    DoNotDisturb,
    /// Active when the shell command exits with 0 (polled).
    Shell {
        command: String,
        #[serde(default = "default_interval", rename = "intervalSecs")]
        interval_secs: u64,
    },
    /// Show the system volume as label.
    Volume,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Button {
    pub id: String,
    /// Index in the grid (row-major).
    pub position: u16,
    pub label: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Alternate actions when the state is active (toggle buttons).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<StateSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_color: Option<String>,
}

fn default_color() -> String {
    "#3b4252".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub buttons: Vec<Button>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default = "default_cols")]
    pub columns: u8,
    #[serde(default = "default_rows")]
    pub rows: u8,
    pub pages: Vec<Page>,
}

fn default_cols() -> u8 {
    4
}
fn default_rows() -> u8 {
    3
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckConfig {
    pub active_profile: String,
    pub profiles: Vec<Profile>,
}

impl Default for DeckConfig {
    fn default() -> Self {
        default_profiles()
    }
}

fn id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn btn(pos: u16, label: &str, icon: &str, color: &str, cat: &str, actions: Vec<Action>) -> Button {
    Button {
        id: id(),
        position: pos,
        label: label.into(),
        icon: icon.into(),
        color: color.into(),
        category: cat.into(),
        actions,
        active_actions: vec![],
        state: None,
        active_color: None,
    }
}

fn media(a: &str) -> Vec<Action> {
    vec![Action::Media { action: a.into() }]
}

fn keys(k: &str) -> Vec<Action> {
    vec![Action::Shortcut { keys: k.into() }]
}

pub fn default_profiles() -> DeckConfig {
    let mut play = btn(
        0,
        "Play/Pausa",
        "play_pause",
        "#2e7d32",
        "Media",
        media("PlayPause"),
    );
    play.state = Some(StateSource::MediaPlaying);
    play.active_color = Some("#43a047".into());
    let mut vol = btn(
        3,
        "Volume",
        "volume",
        "#37474f",
        "Media",
        vec![Action::Volume {
            set: None,
            delta: Some(5),
        }],
    );
    vol.state = Some(StateSource::Volume);
    let mut dnd = btn(
        4,
        "Non disturbare",
        "dnd",
        "#4a148c",
        "Sistema",
        vec![Action::Plugin {
            plugin: "dnd".into(),
            action: "toggle".into(),
            args: Value::Null,
            device: None,
        }],
    );
    dnd.state = Some(StateSource::DoNotDisturb);
    dnd.active_color = Some("#8e24aa".into());
    let work = Profile {
        id: "work".into(),
        name: "Lavoro".into(),
        icon: "work".into(),
        columns: 4,
        rows: 3,
        pages: vec![Page {
            id: "main".into(),
            name: "Principale".into(),
            buttons: vec![
                play,
                btn(
                    1,
                    "Precedente",
                    "skip_previous",
                    "#37474f",
                    "Media",
                    media("Previous"),
                ),
                btn(
                    2,
                    "Successivo",
                    "skip_next",
                    "#37474f",
                    "Media",
                    media("Next"),
                ),
                vol,
                dnd,
                btn(
                    5,
                    "Copia",
                    "content_copy",
                    "#1565c0",
                    "Modifica",
                    keys("ctrl+c"),
                ),
                btn(
                    6,
                    "Incolla",
                    "content_paste",
                    "#1565c0",
                    "Modifica",
                    keys("ctrl+v"),
                ),
                btn(7, "Cerca", "search", "#1565c0", "Modifica", keys("ctrl+f")),
                btn(
                    8,
                    "Mute volume",
                    "volume_off",
                    "#b71c1c",
                    "Media",
                    vec![Action::Volume {
                        set: Some(0),
                        delta: None,
                    }],
                ),
                btn(
                    9,
                    "Cambia finestra",
                    "tab",
                    "#455a64",
                    "Sistema",
                    keys("alt+tab"),
                ),
            ],
        }],
    };
    let streaming = Profile {
        id: "streaming".into(),
        name: "Streaming".into(),
        icon: "videocam".into(),
        columns: 4,
        rows: 3,
        pages: vec![Page {
            id: "obs".into(),
            name: "OBS".into(),
            buttons: vec![
                btn(
                    0,
                    "Scena 1",
                    "looks_one",
                    "#6a1b9a",
                    "Scene",
                    keys("ctrl+shift+F1"),
                ),
                btn(
                    1,
                    "Scena 2",
                    "looks_two",
                    "#6a1b9a",
                    "Scene",
                    keys("ctrl+shift+F2"),
                ),
                btn(
                    2,
                    "Scena 3",
                    "looks_3",
                    "#6a1b9a",
                    "Scene",
                    keys("ctrl+shift+F3"),
                ),
                btn(
                    3,
                    "Live",
                    "sensors",
                    "#c62828",
                    "Stream",
                    keys("ctrl+shift+F9"),
                ),
                btn(
                    4,
                    "Registra",
                    "fiber_manual_record",
                    "#c62828",
                    "Stream",
                    keys("ctrl+shift+F10"),
                ),
                btn(
                    5,
                    "Mute mic",
                    "mic_off",
                    "#ef6c00",
                    "Audio",
                    keys("ctrl+shift+m"),
                ),
            ],
        }],
    };
    let presentation = Profile {
        id: "presentation".into(),
        name: "Presentazione".into(),
        icon: "slideshow".into(),
        columns: 3,
        rows: 2,
        pages: vec![Page {
            id: "slides".into(),
            name: "Slide".into(),
            buttons: vec![
                btn(
                    0,
                    "Indietro",
                    "arrow_back",
                    "#0277bd",
                    "Slide",
                    keys("PageUp"),
                ),
                btn(1, "Avvia", "play_arrow", "#2e7d32", "Slide", keys("F5")),
                btn(
                    2,
                    "Avanti",
                    "arrow_forward",
                    "#0277bd",
                    "Slide",
                    keys("PageDown"),
                ),
                btn(
                    3,
                    "Schermo nero",
                    "dark_mode",
                    "#212121",
                    "Slide",
                    keys("b"),
                ),
                btn(4, "Esci", "stop", "#c62828", "Slide", keys("Escape")),
            ],
        }],
    };
    let gaming = Profile {
        id: "gaming".into(),
        name: "Gaming".into(),
        icon: "sports_esports".into(),
        columns: 4,
        rows: 2,
        pages: vec![Page {
            id: "game".into(),
            name: "Gioco".into(),
            buttons: vec![
                btn(
                    0,
                    "Screenshot",
                    "screenshot",
                    "#00695c",
                    "Cattura",
                    keys("F12"),
                ),
                btn(
                    1,
                    "Mute Discord",
                    "headset_off",
                    "#5865f2",
                    "Voce",
                    keys("ctrl+shift+m"),
                ),
                btn(
                    2,
                    "Push to talk",
                    "record_voice_over",
                    "#5865f2",
                    "Voce",
                    keys("ctrl+`"),
                ),
            ],
        }],
    };
    DeckConfig {
        active_profile: "work".into(),
        profiles: vec![work, streaming, presentation, gaming],
    }
}

impl DeckConfig {
    pub fn validate(&self) -> Result<()> {
        if self.profiles.is_empty() {
            bail!("at least one profile is required");
        }
        let mut ids = std::collections::HashSet::new();
        for p in &self.profiles {
            if !ids.insert(&p.id) {
                bail!("duplicate profile id {}", p.id);
            }
            if p.columns == 0 || p.rows == 0 || p.columns > 12 || p.rows > 12 {
                bail!("invalid grid size in profile {}", p.name);
            }
            let cap = p.columns as u16 * p.rows as u16;
            for page in &p.pages {
                let mut pos = std::collections::HashSet::new();
                for b in &page.buttons {
                    if b.position >= cap {
                        bail!("button '{}' is outside the grid", b.label);
                    }
                    if !pos.insert(b.position) {
                        bail!(
                            "two buttons share position {} on page {}",
                            b.position,
                            page.name
                        );
                    }
                    validate_all(&b.actions)?;
                    validate_all(&b.active_actions)?;
                }
            }
        }
        Ok(())
    }

    pub fn find_button(&self, profile: &str, page: &str, button: &str) -> Option<&Button> {
        self.profiles
            .iter()
            .find(|p| p.id == profile)?
            .pages
            .iter()
            .find(|pg| pg.id == page)?
            .buttons
            .iter()
            .find(|b| b.id == button)
    }

    /// Appearance-only view sent to the phone (no commands).
    pub fn public_layout(&self) -> Value {
        let profiles: Vec<Value> = self
            .profiles
            .iter()
            .map(|p| {
                json!({
                    "id": p.id, "name": p.name, "icon": p.icon, "columns": p.columns, "rows": p.rows,
                    "pages": p.pages.iter().map(|pg| json!({
                        "id": pg.id, "name": pg.name,
                        "buttons": pg.buttons.iter().map(|b| json!({
                            "id": b.id, "position": b.position, "label": b.label, "icon": b.icon,
                            "color": b.color, "category": b.category, "activeColor": b.active_color,
                            "stateful": b.state.is_some(),
                        })).collect::<Vec<_>>(),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        json!({ "type": "layout", "activeProfile": self.active_profile, "profiles": profiles })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ButtonState {
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

pub struct DeckPlugin {
    platform: Platform,
    executor: Executor,
    core: OnceLock<CoreHandle>,
    pub config: JsonStore<DeckConfig>,
    states: Mutex<BTreeMap<String, ButtonState>>,
    shell_cache: Mutex<HashMap<String, (std::time::Instant, bool)>>,
}

impl DeckPlugin {
    pub fn new(platform: Platform, executor: Executor) -> Self {
        Self {
            platform,
            executor,
            core: OnceLock::new(),
            config: JsonStore::default(),
            states: Mutex::default(),
            shell_cache: Mutex::default(),
        }
    }

    async fn push_layout(&self) {
        let Some(core) = self.core.get() else { return };
        let layout = self.config.with(DeckConfig::public_layout);
        for d in core.devices() {
            if d.has_permission(Permission::Deck) {
                let _ = d
                    .send_if_supported(Packet::new(types::DECK, layout.clone()))
                    .await;
            }
        }
    }

    async fn evaluate(&self, src: &StateSource) -> ButtonState {
        match src {
            StateSource::MediaPlaying => ButtonState {
                active: matches!(self.platform.media.status(None).await, Ok(Some(s)) if s.is_playing),
                label: None,
            },
            StateSource::DoNotDisturb => ButtonState {
                active: matches!(self.platform.dnd.get().await, Ok(Some(true))),
                label: None,
            },
            StateSource::Volume => {
                let v = self.platform.media.volume().await.ok().flatten();
                ButtonState {
                    active: v.is_some_and(|v| v > 0),
                    label: v.map(|v| format!("{v}%")),
                }
            }
            StateSource::Shell {
                command,
                interval_secs,
            } => {
                let fresh = self
                    .shell_cache
                    .lock()
                    .unwrap()
                    .get(command)
                    .and_then(|(t, v)| {
                        (t.elapsed() < Duration::from_secs((*interval_secs).max(1))).then_some(*v)
                    });
                let active = match fresh {
                    Some(v) => v,
                    None => {
                        let ok = self
                            .platform
                            .launcher
                            .run_shell(command, Duration::from_secs(10))
                            .await
                            .map(|o| o.code == Some(0))
                            .unwrap_or(false);
                        self.shell_cache
                            .lock()
                            .unwrap()
                            .insert(command.clone(), (std::time::Instant::now(), ok));
                        ok
                    }
                };
                ButtonState {
                    active,
                    label: None,
                }
            }
        }
    }

    /// Recompute states of the active profile; returns changed entries.
    pub async fn refresh_states(&self) -> BTreeMap<String, ButtonState> {
        let buttons: Vec<(String, StateSource)> = self.config.with(|c| {
            c.profiles
                .iter()
                .filter(|p| p.id == c.active_profile)
                .flat_map(|p| p.pages.iter())
                .flat_map(|pg| pg.buttons.iter())
                .filter_map(|b| b.state.clone().map(|s| (b.id.clone(), s)))
                .collect()
        });
        let mut changed = BTreeMap::new();
        for (id, src) in buttons {
            let st = self.evaluate(&src).await;
            let mut states = self.states.lock().unwrap();
            if states.get(&id) != Some(&st) {
                states.insert(id.clone(), st.clone());
                changed.insert(id, st);
            }
        }
        changed
    }

    pub async fn press(&self, profile: &str, page: &str, button: &str) -> Result<Vec<Value>> {
        let b = self
            .config
            .with(|c| c.find_button(profile, page, button).cloned())
            .context("unknown deck button")?;
        let active = self
            .states
            .lock()
            .unwrap()
            .get(&b.id)
            .map(|s| s.active)
            .unwrap_or(false);
        let actions = if active && !b.active_actions.is_empty() {
            &b.active_actions
        } else {
            &b.actions
        };
        self.executor.run(self.core.get(), actions).await
    }
}

#[async_trait]
impl Plugin for DeckPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "deck",
            name: "Mobile Deck",
            description: "Customisable grid of buttons, macros and profiles on the phone",
            permission: Permission::Deck,
            incoming: &[types::DECK_REQUEST],
            outgoing: &[types::DECK],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "deck", "deck.json");
        let _ = self.core.set(core.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            while core.is_alive() {
                tick.tick().await;
                let devs: Vec<_> = core
                    .devices()
                    .into_iter()
                    .filter(|d| d.has_permission(Permission::Deck) && d.supports(types::DECK))
                    .collect();
                if devs.is_empty() {
                    continue;
                }
                let changed = self.refresh_states().await;
                if changed.is_empty() {
                    continue;
                }
                let p = Packet::new(types::DECK, json!({ "type": "state", "states": changed }));
                for d in devs {
                    let _ = d.send(p.clone()).await;
                }
            }
        });
    }

    async fn on_connected(&self, dev: &DeviceHandle) {
        let layout = self.config.with(DeckConfig::public_layout);
        let _ = dev
            .send_if_supported(Packet::new(types::DECK, layout))
            .await;
        let states = self.states.lock().unwrap().clone();
        if !states.is_empty() {
            let _ = dev
                .send_if_supported(Packet::new(
                    types::DECK,
                    json!({ "type": "state", "states": states }),
                ))
                .await;
        }
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        match p.get_str("request").unwrap_or_default() {
            "layout" => {
                let layout = self.config.with(DeckConfig::public_layout);
                dev.send(Packet::new(types::DECK, layout)).await?;
                let states = self.states.lock().unwrap().clone();
                dev.send(Packet::new(
                    types::DECK,
                    json!({ "type": "state", "states": states }),
                ))
                .await?;
            }
            "press" => {
                let (profile, page, button) = (
                    p.get_str("profile").unwrap_or_default(),
                    p.get_str("page").unwrap_or_default(),
                    p.get_str("button").unwrap_or_default(),
                );
                let r = self.press(profile, page, button).await;
                dev.emit("deck", "pressed", json!({ "button": button, "ok": r.is_ok(), "error": r.as_ref().err().map(|e| e.to_string()) }));
                // Reflect toggles immediately.
                let changed = self.refresh_states().await;
                if !changed.is_empty() {
                    dev.send(Packet::new(
                        types::DECK,
                        json!({ "type": "state", "states": changed }),
                    ))
                    .await?;
                }
                r?;
            }
            "setProfile" => {
                let id = p.get_str("profile").unwrap_or_default().to_owned();
                if !self.config.with(|c| c.profiles.iter().any(|x| x.id == id)) {
                    bail!("unknown profile");
                }
                self.config.update(|c| c.active_profile = id);
                self.states.lock().unwrap().clear();
                self.push_layout().await;
            }
            other => tracing::debug!("unknown deck request {other}"),
        }
        Ok(())
    }

    async fn handle_action(
        &self,
        _: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        match action {
            "get" => Ok(serde_json::to_value(self.config.get())?),
            "save" => {
                let c: DeckConfig = crate::args(args)?;
                c.validate()?;
                self.config.update(|v| *v = c);
                self.states.lock().unwrap().clear();
                self.push_layout().await;
                Ok(Value::Null)
            }
            "duplicateProfile" => {
                let src = args
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let new_id = self.config.update(|c| -> Result<String> {
                    let mut p = c
                        .profiles
                        .iter()
                        .find(|p| p.id == src)
                        .cloned()
                        .context("unknown profile")?;
                    p.id = id();
                    p.name = format!("{} (copia)", p.name);
                    for pg in &mut p.pages {
                        for b in &mut pg.buttons {
                            b.id = id();
                        }
                    }
                    let nid = p.id.clone();
                    c.profiles.push(p);
                    Ok(nid)
                })?;
                self.push_layout().await;
                Ok(json!({ "id": new_id }))
            }
            "deleteProfile" => {
                let target = args
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                self.config.update(|c| -> Result<()> {
                    if c.profiles.len() <= 1 {
                        bail!("cannot delete the last profile");
                    }
                    c.profiles.retain(|p| p.id != target);
                    if c.active_profile == target {
                        c.active_profile = c.profiles[0].id.clone();
                    }
                    Ok(())
                })?;
                self.push_layout().await;
                Ok(Value::Null)
            }
            "setActiveProfile" => {
                let target = args
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if !self
                    .config
                    .with(|c| c.profiles.iter().any(|p| p.id == target))
                {
                    bail!("unknown profile");
                }
                self.config.update(|c| c.active_profile = target);
                self.states.lock().unwrap().clear();
                self.push_layout().await;
                Ok(Value::Null)
            }
            "press" => {
                let s = |k: &str| {
                    args.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                Ok(Value::Array(
                    self.press(&s("profile"), &s("page"), &s("button")).await?,
                ))
            }
            "reset" => {
                self.config.update(|c| *c = default_profiles());
                self.push_layout().await;
                Ok(Value::Null)
            }
            _ => bail!("unknown action {action}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandsPlugin;
    use konnekta_platform::mock::MockPlatform;

    fn plugin(mock: &Arc<MockPlatform>) -> DeckPlugin {
        let platform = mock.platform();
        let exec = Executor::new(platform.clone(), CommandsPlugin::new(platform.clone()));
        DeckPlugin::new(platform, exec)
    }

    #[test]
    fn defaults_are_valid_and_public_layout_hides_actions() {
        let c = default_profiles();
        c.validate().unwrap();
        let s = serde_json::to_string(&c.public_layout()).unwrap();
        assert!(!s.contains("actions"));
        assert!(!s.contains("ctrl+c"));
    }

    #[test]
    fn validation_catches_bad_layouts() {
        let mut c = default_profiles();
        c.profiles[0].pages[0].buttons[1].position = 0;
        assert!(c.validate().is_err());
        let mut c = default_profiles();
        c.profiles[0].pages[0].buttons[0].position = 99;
        assert!(c.validate().is_err());
        let mut c = default_profiles();
        c.profiles.clear();
        assert!(c.validate().is_err());
    }

    #[tokio::test]
    async fn press_and_toggle_state() {
        let mock = MockPlatform::new();
        let d = plugin(&mock);
        let c = d.config.get();
        let page = &c.profiles[0].pages[0];
        let next = page
            .buttons
            .iter()
            .find(|b| b.label == "Successivo")
            .unwrap();
        d.press("work", &page.id, &next.id).await.unwrap();
        assert_eq!(mock.calls(), vec!["media:Next"]);
        // Mock media reports playing; volume 0 -> inactive with label.
        let changed = d.refresh_states().await;
        let play = page
            .buttons
            .iter()
            .find(|b| b.state == Some(StateSource::MediaPlaying))
            .unwrap();
        assert!(changed[&play.id].active);
        assert!(
            d.refresh_states().await.is_empty(),
            "unchanged states are not resent"
        );
        assert!(d.press("work", &page.id, "nope").await.is_err());
    }
}
