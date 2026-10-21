//! Typed packet bodies. Where a bridge exists, field names match the other
//! app so the bridge only has to rename the packet type.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct PairBody {
    pub pair: bool,
    /// Seconds since epoch; mixed into the verification key (protocol v8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct PingBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct BatteryBody {
    #[serde(rename = "currentCharge")]
    pub current_charge: i32,
    #[serde(rename = "isCharging")]
    pub is_charging: bool,
    #[serde(rename = "thresholdEvent", default)]
    pub threshold_event: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ClipboardBody {
    pub content: String,
    /// Only present in `konnekta.clipboard.initial`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ShareBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(
        rename = "creationTime",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub creation_time: Option<i64>,
    #[serde(
        rename = "lastModified",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub last_modified: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    #[serde(
        rename = "numberOfFiles",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub number_of_files: Option<u32>,
    #[serde(
        rename = "totalPayloadSize",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub total_payload_size: Option<i64>,
    /// Konnekta extension: resume offset (bytes already present at receiver).
    #[serde(
        rename = "konnektaResumeFrom",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub resume_from: Option<u64>,
    /// Konnekta extension: SHA-256 of the full file, for integrity / dedup.
    #[serde(
        rename = "konnektaSha256",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MprisBody {
    #[serde(
        rename = "playerList",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub player_list: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(rename = "isPlaying", default, skip_serializing_if = "Option::is_none")]
    pub is_playing: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<i64>,
    /// Cover of the current track (MPRIS `mpris:artUrl`); the picture itself
    /// follows as the payload of a packet with `transferringAlbumArt`.
    #[serde(
        rename = "albumArtUrl",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub album_art_url: Option<String>,
    #[serde(
        rename = "transferringAlbumArt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub transferring_album_art: Option<bool>,
    #[serde(rename = "canPause", default, skip_serializing_if = "Option::is_none")]
    pub can_pause: Option<bool>,
    #[serde(rename = "canPlay", default, skip_serializing_if = "Option::is_none")]
    pub can_play: Option<bool>,
    #[serde(rename = "canGoNext", default, skip_serializing_if = "Option::is_none")]
    pub can_go_next: Option<bool>,
    #[serde(
        rename = "canGoPrevious",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub can_go_previous: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MprisRequestBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<String>,
    /// Play, Pause, PlayPause, Stop, Next, Previous
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(rename = "setVolume", default, skip_serializing_if = "Option::is_none")]
    pub set_volume: Option<i32>,
    #[serde(
        rename = "requestPlayerList",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub request_player_list: Option<bool>,
    #[serde(
        rename = "requestNowPlaying",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub request_now_playing: Option<bool>,
    #[serde(
        rename = "requestVolume",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub request_volume: Option<bool>,
    #[serde(rename = "Seek", default, skip_serializing_if = "Option::is_none")]
    pub seek: Option<i64>,
    #[serde(
        rename = "SetPosition",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub set_position: Option<i64>,
}

/// Change one mixer channel and/or ask for the mixer state.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MixerRequestBody {
    /// Send the state now and push changes for a while.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
    /// Channel kind: `output`, `input` or `app`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// Percent, 0..=150.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mute: Option<bool>,
    /// Make this output / input the system default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub make_default: Option<bool>,
    /// Move this app to another output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MousepadBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dx: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dy: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub singleclick: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub doubleclick: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub middleclick: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rightclick: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub singlehold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub singlerelease: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scroll: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(
        rename = "specialKey",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub special_key: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shift: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ctrl: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub alt: bool,
    #[serde(rename = "super", default, skip_serializing_if = "std::ops::Not::not")]
    pub super_key: bool,
    #[serde(
        rename = "sendAck",
        default,
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub send_ack: bool,
    /// Konnekta extension: absolute pointer position normalised to 0..1
    /// (used by remote desktop where the phone shows the screen).
    #[serde(
        rename = "konnektaAbsX",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub abs_x: Option<f64>,
    #[serde(
        rename = "konnektaAbsY",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub abs_y: Option<f64>,
    /// Konnekta extension: three-finger swipe, `left` / `right` (switch
    /// workspace) or `up` / `down` (overview).
    #[serde(
        rename = "konnektaSwipe",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub swipe: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct PresenterBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dx: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dy: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stop: bool,
}

