//! Bridge plugins for mobile front-ends.
//!
//! On Android / iOS the OS integrations (notification listener, camera,
//! telephony, MediaSession...) live in Kotlin / Swift / Dart. These plugins
//! declare the *phone side* capabilities and permissions, and forward every
//! authorised packet to the host app as an event; the host replies through
//! the FFI `sendPacket` call. Networking, TLS, pairing and permission checks
//! all stay in the Rust core.

use anyhow::Result;
use async_trait::async_trait;
use konnekta_core::proto::{Packet, types};
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Packets with a payload waiting for the host to download them.
#[derive(Default)]
pub struct PendingPayloads {
    map: Mutex<HashMap<String, (String, Packet, Instant)>>,
}

impl PendingPayloads {
    pub fn put(&self, device_id: &str, p: Packet) -> String {
        let key = format!("{}-{}", device_id, p.id);
        let mut m = self.map.lock().unwrap();
        m.retain(|_, (_, _, t)| t.elapsed() < Duration::from_secs(120));
        m.insert(key.clone(), (device_id.to_owned(), p, Instant::now()));
        key
    }

    pub fn take(&self, key: &str) -> Option<(String, Packet)> {
        self.map.lock().unwrap().remove(key).map(|(d, p, _)| (d, p))
    }
}

pub struct BridgePlugin {
    info: PluginInfo,
    pending: Arc<PendingPayloads>,
}

#[async_trait]
impl Plugin for BridgePlugin {
    fn info(&self) -> PluginInfo {
        self.info.clone()
    }

    async fn on_connected(&self, dev: &DeviceHandle) {
        dev.emit(self.info.id, "connected", json!({}));
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        let payload_key = p
            .has_payload()
            .then(|| self.pending.put(dev.id(), p.clone()));
        dev.emit(
            self.info.id,
            "packet",
            json!({ "type": p.packet_type, "body": p.body, "payloadSize": p.payload_size, "payloadKey": payload_key }),
        );
        Ok(())
    }
}

macro_rules! bridge {
    ($id:literal, $name:literal, $perm:ident, [$($in:expr),*], [$($out:expr),*]) => {
        PluginInfo {
            id: $id,
            name: $name,
            description: $name,
            permission: Permission::$perm,
            incoming: &[$($in),*],
            outgoing: &[$($out),*],
        }
    };
}

/// Phone-side plugin set. `call_control` should be false on iOS.
pub fn mobile(pending: Arc<PendingPayloads>, call_control: bool) -> Vec<Arc<dyn Plugin>> {
    use types::*;
    let telephony_in: &'static [&'static str] = if call_control {
        &[CALL_MUTE, CALL_ACTION]
    } else {
        &[CALL_MUTE]
    };
    let mut infos = vec![
        bridge!("ping", "Ping", Ping, [PING], [PING]),
        bridge!("battery", "Battery", Battery, [], [BATTERY]),
        bridge!(
            "clipboard",
            "Clipboard",
            Clipboard,
            [CLIPBOARD, CLIPBOARD_INITIAL],
            [CLIPBOARD, CLIPBOARD_INITIAL]
        ),
        bridge!(
            "share",
            "Share",
            Files,
            [SHARE, SHARE_UPDATE],
            [SHARE, SHARE_UPDATE]
        ),
        bridge!(
            "media",
            "Media control",
            MediaControl,
            [MEDIA, MEDIA_REQUEST],
            [MEDIA_REQUEST, MEDIA]
        ),
        bridge!(
            "mixer",
            "Audio mixer",
            MediaControl,
            [MIXER],
            [MIXER_REQUEST]
        ),
        bridge!(
            "input",
            "Remote input",
            RemoteInput,
            [INPUT_STATE, INPUT_ECHO],
            [INPUT]
        ),
        bridge!(
            "presenter",
            "Presentation remote",
            Presenter,
            [],
            [PRESENTER]
        ),
        bridge!(
            "commands",
            "Custom commands",
            Commands,
            [COMMANDS],
            [COMMANDS_REQUEST]
        ),
        bridge!(
            "notifications",
            "Notifications",
            Notifications,
            [
                NOTIFICATION_REQUEST,
                NOTIFICATION_REPLY,
                NOTIFICATION_ACTION
            ],
            [NOTIFICATION]
        ),
        bridge!(
            "sms",
            "Messages",
            Messages,
            [SMS_SEND, SMS_CONVERSATIONS_REQUEST, SMS_THREAD_REQUEST],
            [SMS_MESSAGES]
        ),
        bridge!(
            "photo",
            "Camera and scanner",
            Files,
            [PHOTO_REQUEST],
            [PHOTO]
        ),
        bridge!(
            "approve",
            "Fingerprint approval",
            Approve,
            [APPROVE_REQUEST],
            [APPROVE]
        ),
        bridge!("appupdate", "App update", Files, [APP_UPDATE], []),
        bridge!(
            "finddevice",
            "Find my phone",
            FindMyPhone,
            [FIND_DEVICE],
            []
        ),
        bridge!("deck", "Mobile Deck", Deck, [DECK], [DECK_REQUEST]),
        bridge!("dnd", "Do Not Disturb sync", DoNotDisturb, [DND], [DND]),
        bridge!(
            "screenshot",
            "Screenshots",
            Screenshots,
            [SCREENSHOT],
            [SCREENSHOT]
        ),
        bridge!(
            "browse",
            "Browse PC",
            BrowseFiles,
            [BROWSE],
            [BROWSE_REQUEST]
        ),
        bridge!(
            "microphone",
            "Microphone",
            Microphone,
            [MICROPHONE],
            [MICROPHONE]
        ),
        bridge!("camera", "Camera", Camera, [CAMERA], [CAMERA]),
        bridge!(
            "phonescreen",
            "Phone screen",
            Desktop,
            [PHONE_SCREEN],
            [PHONE_SCREEN]
        ),
        bridge!("desktop", "Remote desktop", Desktop, [DESKTOP], [DESKTOP]),
        bridge!(
            "phoneaudio",
            "Phone audio",
            SystemAudio,
            [PHONE_AUDIO],
            [PHONE_AUDIO]
        ),
        bridge!(
            "systemaudio",
            "Computer audio",
            SystemAudio,
            [SYSTEM_AUDIO],
            [SYSTEM_AUDIO]
        ),
    ];
    infos.push(PluginInfo {
        id: "calls",
        name: "Calls",
        description: "Calls",
        permission: Permission::Telephony,
        incoming: telephony_in,
        outgoing: &[CALL],
    });
    infos
        .into_iter()
        .map(|info| {
            Arc::new(BridgePlugin {
                info,
                pending: pending.clone(),
            }) as Arc<dyn Plugin>
        })
        .collect()
}
