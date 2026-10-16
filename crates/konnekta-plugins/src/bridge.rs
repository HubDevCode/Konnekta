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
use konnekta_core::{DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin};
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
    ($id:literal, $name:literal, $origin:ident, $perm:ident, [$($in:expr),*], [$($out:expr),*]) => {
        PluginInfo {
            id: $id,
            name: $name,
            description: $name,
            origin: PluginOrigin::$origin,
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
        &[TELEPHONY_REQUEST_MUTE, K_CALL_ACTION]
    } else {
        &[TELEPHONY_REQUEST_MUTE]
    };
    let mut infos = vec![
        bridge!("ping", "Ping", KdeConnect, Ping, [PING], [PING]),
        bridge!("battery", "Battery", KdeConnect, Battery, [], [BATTERY]),
        bridge!(
            "clipboard",
            "Clipboard",
            KdeConnect,
            Clipboard,
            [CLIPBOARD, CLIPBOARD_CONNECT],
            [CLIPBOARD, CLIPBOARD_CONNECT]
        ),
        bridge!(
            "share",
            "Share",
            KdeConnect,
            Files,
            [SHARE_REQUEST, SHARE_REQUEST_UPDATE],
            [SHARE_REQUEST, SHARE_REQUEST_UPDATE]
        ),
        bridge!(
            "mpris",
            "Media control",
            KdeConnect,
            MediaControl,
            [MPRIS, MPRIS_REQUEST],
            [MPRIS_REQUEST, MPRIS]
        ),
        bridge!(
            "mousepad",
            "Remote input",
            KdeConnect,
            RemoteInput,
            [MOUSEPAD_KEYBOARDSTATE, MOUSEPAD_ECHO],
            [MOUSEPAD_REQUEST]
        ),
        bridge!(
            "presenter",
            "Presentation remote",
            KdeConnect,
            Presenter,
            [],
            [PRESENTER]
        ),
        bridge!(
            "runcommand",
            "Custom commands",
            KdeConnect,
            Commands,
            [RUNCOMMAND],
            [RUNCOMMAND_REQUEST]
        ),
        bridge!(
            "notifications",
            "Notifications",
            KdeConnect,
            Notifications,
            [
                NOTIFICATION_REQUEST,
                NOTIFICATION_REPLY,
                NOTIFICATION_ACTION
            ],
            [NOTIFICATION]
        ),
        bridge!(
            "findmyphone",
            "Find my phone",
            KdeConnect,
            FindMyPhone,
            [FINDMYPHONE_REQUEST],
            []
        ),
        bridge!(
            "deck",
            "Mobile Deck",
            Konnekta,
            Deck,
            [K_DECK],
            [K_DECK_REQUEST]
        ),
        bridge!(
            "dnd",
            "Do Not Disturb sync",
            Konnekta,
            DoNotDisturb,
            [K_DND],
            [K_DND]
        ),
        bridge!(
            "screenshot",
            "Screenshots",
            Konnekta,
            Screenshots,
            [K_SCREENSHOT],
            [K_SCREENSHOT]
        ),
        bridge!(
            "microphone",
            "Microphone",
            Konnekta,
            Microphone,
            [K_MICROPHONE],
            [K_MICROPHONE]
        ),
        bridge!("camera", "Camera", Konnekta, Camera, [K_CAMERA], [K_CAMERA]),
        bridge!(
            "phonescreen",
            "Phone screen",
            Konnekta,
            Desktop,
            [K_PHONE_SCREEN],
            [K_PHONE_SCREEN]
        ),
        bridge!(
            "desktop",
            "Remote desktop",
            Konnekta,
            Desktop,
            [K_DESKTOP],
            [K_DESKTOP]
        ),
        bridge!(
            "systemaudio",
            "Computer audio",
            Konnekta,
            SystemAudio,
            [K_SYSTEM_AUDIO],
            [K_SYSTEM_AUDIO]
        ),
    ];
    infos.push(PluginInfo {
        id: "telephony",
        name: "Calls",
        description: "Calls",
        origin: PluginOrigin::KdeConnect,
        permission: Permission::Telephony,
        incoming: telephony_in,
        outgoing: &[TELEPHONY],
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
