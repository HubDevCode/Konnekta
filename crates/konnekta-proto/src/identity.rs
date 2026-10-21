use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    #[default]
    Desktop,
    Laptop,
    Phone,
    Tablet,
    Tv,
    #[serde(other)]
    Unknown,
}

/// Body of `konnekta.identity` (and of the bridges' identity packets).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentityBody {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    #[serde(rename = "deviceName")]
    pub device_name: String,
    #[serde(rename = "deviceType", default)]
    pub device_type: DeviceType,
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    #[serde(rename = "incomingCapabilities", default)]
    pub incoming_capabilities: Vec<String>,
    #[serde(rename = "outgoingCapabilities", default)]
    pub outgoing_capabilities: Vec<String>,
    #[serde(rename = "tcpPort", default, skip_serializing_if = "Option::is_none")]
    pub tcp_port: Option<u16>,
    /// Program name ("Konnekta"); other apps may send their own or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(
        rename = "appVersion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub app_version: Option<String>,
}

impl IdentityBody {
    /// Validate fields received from the network before trusting them for
    /// anything (display, storage keys, file names, ...).
    pub fn validate(&self) -> Result<(), &'static str> {
        if !is_valid_device_id(&self.device_id) {
            return Err("invalid deviceId");
        }
        if self.device_name.trim().is_empty() || self.device_name.chars().count() > 64 {
            return Err("invalid deviceName");
        }
        if self.device_name.chars().any(|c| c.is_control()) {
            return Err("invalid deviceName");
        }
        if self.incoming_capabilities.len() > 256 || self.outgoing_capabilities.len() > 256 {
            return Err("too many capabilities");
        }
        let long = |s: &Option<String>| s.as_ref().is_some_and(|v| v.len() > 64);
        if long(&self.app) || long(&self.app_version) {
            return Err("invalid app field");
        }
        Ok(())
    }

    pub fn supports_incoming(&self, packet_type: &str) -> bool {
        self.incoming_capabilities.iter().any(|c| c == packet_type)
    }
}

/// Device ids: 32..=38 chars of `[A-Za-z0-9_-]` (Konnekta uses 32 hex).
/// Older peers used UUIDs with dashes; we accept those too but never generate them.
pub fn is_valid_device_id(id: &str) -> bool {
    (32..=38).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_rules() {
        assert!(is_valid_device_id("0123456789abcdef0123456789abcdef"));
        assert!(is_valid_device_id("_0123456789abcdef0123456789abcde_"));
        assert!(!is_valid_device_id("short"));
        assert!(!is_valid_device_id(
            "../../../../etc/passwd/aaaaaaaaaaaaaaa"
        ));
    }

    #[test]
    fn unknown_device_type_is_tolerated() {
        let b: IdentityBody = serde_json::from_str(
            r#"{"deviceId":"0123456789abcdef0123456789abcdef","deviceName":"x","deviceType":"fridge","protocolVersion":8}"#,
        )
        .unwrap();
        assert_eq!(b.device_type, DeviceType::Unknown);
        assert!(b.validate().is_ok());
    }
}
