//! Konnekta wire protocol.
//!
//! Konnekta speaks its **own protocol** (`Protocol::Konnekta`) on its own
//! ports. The packet format derives from KDE Connect v8 — newline-delimited
//! JSON `{"id": <ms>, "type": "konnekta.<feature>", "body": {...}}` with
//! optional out-of-band payloads — but the handshake is TLS-first (no clear
//! text identity), pairing keys have 64 bits and every packet type lives in
//! the `konnekta.` namespace.
//!
//! An optional compatibility mode (`Protocol::KdeConnect`, off by default)
//! lets the KDE Connect / GSConnect apps connect: [`kde`] translates the
//! standard packet types at the link boundary, so plugins only ever see the
//! canonical `konnekta.*` names.

pub mod bodies;
pub mod identity;
pub mod kde;
pub mod packet;
pub mod types;

pub use identity::{DeviceType, IdentityBody, is_valid_device_id};
pub use packet::{MAX_PACKET_LEN, Packet, PacketError, PayloadTransferInfo};

use std::ops::RangeInclusive;

/// Version of the native Konnekta protocol.
pub const PROTOCOL_VERSION: u32 = 1;
/// KDE Connect protocol version spoken in compatibility mode.
pub const KDE_PROTOCOL_VERSION: u32 = 8;

/// Which protocol a link speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Protocol {
    /// Native Konnekta protocol.
    Konnekta,
    /// KDE Connect v8 compatibility mode.
    KdeConnect,
}

impl Protocol {
    pub fn version(self) -> u32 {
        match self {
            Self::Konnekta => PROTOCOL_VERSION,
            Self::KdeConnect => KDE_PROTOCOL_VERSION,
        }
    }

    pub fn udp_port(self) -> u16 {
        match self {
            Self::Konnekta => 17160,
            Self::KdeConnect => 1716,
        }
    }

    pub fn tcp_ports(self) -> RangeInclusive<u16> {
        match self {
            Self::Konnekta => 17160..=17168,
            Self::KdeConnect => 1716..=1764,
        }
    }

    pub fn payload_ports(self) -> RangeInclusive<u16> {
        match self {
            Self::Konnekta => 17170..=17199,
            Self::KdeConnect => 1739..=1764,
        }
    }

    pub fn identity_type(self) -> &'static str {
        match self {
            Self::Konnekta => types::IDENTITY,
            Self::KdeConnect => kde::IDENTITY,
        }
    }

    /// Hex characters of the pairing verification key (64 bits natively,
    /// 32 bits for KDE Connect clients, which only show 8).
    pub fn key_len(self) -> usize {
        match self {
            Self::Konnekta => 16,
            Self::KdeConnect => 8,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Konnekta => "Konnekta",
            Self::KdeConnect => "KDE Connect",
        }
    }
}

/// mDNS service type of the native protocol.
pub const MDNS_SERVICE: &str = "_konnekta._tcp.local.";
