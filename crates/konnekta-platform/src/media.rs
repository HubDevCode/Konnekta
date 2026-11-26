//! Media player control.
//!
//! * Linux: MPRIS through `playerctl`, volume through `wpctl` / `pactl`;
//! * macOS: AppleScript for Music and Spotify, system output volume;
//! * Windows: system media keys (works with any app honouring SMTC) and
//!   volume keys.

use crate::input::{InputBackend, InputEvent};
use crate::keys::{Key, Modifiers, SpecialKey};
use crate::watch::LineWatcher;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaAction {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
}

impl MediaAction {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "Play" => Self::Play,
            "Pause" => Self::Pause,
            "PlayPause" => Self::PlayPause,
            "Stop" => Self::Stop,
            "Next" => Self::Next,
            "Previous" => Self::Previous,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaStatus {
    pub player: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub is_playing: bool,
    pub position_ms: i64,
    pub length_ms: i64,
    pub can_seek: bool,
    /// Cover picture (`file://` or `https://`), empty when the player has none.
    #[serde(default)]
    pub art_url: String,
}

#[async_trait]
pub trait MediaBackend: Send + Sync {
    async fn players(&self) -> Result<Vec<String>>;
    async fn status(&self, player: Option<&str>) -> Result<Option<MediaStatus>>;
    async fn action(&self, player: Option<&str>, action: MediaAction) -> Result<()>;
    /// Seek relative, microseconds.
    async fn seek(&self, player: Option<&str>, offset_us: i64) -> Result<()>;
    /// System output volume 0..=100.
    async fn volume(&self) -> Result<Option<u8>>;
    async fn set_volume(&self, v: u8) -> Result<()>;
}

#[cfg(feature = "desktop")]
pub fn native(input: Arc<dyn InputBackend>) -> Arc<dyn MediaBackend> {
    #[cfg(target_os = "linux")]
    if crate::cmd::exists("playerctl") {
        return Arc::new(Cached::new(
            Arc::new(linux::Playerctl { input }),
            Some(&crate::watch::PLAYERS),
            Some(&crate::watch::PULSE),
        ));
    }
    #[cfg(target_os = "macos")]
    return Arc::new(Cached::new(
        Arc::new(macos::AppleScript { input }),
        None,
        None,
    ));
    #[allow(unreachable_code)]
    Arc::new(MediaKeys { input })
}

/// Longest a status is reused while a watcher reports no change: playing,
/// the position moves (a seek in the player itself sends no line); paused,
/// nothing moves.
const PLAYING_MAX_AGE: Duration = Duration::from_secs(10);
const IDLE_MAX_AGE: Duration = Duration::from_secs(60);
/// Without a watcher: answers are shared by everyone asking within this
/// time (the media plugin and the Deck buttons ask about the same second).
const UNWATCHED_MAX_AGE: Duration = Duration::from_secs(1);

type Stamp = (Option<u64>, u64, Instant);
type StatusCache = HashMap<Option<String>, (Stamp, Option<MediaStatus>)>;

/// Reads the player and the volume again only when they may have changed:
/// a watcher line ([`crate::watch`]), an action sent from here, or the
/// maximum age. Each read of the Linux backend starts a program.
pub struct Cached {
    inner: Arc<dyn MediaBackend>,
    players: Option<&'static LineWatcher>,
    volume: Option<&'static LineWatcher>,
    /// Bumped by every action sent through this backend.
    local: AtomicU64,
    status: Mutex<StatusCache>,
    vol: Mutex<Option<(Stamp, Option<u8>)>>,
}

impl Cached {
    pub fn new(
        inner: Arc<dyn MediaBackend>,
        players: Option<&'static LineWatcher>,
        volume: Option<&'static LineWatcher>,
    ) -> Self {
        Self {
            inner,
            players,
            volume,
            local: AtomicU64::new(0),
            status: Mutex::default(),
            vol: Mutex::default(),
        }
    }

    fn stamp(&self, w: Option<&'static LineWatcher>) -> Stamp {
        (
            w.and_then(LineWatcher::generation),
            self.local.load(Ordering::SeqCst),
            Instant::now(),
        )
    }

