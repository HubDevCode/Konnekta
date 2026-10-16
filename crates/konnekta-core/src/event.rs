use crate::device::DeviceInfo;
use serde::Serialize;
use serde_json::Value;

/// Events published to user interfaces (desktop app, CLI, mobile FFI).
/// They never contain secrets; private content (clipboard, notifications) is
/// only included in plugin events meant to be shown to the local user.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    DeviceConnected {
        device: DeviceInfo,
    },
    DeviceUpdated {
        device: DeviceInfo,
    },
    #[serde(rename_all = "camelCase")]
    DeviceDisconnected {
        device_id: String,
    },
    /// A remote device asks to pair. Show `verification_key` and ask the user.
    #[serde(rename_all = "camelCase")]
    PairRequested {
        device_id: String,
        device_name: String,
        verification_key: String,
    },
    /// We asked to pair; the same key must be displayed on the other device.
    #[serde(rename_all = "camelCase")]
    PairOutgoing {
        device_id: String,
        verification_key: String,
    },
    #[serde(rename_all = "camelCase")]
    Paired {
        device_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Unpaired {
        device_id: String,
    },
    #[serde(rename_all = "camelCase")]
    PairRejected {
        device_id: String,
        reason: String,
    },
    #[serde(rename_all = "camelCase")]
    SecurityWarning {
        device_id: Option<String>,
        message: String,
    },
    #[serde(rename_all = "camelCase")]
    Plugin {
        device_id: Option<String>,
        plugin: String,
        kind: String,
        data: Value,
    },
}
