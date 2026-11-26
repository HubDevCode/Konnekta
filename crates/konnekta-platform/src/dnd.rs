//! System "Do Not Disturb" state.
//!
//! Only GNOME exposes a scriptable switch (`show-banners`). KDE Plasma,
//! Windows Focus Assist and macOS Focus have no stable public API, so the
//! backend reports `unsupported` there and the plugin degrades to one-way
//! sync (desktop follows phone where possible, otherwise just shows state).

use anyhow::{Result, bail};
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait DndBackend: Send + Sync {
    async fn get(&self) -> Result<Option<bool>>;
    async fn set(&self, enabled: bool) -> Result<()>;
    fn supported(&self) -> bool;
}

pub fn native() -> Arc<dyn DndBackend> {
    #[cfg(target_os = "linux")]
    if crate::cmd::exists("gsettings")
        && std::env::var("XDG_CURRENT_DESKTOP")
            .map(|d| d.contains("GNOME"))
            .unwrap_or(false)
    {
        return Arc::new(Gnome);
    }
    Arc::new(crate::mock::Unsupported)
}

#[cfg(target_os = "linux")]
pub struct Gnome;

/// `gsettings monitor` prints a line at each change of the switch: the
/// value is read again only then (or after a while), not at every ask.
#[cfg(target_os = "linux")]
static BANNERS: std::sync::LazyLock<crate::watch::LineWatcher> = std::sync::LazyLock::new(|| {
    crate::watch::LineWatcher::new(
        "gsettings",
        &["monitor", "org.gnome.desktop.notifications", "show-banners"],
        |_| true,
    )
});

/// Last value read, with the watcher generation it was read at.
#[cfg(target_os = "linux")]
static LAST: std::sync::Mutex<Option<(Option<u64>, std::time::Instant, bool)>> =
    std::sync::Mutex::new(None);

#[cfg(target_os = "linux")]
#[async_trait]
impl DndBackend for Gnome {
    async fn get(&self) -> Result<Option<bool>> {
        let generation = BANNERS.generation();
        let max_age = std::time::Duration::from_secs(if generation.is_some() { 300 } else { 1 });
        if let Some((g, at, v)) = *LAST.lock().unwrap()
            && g == generation
            && at.elapsed() < max_age
        {
            return Ok(Some(v));
        }
        let v = crate::cmd::output(
            "gsettings",
            &["get", "org.gnome.desktop.notifications", "show-banners"],
        )
        .await?;
        let v = v.trim() == "false";
        *LAST.lock().unwrap() = Some((generation, std::time::Instant::now(), v));
        Ok(Some(v))
    }

    async fn set(&self, enabled: bool) -> Result<()> {
        let v = if enabled { "false" } else { "true" };
        let r = crate::cmd::output(
            "gsettings",
            &["set", "org.gnome.desktop.notifications", "show-banners", v],
        )
        .await
        .map(drop);
        *LAST.lock().unwrap() = None;
        r
    }

    fn supported(&self) -> bool {
        true
    }
}

#[async_trait]
impl DndBackend for crate::mock::Unsupported {
    async fn get(&self) -> Result<Option<bool>> {
        Ok(None)
    }
    async fn set(&self, _: bool) -> Result<()> {
        bail!("Do Not Disturb control is not supported on this desktop")
    }
    fn supported(&self) -> bool {
        false
    }
}
