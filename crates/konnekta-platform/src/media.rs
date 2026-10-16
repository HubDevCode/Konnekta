//! Media player control.
//!
//! * Linux: MPRIS through `playerctl`, volume through `wpctl` / `pactl`;
//! * macOS: AppleScript for Music and Spotify, system output volume;
//! * Windows: system media keys (works with any app honouring SMTC) and
//!   volume keys.

use crate::input::{InputBackend, InputEvent};
use crate::keys::{Key, Modifiers, SpecialKey};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
        return Arc::new(linux::Playerctl { input });
    }
    #[cfg(target_os = "macos")]
    return Arc::new(macos::AppleScript { input });
    #[allow(unreachable_code)]
    Arc::new(MediaKeys { input })
}

/// Fallback: drive players with media keys (no metadata).
pub struct MediaKeys {
    pub input: Arc<dyn InputBackend>,
}

impl MediaKeys {
    fn key(&self, k: SpecialKey) -> Result<()> {
        self.input.send(InputEvent::Key { key: Key::Special(k), modifiers: Modifiers::default() })
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
            MediaAction::Play | MediaAction::Pause | MediaAction::PlayPause => SpecialKey::MediaPlayPause,
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
            Ok(output("playerctl", &["-l"]).await.unwrap_or_default().lines().map(str::to_owned).collect())
        }

        async fn status(&self, player: Option<&str>) -> Result<Option<MediaStatus>> {
            let fmt = "{{playerName}}\t{{title}}\t{{artist}}\t{{album}}\t{{status}}\t{{position}}\t{{mpris:length}}";
            let Ok(line) = pc(player, &["metadata", "--format", fmt]).await else { return Ok(None) };
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 7 {
                return Ok(None);
            }
            Ok(Some(MediaStatus {
                player: player.unwrap_or(f[0]).to_owned(),
                title: f[1].into(),
                artist: f[2].into(),
                album: f[3].into(),
                is_playing: f[4] == "Playing",
                position_ms: f[5].parse::<i64>().unwrap_or(0) / 1000,
                length_ms: f[6].parse::<i64>().unwrap_or(0) / 1000,
                can_seek: true,
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
                return MediaKeys { input: self.input.clone() }.action(None, a).await;
            }
            Ok(())
        }

        async fn seek(&self, player: Option<&str>, offset_us: i64) -> Result<()> {
            let secs = offset_us as f64 / 1_000_000.0;
            let arg = if secs >= 0.0 { format!("{secs}+") } else { format!("{}-", -secs) };
            pc(player, &["position", &arg]).await.map(drop)
        }

        async fn volume(&self) -> Result<Option<u8>> {
            if let Ok(out) = output("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).await {
                // "Volume: 0.45" or "Volume: 0.45 [MUTED]"
                if let Some(v) = out.split_whitespace().nth(1).and_then(|v| v.parse::<f64>().ok()) {
                    return Ok(Some((v * 100.0).round().clamp(0.0, 100.0) as u8));
                }
            }
            if let Ok(out) = output("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]).await {
                if let Some(p) = out.split('/').nth(1) {
                    return Ok(p.trim().trim_end_matches('%').trim().parse().ok());
                }
            }
            Ok(None)
        }

        async fn set_volume(&self, v: u8) -> Result<()> {
            let v = v.min(100);
            if output("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{}", f64::from(v) / 100.0)])
                .await
                .is_ok()
            {
                return Ok(());
            }
            output("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &format!("{v}%")]).await.map(drop)
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
        osa(&format!("application \"{app}\" is running")).await.map(|s| s == "true").unwrap_or(false)
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
            let Some(app) = pick(player).await else { return Ok(None) };
            let script = format!(
                "tell application \"{app}\" to return (name of current track) & \"\\t\" & (artist of current track) & \"\\t\" & (album of current track) & \"\\t\" & (player state as string) & \"\\t\" & (player position as string) & \"\\t\" & (duration of current track as string)"
            );
            let Ok(out) = osa(&script).await else { return Ok(None) };
            let f: Vec<&str> = out.split('\t').collect();
            if f.len() < 6 {
                return Ok(None);
            }
            let dur: f64 = f[5].replace(',', ".").parse().unwrap_or(0.0);
            // Spotify reports ms, Music reports seconds.
            let length_ms = if app == "Spotify" { dur as i64 } else { (dur * 1000.0) as i64 };
            Ok(Some(MediaStatus {
                player: app,
                title: f[0].into(),
                artist: f[1].into(),
                album: f[2].into(),
                is_playing: f[3] == "playing",
                position_ms: (f[4].replace(',', ".").parse::<f64>().unwrap_or(0.0) * 1000.0) as i64,
                length_ms,
                can_seek: true,
            }))
        }

        async fn action(&self, player: Option<&str>, a: MediaAction) -> Result<()> {
            let Some(app) = pick(player).await else {
                return MediaKeys { input: self.input.clone() }.action(None, a).await;
            };
            let verb = match a {
                MediaAction::Play => "play",
                MediaAction::Pause => "pause",
                MediaAction::PlayPause => "playpause",
                MediaAction::Stop => "pause",
                MediaAction::Next => "next track",
                MediaAction::Previous => "previous track",
            };
            osa(&format!("tell application \"{app}\" to {verb}")).await.map(drop)
        }

        async fn seek(&self, player: Option<&str>, offset_us: i64) -> Result<()> {
            let Some(app) = pick(player).await else { anyhow::bail!("no player") };
            let s = offset_us as f64 / 1_000_000.0;
            osa(&format!("tell application \"{app}\" to set player position to (player position + ({s}))"))
                .await
                .map(drop)
        }

        async fn volume(&self) -> Result<Option<u8>> {
            Ok(osa("output volume of (get volume settings)").await?.parse().ok())
        }

        async fn set_volume(&self, v: u8) -> Result<()> {
            osa(&format!("set volume output volume {}", v.min(100))).await.map(drop)
        }
    }
}