    fn fresh(old: &Stamp, now: &Stamp, max_age: Duration) -> bool {
        let max_age = if now.0.is_some() {
            max_age
        } else {
            UNWATCHED_MAX_AGE
        };
        old.0 == now.0 && old.1 == now.1 && now.2.duration_since(old.2) < max_age
    }

    fn changed_here(&self) {
        self.local.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl MediaBackend for Cached {
    async fn players(&self) -> Result<Vec<String>> {
        self.inner.players().await
    }

    async fn status(&self, player: Option<&str>) -> Result<Option<MediaStatus>> {
        let key = player.filter(|p| !p.is_empty()).map(str::to_owned);
        // Taken before reading: a change during the read makes it stale.
        let now = self.stamp(self.players);
        if let Some((at, s)) = self.status.lock().unwrap().get(&key) {
            let age = if s.as_ref().is_some_and(|s| s.is_playing) {
                PLAYING_MAX_AGE
            } else {
                IDLE_MAX_AGE
            };
            if Self::fresh(at, &now, age) {
                // Playing, the position moved on since it was read.
                let mut s = s.clone();
                if let Some(s) = s.as_mut().filter(|s| s.is_playing) {
                    let moved = now.2.duration_since(at.2).as_millis() as i64;
                    s.position_ms += moved;
                    if s.length_ms > 0 {
                        s.position_ms = s.position_ms.min(s.length_ms);
                    }
                }
                return Ok(s);
            }
        }
        let s = self.inner.status(player).await?;
        let mut m = self.status.lock().unwrap();
        // Players come and go: keep the table small.
        if m.len() > 16 {
            m.clear();
        }
        m.insert(key, (now, s.clone()));
        Ok(s)
    }

    async fn action(&self, player: Option<&str>, action: MediaAction) -> Result<()> {
        self.changed_here();
        let r = self.inner.action(player, action).await;
        self.changed_here();
        r
    }

    async fn seek(&self, player: Option<&str>, offset_us: i64) -> Result<()> {
        self.changed_here();
        let r = self.inner.seek(player, offset_us).await;
        self.changed_here();
        r
    }

    async fn volume(&self) -> Result<Option<u8>> {
        let now = self.stamp(self.volume);
        if let Some((at, v)) = *self.vol.lock().unwrap()
            && Self::fresh(&at, &now, IDLE_MAX_AGE)
        {
            return Ok(v);
        }
        let v = self.inner.volume().await?;
        *self.vol.lock().unwrap() = Some((now, v));
        Ok(v)
    }

    async fn set_volume(&self, v: u8) -> Result<()> {
        self.changed_here();
        let r = self.inner.set_volume(v).await;
        self.changed_here();
        r
    }
}

/// Fallback: drive players with media keys (no metadata).
pub struct MediaKeys {
    pub input: Arc<dyn InputBackend>,
}

impl MediaKeys {
    fn key(&self, k: SpecialKey) -> Result<()> {
        self.input.send(InputEvent::Key {
            key: Key::Special(k),
            modifiers: Modifiers::default(),
        })
    }
}

#[async_trait]
impl MediaBackend for MediaKeys {
    async fn players(&self) -> Result<Vec<String>> {
        Ok(vec!["System".into()])
    }
    async fn status(&self, _: Option<&str>) -> Result<Option<MediaStatus>> {
        Ok(None)
    }
    async fn action(&self, _: Option<&str>, a: MediaAction) -> Result<()> {
        self.key(match a {
            MediaAction::Play | MediaAction::Pause | MediaAction::PlayPause => {
                SpecialKey::MediaPlayPause
            }
            MediaAction::Stop => SpecialKey::MediaStop,
            MediaAction::Next => SpecialKey::MediaNext,
            MediaAction::Previous => SpecialKey::MediaPrevious,
        })
    }
    async fn seek(&self, _: Option<&str>, _: i64) -> Result<()> {
        anyhow::bail!("seeking not supported on this platform")
    }
    async fn volume(&self) -> Result<Option<u8>> {
        Ok(None)
    }
    async fn set_volume(&self, v: u8) -> Result<()> {
        // No absolute volume API without native bindings: approximate by
        // stepping from zero (each key press is 2%).
        for _ in 0..50 {
            self.key(SpecialKey::VolumeDown)?;
        }
        for _ in 0..(v / 2) {
            self.key(SpecialKey::VolumeUp)?;
        }
        Ok(())
    }
}

#[cfg(all(feature = "desktop", target_os = "linux"))]
mod linux {
    use super::*;
    use crate::cmd::output;

    pub struct Playerctl {
        pub input: Arc<dyn InputBackend>,
    }

    fn player_args(player: Option<&str>) -> Vec<String> {
        match player {
            Some(p) if !p.is_empty() => vec!["-p".into(), p.into()],
            _ => vec![],
        }
    }

    async fn pc(player: Option<&str>, args: &[&str]) -> Result<String> {
        let mut a = player_args(player);
        a.extend(args.iter().map(|s| s.to_string()));
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        output("playerctl", &a).await
    }

    #[async_trait]
    impl MediaBackend for Playerctl {
        async fn players(&self) -> Result<Vec<String>> {
            Ok(output("playerctl", &["-l"])
                .await
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect())
        }

        async fn status(&self, player: Option<&str>) -> Result<Option<MediaStatus>> {
            let fmt = "{{playerName}}\t{{title}}\t{{artist}}\t{{album}}\t{{status}}\t{{position}}\t{{mpris:length}}\t{{mpris:artUrl}}";
            let named = player.is_some_and(|p| !p.is_empty());
            // No player named: look at all of them and take the one playing.
            let out = if named {
                pc(player, &["metadata", "--format", fmt]).await
            } else {
                output("playerctl", &["-a", "metadata", "--format", fmt]).await
            };
            let Ok(out) = out else {
                return Ok(None);
            };
            let rows: Vec<Vec<&str>> = out
                .lines()
                .map(|l| l.split('\t').collect::<Vec<_>>())
                .filter(|f| f.len() >= 7)
                .collect();
            let Some(f) = rows
                .iter()
                .find(|f| f[4] == "Playing")
                .or_else(|| rows.first())
            else {
                return Ok(None);
            };
            Ok(Some(MediaStatus {
                player: if named {
                    player.unwrap_or_default()
                } else {
                    f[0]
                }
                .to_owned(),
                title: f[1].into(),
                artist: f[2].into(),
                album: f[3].into(),
                is_playing: f[4] == "Playing",
                position_ms: f[5].parse::<i64>().unwrap_or(0) / 1000,
                length_ms: f[6].parse::<i64>().unwrap_or(0) / 1000,
                can_seek: true,
                art_url: f.get(7).copied().unwrap_or_default().to_owned(),
            }))
        }

        async fn action(&self, player: Option<&str>, a: MediaAction) -> Result<()> {
            let cmd = match a {
                MediaAction::Play => "play",
                MediaAction::Pause => "pause",
                MediaAction::PlayPause => "play-pause",
                MediaAction::Stop => "stop",
                MediaAction::Next => "next",
                MediaAction::Previous => "previous",
            };
            if pc(player, &[cmd]).await.is_err() {
                // No MPRIS player: fall back to global media keys.
                return MediaKeys {
                    input: self.input.clone(),
                }
                .action(None, a)
                .await;
            }
            Ok(())
        }

        async fn seek(&self, player: Option<&str>, offset_us: i64) -> Result<()> {
            let secs = offset_us as f64 / 1_000_000.0;
            let arg = if secs >= 0.0 {
                format!("{secs}+")
            } else {
                format!("{}-", -secs)
            };
            pc(player, &["position", &arg]).await.map(drop)
        }

        async fn volume(&self) -> Result<Option<u8>> {
            if let Ok(out) = output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).await {
                // "Volume: 0.45" or "Volume: 0.45 [MUTED]"
                if let Some(v) = out
                    .split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<f64>().ok())
                {
                    return Ok(Some((v * 100.0).round().clamp(0.0, 100.0) as u8));
                }
            }
            if let Ok(out) = output("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]).await
                && let Some(p) = out.split('/').nth(1)
            {
                return Ok(p.trim().trim_end_matches('%').trim().parse().ok());
            }
            Ok(None)
        }

        async fn set_volume(&self, v: u8) -> Result<()> {
            let v = v.min(100);
            if output(
                "wpctl",
                &[
                    "set-volume",
                    "@DEFAULT_AUDIO_SINK@",
                    &format!("{}", f64::from(v) / 100.0),
                ],
            )
            .await
            .is_ok()
            {
                return Ok(());
            }
            output(
                "pactl",
                &["set-sink-volume", "@DEFAULT_SINK@", &format!("{v}%")],
            )
            .await
            .map(drop)
        }
    }
}

#[cfg(all(feature = "desktop", target_os = "macos"))]
mod macos {
    use super::*;
    use crate::cmd::output;

