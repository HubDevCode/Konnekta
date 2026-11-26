//! Audio mixer: volume and mute of every output, input and playing app,
//! default devices, and which output an app plays on.
//!
//! * Linux: `pactl` (PulseAudio and PipeWire through pipewire-pulse);
//! * other systems: not supported yet.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MixerKind {
    Output,
    Input,
    App,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MixerChannel {
    pub id: u32,
    /// Display name: device description or application name.
    pub label: String,
    /// What the app is playing (stream title), for apps only.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Volume in percent; above 100 is software amplification.
    pub volume: u16,
    pub muted: bool,
    /// Default output / input of the system.
    #[serde(default)]
    pub is_default: bool,
    /// For apps: the output they play on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MixerState {
    pub outputs: Vec<MixerChannel>,
    pub inputs: Vec<MixerChannel>,
    pub apps: Vec<MixerChannel>,
}

/// Highest volume accepted, in percent.
pub const MAX_VOLUME: u16 = 150;

#[async_trait]
pub trait MixerBackend: Send + Sync {
    /// Current state, `None` where there is no mixer.
    async fn state(&self) -> Result<Option<MixerState>>;
    async fn set_volume(&self, kind: MixerKind, id: u32, volume: u16) -> Result<()>;
    async fn set_mute(&self, kind: MixerKind, id: u32, muted: bool) -> Result<()>;
    /// Make an output or input the system default.
    async fn set_default(&self, kind: MixerKind, id: u32) -> Result<()>;
    /// Move a playing app to another output.
    async fn move_app(&self, app: u32, output: u32) -> Result<()>;
}

/// Longest the state is reused while the watcher reports no change.
const WATCHED_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30);
/// Without a watcher: one read shared by the callers of the same second.
const UNWATCHED_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(1);

/// Reads the mixer again only after a change (`pactl subscribe`), a change
/// made from here or the maximum age: a read starts five `pactl` programs.
pub struct Cached {
    inner: std::sync::Arc<dyn MixerBackend>,
    watcher: Option<&'static crate::watch::LineWatcher>,
    local: std::sync::atomic::AtomicU64,
    #[allow(clippy::type_complexity)]
    last: std::sync::Mutex<Option<((Option<u64>, u64, std::time::Instant), Option<MixerState>)>>,
}

impl Cached {
    pub fn new(
        inner: std::sync::Arc<dyn MixerBackend>,
        watcher: Option<&'static crate::watch::LineWatcher>,
    ) -> Self {
        Self {
            inner,
            watcher,
            local: Default::default(),
            last: Default::default(),
        }
    }

    fn changed_here(&self) {
        self.local
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    async fn around<T>(&self, f: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        self.changed_here();
        let r = f.await;
        self.changed_here();
        r
    }
}

#[async_trait]
impl MixerBackend for Cached {
    async fn state(&self) -> Result<Option<MixerState>> {
        let now = (
            self.watcher.and_then(crate::watch::LineWatcher::generation),
            self.local.load(std::sync::atomic::Ordering::SeqCst),
            std::time::Instant::now(),
        );
        let max_age = if now.0.is_some() {
            WATCHED_MAX_AGE
        } else {
            UNWATCHED_MAX_AGE
        };
        if let Some((at, s)) = self.last.lock().unwrap().as_ref()
            && at.0 == now.0
            && at.1 == now.1
            && now.2.duration_since(at.2) < max_age
        {
            return Ok(s.clone());
        }
        let s = self.inner.state().await?;
        *self.last.lock().unwrap() = Some((now, s.clone()));
        Ok(s)
    }
    async fn set_volume(&self, kind: MixerKind, id: u32, volume: u16) -> Result<()> {
        self.around(self.inner.set_volume(kind, id, volume)).await
    }
    async fn set_mute(&self, kind: MixerKind, id: u32, muted: bool) -> Result<()> {
        self.around(self.inner.set_mute(kind, id, muted)).await
    }
    async fn set_default(&self, kind: MixerKind, id: u32) -> Result<()> {
        self.around(self.inner.set_default(kind, id)).await
    }
    async fn move_app(&self, app: u32, output: u32) -> Result<()> {
        self.around(self.inner.move_app(app, output)).await
    }
}

#[cfg(feature = "desktop")]
pub fn native() -> std::sync::Arc<dyn MixerBackend> {
    use std::sync::Arc;
    #[cfg(target_os = "linux")]
    if crate::cmd::exists("pactl") {
        return Arc::new(Cached::new(
            Arc::new(linux::Pactl),
            Some(&crate::watch::PULSE),
        ));
    }
    Arc::new(crate::mock::Unsupported)
}

#[cfg(target_os = "linux")]
pub mod linux {
    use super::*;
    use crate::cmd::output;
    use anyhow::{Context, bail};
    use serde_json::Value;

    pub struct Pactl;

    /// Average of the channel volumes, in percent.
    fn volume(v: &Value) -> u16 {
        let raw: Vec<f64> = v
            .get("volume")
            .and_then(Value::as_object)
            .map(|m| {
                m.values()
                    .filter_map(|c| c.get("value").and_then(Value::as_f64))
                    .collect()
            })
            .unwrap_or_default();
        if raw.is_empty() {
            return 0;
        }
        let avg = raw.iter().sum::<f64>() / raw.len() as f64;
        (avg / 65536.0 * 100.0).round() as u16
    }

    fn prop<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
        v.get("properties")?.get(key)?.as_str()
    }

