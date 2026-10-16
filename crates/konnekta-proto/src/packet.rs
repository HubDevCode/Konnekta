use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// Hard upper bound for a single JSON packet line. Anything bigger is treated
/// as hostile and the connection is dropped.
pub const MAX_PACKET_LEN: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum PacketError {
    #[error("packet exceeds {MAX_PACKET_LEN} bytes")]
    TooLarge,
    #[error("malformed packet: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("invalid body for {0}")]
    InvalidBody(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadTransferInfo {
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Packet {
    pub id: i64,
    #[serde(rename = "type")]
    pub packet_type: String,
    #[serde(default)]
    pub body: Map<String, Value>,
    #[serde(
        rename = "payloadSize",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub payload_size: Option<i64>,
    #[serde(
        rename = "payloadTransferInfo",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub payload_transfer_info: Option<PayloadTransferInfo>,
}

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

impl Packet {
    pub fn new(packet_type: impl Into<String>, body: impl Serialize) -> Self {
        let body = match serde_json::to_value(body) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        Self {
            id: now_millis(),
            packet_type: packet_type.into(),
            body,
            payload_size: None,
            payload_transfer_info: None,
        }
    }

    pub fn empty(packet_type: impl Into<String>) -> Self {
        Self::new(packet_type, Map::new())
    }

    pub fn with_payload(mut self, size: i64, port: u16) -> Self {
        self.payload_size = Some(size);
        self.payload_transfer_info = Some(PayloadTransferInfo { port });
        self
    }

    pub fn has_payload(&self) -> bool {
        self.payload_transfer_info.is_some()
    }

    /// Decode the body into a typed structure.
    pub fn body_as<T: serde::de::DeserializeOwned>(&self) -> Result<T, PacketError> {
        serde_json::from_value(Value::Object(self.body.clone()))
            .map_err(|_| PacketError::InvalidBody(self.packet_type.clone()))
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.body.get(key).and_then(Value::as_str)
    }

    pub fn get_bool(&self, key: &str) -> bool {
        self.body.get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.body.get(key).and_then(Value::as_i64)
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.body.get(key).and_then(Value::as_f64)
    }

    /// Serialize to a single line terminated by `\n` (wire format).
    pub fn to_line(&self) -> Vec<u8> {
        let mut out = serde_json::to_vec(self).expect("packet serialization cannot fail");
        out.push(b'\n');
        out
    }

    /// Parse a single line (with or without trailing newline).
    pub fn from_line(line: &[u8]) -> Result<Self, PacketError> {
        if line.len() > MAX_PACKET_LEN {
            return Err(PacketError::TooLarge);
        }
        let trimmed = line.strip_suffix(b"\n").unwrap_or(line);
        let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
        Ok(serde_json::from_slice(trimmed)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip() {
        let p = Packet::new("kdeconnect.ping", json!({"message": "hi"})).with_payload(10, 1739);
        let line = p.to_line();
        assert_eq!(*line.last().unwrap(), b'\n');
        let back = Packet::from_line(&line).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.get_str("message"), Some("hi"));
    }

    #[test]
    fn parses_kde_connect_packet() {
        let raw = br#"{"id":1700000000000,"type":"kdeconnect.clipboard","body":{"content":"x"}}"#;
        let p = Packet::from_line(raw).unwrap();
        assert_eq!(p.packet_type, "kdeconnect.clipboard");
        assert!(!p.has_payload());
    }

    #[test]
    fn rejects_oversized() {
        let big = vec![b' '; MAX_PACKET_LEN + 1];
        assert!(matches!(
            Packet::from_line(&big),
            Err(PacketError::TooLarge)
        ));
    }

    #[test]
    fn rejects_garbage() {
        assert!(Packet::from_line(b"not json").is_err());
        assert!(Packet::from_line(br#"{"id":1}"#).is_err());
    }
}