    pub struct AppleScript {
        pub input: Arc<dyn InputBackend>,
    }

    const APPS: &[&str] = &["Spotify", "Music"];

    async fn osa(script: &str) -> Result<String> {
        output("osascript", &["-e", script]).await
    }

    async fn running(app: &str) -> bool {
        osa(&format!("application \"{app}\" is running"))
            .await
            .map(|s| s == "true")
            .unwrap_or(false)
    }

    async fn pick(player: Option<&str>) -> Option<String> {
        if let Some(p) = player.filter(|p| APPS.contains(p)) {
            return Some(p.to_owned());
        }
        for a in APPS {
            if running(a).await {
                return Some((*a).to_owned());
            }
        }
        None
    }

    #[async_trait]
    impl MediaBackend for AppleScript {
        async fn players(&self) -> Result<Vec<String>> {
            let mut v = Vec::new();
            for a in APPS {
                if running(a).await {
                    v.push((*a).to_owned());
                }
            }
            Ok(v)
        }

        async fn status(&self, player: Option<&str>) -> Result<Option<MediaStatus>> {
            let Some(app) = pick(player).await else {
                return Ok(None);
            };
            let script = format!(
                "tell application \"{app}\" to return (name of current track) & \"\\t\" & (artist of current track) & \"\\t\" & (album of current track) & \"\\t\" & (player state as string) & \"\\t\" & (player position as string) & \"\\t\" & (duration of current track as string)"
            );
            let Ok(out) = osa(&script).await else {
                return Ok(None);
            };
            let f: Vec<&str> = out.split('\t').collect();
            if f.len() < 6 {
                return Ok(None);
            }
            let dur: f64 = f[5].replace(',', ".").parse().unwrap_or(0.0);
            // Spotify reports ms, Music reports seconds.
            let length_ms = if app == "Spotify" {
                dur as i64
            } else {
                (dur * 1000.0) as i64
            };
            Ok(Some(MediaStatus {
                player: app,
                title: f[0].into(),
                artist: f[1].into(),
                album: f[2].into(),
                is_playing: f[3] == "playing",
                position_ms: (f[4].replace(',', ".").parse::<f64>().unwrap_or(0.0) * 1000.0) as i64,
                length_ms,
                can_seek: true,
                art_url: String::new(),
            }))
        }

