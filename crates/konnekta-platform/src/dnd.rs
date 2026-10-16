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

#[cfg(target_os = "linux")]
#[async_trait]
impl DndBackend for Gnome {
    async fn get(&self) -> Result<Option<bool>> {
        let v = crate::cmd::output(
            "gsettings",
            &["get", "org.gnome.desktop.notifications", "show-banners"],
        )
        .await?;
        Ok(Some(v.trim() == "false"))
    }

    async fn set(&self, enabled: bool) -> Result<()> {
        let v = if enabled { "false" } else { "true" };
        crate::cmd::output(
            "gsettings",
            &["set", "org.gnome.desktop.notifications", "show-banners", v],
        )
        .await
        .map(drop)
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
