//! Konnekta plugins: one independent plugin per feature, each speaking only
//! `konnekta.*` packets. Which of them also work through the optional KDE
//! Connect and Flux bridges follows from their packet types
//! (`PluginInfo::interop`).

pub mod actions;
pub mod approve;
pub mod appupdate;
pub mod battery;
pub mod bridge;
pub mod browse;
pub mod calls;
pub mod chat;
pub mod clipboard;
pub mod commands;
pub mod deck;
pub mod dnd;
pub mod finddevice;
pub mod hyprland;
pub mod input;
pub mod localsend;
pub mod media;
pub mod mixer;
pub mod notifications;
pub mod photo;
pub mod ping;
pub mod presenter;
pub mod screenshot;
pub mod share;
pub mod sms;
pub mod store;
pub mod streams;

use konnekta_core::Plugin;
use konnekta_platform::Platform;
use std::sync::Arc;

/// Every built-in plugin, wired to the given platform backends.
pub fn all(platform: Platform) -> Vec<Arc<dyn Plugin>> {
    let commands = commands::CommandsPlugin::new(platform.clone());
    let executor = actions::Executor::new(platform.clone(), commands.clone());
    vec![
        Arc::new(ping::PingPlugin::new(platform.clone())),
        Arc::new(battery::BatteryPlugin::default()),
        Arc::new(clipboard::ClipboardPlugin::new(platform.clone())),
        share::SharePlugin::new(platform.clone()),
        Arc::new(media::MediaPlugin::new(platform.clone())),
        Arc::new(mixer::MixerPlugin::new(platform.clone())),
        Arc::new(input::InputPlugin::new(platform.clone())),
        Arc::new(presenter::PresenterPlugin::new(platform.clone())),
        commands,
        Arc::new(notifications::NotificationsPlugin::new(platform.clone())),
        Arc::new(calls::CallsPlugin::new(platform.clone())),
        Arc::new(sms::SmsPlugin::default()),
        Arc::new(photo::PhotoPlugin::new(platform.clone())),
        Arc::new(appupdate::AppUpdatePlugin),
        Arc::new(approve::ApprovePlugin::default()),
        Arc::new(hyprland::HyprlandPlugin),
        Arc::new(finddevice::FindDevicePlugin),
        Arc::new(deck::DeckPlugin::new(platform.clone(), executor)),
        Arc::new(dnd::DndPlugin::new(platform.clone())),
        Arc::new(screenshot::ScreenshotPlugin::new(platform.clone())),
        Arc::new(chat::ChatPlugin::default()),
        Arc::new(browse::BrowsePlugin::new(platform.clone())),
        localsend::LocalSendPlugin::new(platform.clone()),
        streams::StreamPlugin::microphone(),
        streams::StreamPlugin::camera(),
        streams::StreamPlugin::phone_screen(),
        streams::StreamPlugin::desktop(),
        streams::StreamPlugin::extend(),
        streams::StreamPlugin::system_audio(),
        streams::StreamPlugin::phone_audio(),
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
