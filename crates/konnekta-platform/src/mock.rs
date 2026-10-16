//! Test doubles and "unsupported" backends.

use crate::clipboard::ClipboardBackend;
use crate::input::{InputBackend, InputEvent};
use crate::launcher::{CommandOutput, LauncherBackend};
use crate::media::{MediaAction, MediaBackend, MediaStatus};
use crate::notify::{ActionCallback, DesktopNotification, NotifierBackend};
use crate::Platform;
use anyhow::{Result, bail};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Backend for features that do not exist on this platform.
pub struct Unsupported;

#[async_trait]
impl ClipboardBackend for Unsupported {
    async fn get_text(&self) -> Result<Option<String>> {
        Ok(None)
    }
    async fn set_text(&self, _: &str) -> Result<()> {
        bail!("clipboard not supported")
    }
    async fn set_png(&self, _: &[u8]) -> Result<()> {
        bail!("clipboard not supported")
    }
}

impl InputBackend for Unsupported {
    fn send(&self, _: InputEvent) -> Result<()> {
        bail!("input injection not supported")
    }
}

#[async_trait]
impl MediaBackend for Unsupported {
    async fn players(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn status(&self, _: Option<&str>) -> Result<Option<MediaStatus>> {
        Ok(None)
    }
    async fn action(&self, _: Option<&str>, _: MediaAction) -> Result<()> {
        bail!("media control not supported")
    }
    async fn seek(&self, _: Option<&str>, _: i64) -> Result<()> {
        bail!("media control not supported")
    }
    async fn volume(&self) -> Result<Option<u8>> {
        Ok(None)
    }
    async fn set_volume(&self, _: u8) -> Result<()> {
        bail!("volume control not supported")
    }
}

impl NotifierBackend for Unsupported {
    fn show(&self, _: DesktopNotification, _: Option<ActionCallback>) -> Result<()> {
        Ok(())
    }
}

/// Records every call; used by plugin tests.
#[derive(Default)]
pub struct MockPlatform {
    pub log: Mutex<Vec<String>>,
    pub clipboard: Mutex<Option<String>>,
    pub png: Mutex<Option<Vec<u8>>>,
    pub dnd: Mutex<bool>,
    pub volume: Mutex<u8>,
}

impl MockPlatform {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn platform(self: &Arc<Self>) -> Platform {
        Platform {
            clipboard: self.clone(),
            input: self.clone(),
            media: self.clone(),
            notifier: self.clone(),
            dnd: self.clone(),
            launcher: self.clone(),
        }
    }

    fn push(&self, s: String) {
        self.log.lock().unwrap().push(s);
    }

    pub fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

#[async_trait]
impl ClipboardBackend for MockPlatform {
    async fn get_text(&self) -> Result<Option<String>> {
        Ok(self.clipboard.lock().unwrap().clone())
    }
    async fn set_text(&self, t: &str) -> Result<()> {
        self.push(format!("clipboard.set:{t}"));
        *self.clipboard.lock().unwrap() = Some(t.to_owned());
        Ok(())
    }
    async fn set_png(&self, png: &[u8]) -> Result<()> {
        self.push(format!("clipboard.png:{}", png.len()));
        *self.png.lock().unwrap() = Some(png.to_vec());
        Ok(())
    }
}

impl InputBackend for MockPlatform {
    fn send(&self, ev: InputEvent) -> Result<()> {
        self.push(format!("input:{}", serde_json::to_string(&ev).unwrap()));
        Ok(())
    }
}

#[async_trait]
impl MediaBackend for MockPlatform {
    async fn players(&self) -> Result<Vec<String>> {
        Ok(vec!["mock".into()])
    }
    async fn status(&self, _: Option<&str>) -> Result<Option<MediaStatus>> {
        Ok(Some(MediaStatus {
            player: "mock".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            is_playing: true,
            ..Default::default()
        }))
    }
    async fn action(&self, _: Option<&str>, a: MediaAction) -> Result<()> {
        self.push(format!("media:{a:?}"));
        Ok(())
    }
    async fn seek(&self, _: Option<&str>, o: i64) -> Result<()> {
        self.push(format!("seek:{o}"));
        Ok(())
    }
    async fn volume(&self) -> Result<Option<u8>> {
        Ok(Some(*self.volume.lock().unwrap()))
    }
    async fn set_volume(&self, v: u8) -> Result<()> {
        self.push(format!("volume:{v}"));
        *self.volume.lock().unwrap() = v;
        Ok(())
    }
}

impl NotifierBackend for MockPlatform {
    fn show(&self, n: DesktopNotification, _: Option<ActionCallback>) -> Result<()> {
        self.push(format!("notify:{}:{}", n.title, n.body));
        Ok(())
    }
    fn close(&self, id: &str) {
        self.push(format!("notify.close:{id}"));
    }
}

#[async_trait]
impl crate::dnd::DndBackend for MockPlatform {
    async fn get(&self) -> Result<Option<bool>> {
        Ok(Some(*self.dnd.lock().unwrap()))
    }
    async fn set(&self, e: bool) -> Result<()> {
        self.push(format!("dnd:{e}"));
        *self.dnd.lock().unwrap() = e;
        Ok(())
    }
    fn supported(&self) -> bool {
        true
    }
}

#[async_trait]
impl LauncherBackend for MockPlatform {
    async fn run_shell(&self, c: &str, _: Duration) -> Result<CommandOutput> {
        self.push(format!("shell:{c}"));
        Ok(CommandOutput { code: Some(0), stdout: String::new(), stderr: String::new(), timed_out: false })
    }
    async fn open(&self, t: &str) -> Result<()> {
        self.push(format!("open:{t}"));
        Ok(())
    }
    async fn launch_app(&self, a: &str, _: &[String]) -> Result<()> {
        self.push(format!("launch:{a}"));
        Ok(())
    }
}