    fn device(v: &Value, default: &str) -> Option<MixerChannel> {
        let name = v.get("name")?.as_str()?;
        Some(MixerChannel {
            id: v.get("index")?.as_u64()? as u32,
            label: v
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or(name)
                .to_owned(),
            detail: String::new(),
            volume: volume(v),
            muted: v.get("mute").and_then(Value::as_bool).unwrap_or(false),
            is_default: name == default,
            output: None,
            icon: prop(v, "device.icon_name").map(str::to_owned),
        })
    }

    fn app(v: &Value) -> Option<MixerChannel> {
        let label = prop(v, "application.name")
            .or_else(|| prop(v, "application.process.binary"))
            .or_else(|| prop(v, "media.name"))?;
        Some(MixerChannel {
            id: v.get("index")?.as_u64()? as u32,
            label: label.to_owned(),
            detail: prop(v, "media.name")
                .filter(|m| *m != label)
                .unwrap_or_default()
                .to_owned(),
            volume: volume(v),
            muted: v.get("mute").and_then(Value::as_bool).unwrap_or(false),
            is_default: false,
            output: v.get("sink").and_then(Value::as_u64).map(|s| s as u32),
            icon: prop(v, "application.icon_name").map(str::to_owned),
        })
    }

    async fn list(what: &str) -> Result<Vec<Value>> {
        let out = output("pactl", &["-f", "json", "list", what]).await?;
        serde_json::from_str(&out).with_context(|| format!("pactl list {what}: bad JSON"))
    }

    fn noun(kind: MixerKind) -> &'static str {
        match kind {
            MixerKind::Output => "sink",
            MixerKind::Input => "source",
            MixerKind::App => "sink-input",
        }
    }

    impl Pactl {
        async fn name_of(&self, kind: MixerKind, id: u32) -> Result<String> {
            let what = match kind {
                MixerKind::Output => "sinks",
                MixerKind::Input => "sources",
                MixerKind::App => bail!("an app cannot be a default device"),
            };
            list(what)
                .await?
                .iter()
                .find(|v| v.get("index").and_then(Value::as_u64) == Some(id.into()))
                .and_then(|v| v.get("name")?.as_str().map(str::to_owned))
                .with_context(|| format!("no {} with id {id}", noun(kind)))
        }
    }

    #[async_trait]
    impl MixerBackend for Pactl {
        async fn state(&self) -> Result<Option<MixerState>> {
            let (sinks, sources, inputs, def_sink, def_source) = tokio::try_join!(
                list("sinks"),
                list("sources"),
                list("sink-inputs"),
                output("pactl", &["get-default-sink"]),
                output("pactl", &["get-default-source"]),
            )?;
            Ok(Some(MixerState {
                outputs: sinks.iter().filter_map(|v| device(v, &def_sink)).collect(),
                // Monitors of outputs are not real inputs.
                inputs: sources
                    .iter()
                    .filter(|v| prop(v, "device.class") != Some("monitor"))
                    .filter_map(|v| device(v, &def_source))
                    .collect(),
                apps: inputs.iter().filter_map(app).collect(),
            }))
        }

        async fn set_volume(&self, kind: MixerKind, id: u32, volume: u16) -> Result<()> {
            let cmd = format!("set-{}-volume", noun(kind));
            let v = format!("{}%", volume.min(MAX_VOLUME));
            output("pactl", &[&cmd, &id.to_string(), &v])
                .await
                .map(drop)
        }

        async fn set_mute(&self, kind: MixerKind, id: u32, muted: bool) -> Result<()> {
            let cmd = format!("set-{}-mute", noun(kind));
            let m = if muted { "1" } else { "0" };
            output("pactl", &[&cmd, &id.to_string(), m]).await.map(drop)
        }

        async fn set_default(&self, kind: MixerKind, id: u32) -> Result<()> {
            let name = self.name_of(kind, id).await?;
            let cmd = format!("set-default-{}", noun(kind));
            output("pactl", &[&cmd, &name]).await.map(drop)
        }

        async fn move_app(&self, app: u32, output_id: u32) -> Result<()> {
            output(
                "pactl",
                &["move-sink-input", &app.to_string(), &output_id.to_string()],
            )
            .await
            .map(drop)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_devices_and_apps() {
            let sink: Value = serde_json::json!({
                "index": 62, "name": "alsa_output.x", "description": "Speakers",
                "mute": false,
                "volume": {"front-left": {"value": 52428}, "front-right": {"value": 52428}},
                "properties": {"device.icon_name": "audio-card"}
            });
            let d = device(&sink, "alsa_output.x").unwrap();
            assert_eq!((d.id, d.volume, d.is_default), (62, 80, true));
            assert_eq!(d.label, "Speakers");

            let input: Value = serde_json::json!({
                "index": 90, "sink": 62, "mute": true,
                "volume": {"mono": {"value": 65536}},
                "properties": {"application.name": "Firefox", "media.name": "Video"}
            });
            let a = app(&input).unwrap();
            assert_eq!(
                (a.id, a.volume, a.muted, a.output),
                (90, 100, true, Some(62))
            );
            assert_eq!((a.label.as_str(), a.detail.as_str()), ("Firefox", "Video"));
        }

        /// Reads the real mixer: `cargo test -p konnekta-platform -- --ignored live`.
        #[tokio::test]
        #[ignore]
        async fn live_state() {
            let s = Pactl.state().await.unwrap().unwrap();
            println!("{}", serde_json::to_string_pretty(&s).unwrap());
            assert!(!s.outputs.is_empty());
        }
    }
}
