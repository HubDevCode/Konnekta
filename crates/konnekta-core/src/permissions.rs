//! Centralised permission model.
//!
//! Every plugin declares the [`Permission`] it needs. Permissions are granted
//! *per paired device* on the local machine and describe what that device may
//! do here (and which local data may be sent to it). Sensitive capabilities
//! (input injection, commands, camera, microphone, desktop) are never granted
//! implicitly.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    Ping,
    Battery,
    Clipboard,
    Files,
    Notifications,
    Telephony,
    MediaControl,
    Presenter,
    RemoteInput,
    Commands,
    Deck,
    Microphone,
    Camera,
    Desktop,
    SystemAudio,
    DoNotDisturb,
    Screenshots,
    FindMyPhone,
}

impl Permission {
    pub const ALL: &'static [Permission] = &[
        Self::Ping,
        Self::Battery,
        Self::Clipboard,
        Self::Files,
        Self::Notifications,
        Self::Telephony,
        Self::MediaControl,
        Self::Presenter,
        Self::RemoteInput,
        Self::Commands,
        Self::Deck,
        Self::Microphone,
        Self::Camera,
        Self::Desktop,
        Self::SystemAudio,
        Self::DoNotDisturb,
        Self::Screenshots,
        Self::FindMyPhone,
    ];

    /// High-risk permissions require an explicit user grant.
    pub fn is_sensitive(self) -> bool {
        matches!(
            self,
            Self::RemoteInput
                | Self::Commands
                | Self::Deck
                | Self::Microphone
                | Self::Camera
                | Self::Desktop
                | Self::SystemAudio
        )
    }

    pub fn defaults() -> BTreeSet<Permission> {
        Self::ALL
            .iter()
            .copied()
            .filter(|p| !p.is_sensitive())
            .collect()
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ping => "Ping",
            Self::Battery => "Battery status",
            Self::Clipboard => "Clipboard",
            Self::Files => "File transfer",
            Self::Notifications => "Notifications",
            Self::Telephony => "Calls",
            Self::MediaControl => "Media control",
            Self::Presenter => "Presentation remote",
            Self::RemoteInput => "Mouse & keyboard control",
            Self::Commands => "Run commands",
            Self::Deck => "Mobile Deck actions",
            Self::Microphone => "Microphone",
            Self::Camera => "Camera / webcam",
            Self::Desktop => "Remote desktop",
            Self::SystemAudio => "System audio",
            Self::DoNotDisturb => "Do Not Disturb sync",
            Self::Screenshots => "Screenshots",
            Self::FindMyPhone => "Find my device",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_not_default() {
        let d = Permission::defaults();
        for p in Permission::ALL {
            assert_eq!(d.contains(p), !p.is_sensitive());
        }
        assert!(!d.contains(&Permission::Commands));
        assert!(!d.contains(&Permission::RemoteInput));
    }

    #[test]
    fn serde_names() {
        assert_eq!(
            serde_json::to_string(&Permission::RemoteInput).unwrap(),
            "\"remoteInput\""
        );
    }
}
