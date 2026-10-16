//! Konnekta plugins.
//!
//! *KDE Connect compatible* (standard `kdeconnect.*` packets): ping, battery,
//! clipboard, share, mpris, mousepad, presenter, runcommand, notification,
//! telephony, findmyphone.
//!
//! *Konnekta extensions* (`konnekta.*` packets, only with Konnekta peers):
//! deck, dnd, screenshot, microphone, camera, desktop, systemaudio,
//! phonescreen.

pub mod actions;
pub mod battery;
pub mod bridge;
pub mod clipboard;
pub mod deck;
pub mod dnd;
pub mod findmyphone;
pub mod mousepad;
pub mod mpris;
pub mod notifications;
pub mod ping;
pub mod presenter;
pub mod runcommand;
pub mod screenshot;
pub mod share;
pub mod store;
pub mod streams;
pub mod telephony;

use konnekta_core::Plugin;
use konnekta_platform::Platform;
use std::sync::Arc;

/// Every built-in plugin, wired to the given platform backends.
pub fn all(platform: Platform) -> Vec<Arc<dyn Plugin>> {
    let commands = runcommand::RunCommandPlugin::new(platform.clone());
    let executor = actions::Executor::new(platform.clone(), commands.clone());
    vec![
        Arc::new(ping::PingPlugin::new(platform.clone())),
        Arc::new(battery::BatteryPlugin::default()),
        Arc::new(clipboard::ClipboardPlugin::new(platform.clone())),
        share::SharePlugin::new(platform.clone()),
        Arc::new(mpris::MprisPlugin::new(platform.clone())),
        Arc::new(mousepad::MousepadPlugin::new(platform.clone())),
        Arc::new(presenter::PresenterPlugin::new(platform.clone())),
        commands,
        Arc::new(notifications::NotificationsPlugin::new(platform.clone())),
        Arc::new(telephony::TelephonyPlugin::new(platform.clone())),
        Arc::new(findmyphone::FindMyPhonePlugin),
        Arc::new(deck::DeckPlugin::new(platform.clone(), executor)),
        Arc::new(dnd::DndPlugin::new(platform.clone())),
        Arc::new(screenshot::ScreenshotPlugin::new(platform.clone())),
        streams::StreamPlugin::microphone(),
        streams::StreamPlugin::camera(),
        streams::StreamPlugin::phone_screen(),
        streams::StreamPlugin::desktop(),
        streams::StreamPlugin::system_audio(),
    ]
}

/// Parse UI action arguments.
pub(crate) fn args<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> anyhow::Result<T> {
    serde_json::from_value(v).map_err(|e| anyhow::anyhow!("invalid arguments: {e}"))
}

pub(crate) fn need_dev(
    dev: Option<&konnekta_core::DeviceHandle>,
) -> anyhow::Result<&konnekta_core::DeviceHandle> {
    dev.ok_or_else(|| anyhow::anyhow!("this action needs a device"))
}