        async fn action(&self, player: Option<&str>, a: MediaAction) -> Result<()> {
            let Some(app) = pick(player).await else {
                return MediaKeys {
                    input: self.input.clone(),
                }
                .action(None, a)
                .await;
            };
            let verb = match a {
                MediaAction::Play => "play",
                MediaAction::Pause => "pause",
                MediaAction::PlayPause => "playpause",
                MediaAction::Stop => "pause",
                MediaAction::Next => "next track",
                MediaAction::Previous => "previous track",
            };
            osa(&format!("tell application \"{app}\" to {verb}"))
                .await
                .map(drop)
        }

        async fn seek(&self, player: Option<&str>, offset_us: i64) -> Result<()> {
            let Some(app) = pick(player).await else {
                anyhow::bail!("no player")
            };
            let s = offset_us as f64 / 1_000_000.0;
            osa(&format!(
                "tell application \"{app}\" to set player position to (player position + ({s}))"
            ))
            .await
            .map(drop)
        }

        async fn volume(&self) -> Result<Option<u8>> {
            Ok(osa("output volume of (get volume settings)")
                .await?
                .parse()
                .ok())
        }

        async fn set_volume(&self, v: u8) -> Result<()> {
            osa(&format!("set volume output volume {}", v.min(100)))
                .await
                .map(drop)
        }
    }
}