/// Entry inside the `commandList` JSON string of `konnekta.commands`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunCommandEntry {
    pub name: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RunCommandBody {
    /// Sent as a JSON-encoded *string* (as KDE Connect apps expect) of
    /// `{key: {name, command}}`.
    #[serde(rename = "commandList")]
    pub command_list: String,
    #[serde(rename = "canAddCommand", default)]
    pub can_add_command: bool,
}

impl RunCommandBody {
    pub fn from_map(map: &BTreeMap<String, RunCommandEntry>) -> Self {
        Self {
            command_list: serde_json::to_string(map).unwrap_or_else(|_| "{}".into()),
            can_add_command: false,
        }
    }

    pub fn entries(&self) -> BTreeMap<String, RunCommandEntry> {
        serde_json::from_str(&self.command_list).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RunCommandRequestBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(
        rename = "requestCommandList",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub request_command_list: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct NotificationBody {
    pub id: String,
    #[serde(rename = "appName", default)]
    pub app_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticker: Option<String>,
    #[serde(rename = "isClearable", default)]
    pub is_clearable: bool,
    #[serde(rename = "isCancel", default)]
    pub is_cancel: bool,
    #[serde(
        rename = "requestReplyId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub request_reply_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<String>>,
    #[serde(rename = "onlyOnce", default)]
    pub only_once: bool,
    #[serde(default)]
    pub silent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct TelephonyBody {
    /// ringing, talking, missedCall, sms (deprecated)
    pub event: String,
    #[serde(
        rename = "phoneNumber",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub phone_number: Option<String>,
    #[serde(
        rename = "contactName",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub contact_name: Option<String>,
    #[serde(rename = "isCancel", default)]
    pub is_cancel: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CallAction {
    Accept,
    Reject,
    Hangup,
    Mute,
    Unmute,
    Speaker,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CallActionBody {
    pub action: CallAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DndBody {
    pub enabled: bool,
    /// Wall clock ms when the change happened on the origin device.
    #[serde(rename = "changedAt")]
    pub changed_at: i64,
    /// True for a user change, false for a state report (e.g. on connect).
    #[serde(rename = "userInitiated", default)]
    pub user_initiated: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StreamAction {
    /// Consumer asks the producer to start streaming.
    Request,
    /// Producer announces a stream channel (`port`) and its parameters.
    Offer,
    /// Either side ends the session.
    Stop,
    /// Consumer asks for an IDR frame (video).
    Keyframe,
    /// Consumer requests a different quality / camera.
    Configure,
}

/// Body of every `konnekta.<stream>` packet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamMessage {
    pub action: StreamAction,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default)]
    pub params: StreamParams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// With a stop: what this computer still needs for the stream
    /// (program or module names) and the commands that install it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fix: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamParams {
    /// "pcm_s16le", "h264"
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub fps: u32,
    #[serde(rename = "bitrateKbps", default)]
    pub bitrate_kbps: u32,
    #[serde(rename = "sampleRate", default)]
    pub sample_rate: u32,
    #[serde(default)]
    pub channels: u8,
    /// "front" / "back" for cameras.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing: Option<String>,
}

impl Default for StreamParams {
    fn default() -> Self {
        Self {
            codec: String::new(),
            width: 0,
            height: 0,
            fps: 0,
            bitrate_kbps: 0,
            sample_rate: 48_000,
            channels: 1,
            facing: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runcommand_list_is_string_encoded() {
        let mut m = BTreeMap::new();
        m.insert(
            "k1".to_string(),
            RunCommandEntry {
                name: "Lock".into(),
                command: "loginctl lock-session".into(),
            },
        );
        let body = RunCommandBody::from_map(&m);
        let v = serde_json::to_value(&body).unwrap();
        assert!(v["commandList"].is_string());
        assert_eq!(body.entries(), m);
    }

    #[test]
    fn mousepad_skips_false_flags() {
        let b = MousepadBody {
            dx: Some(1.0),
            ..Default::default()
        };
        let s = serde_json::to_string(&b).unwrap();
        assert_eq!(s, r#"{"dx":1.0}"#);
    }
}
