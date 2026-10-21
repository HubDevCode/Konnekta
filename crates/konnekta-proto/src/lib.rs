//! Konnekta wire protocol.
//!
//! Konnekta speaks its **own protocol** (`Protocol::Konnekta`) on its own
//! ports: newline-delimited JSON packets `{"id": <ms>, "type":
//! "konnekta.<feature>", "body": {...}}` with optional out-of-band payloads,
//! a TLS-first handshake (no clear-text identity), 64-bit pairing keys and
//! the `konnekta.` namespace for every packet type.
//!
//! Optional interop adapters, off by default, let devices of other
//! ecosystems connect: `Protocol::KdeConnect` (KDE Connect, GSConnect) and
//! `Protocol::Flux` (Flux apps). [`interop`] translates packet types and
//! bodies at the link boundary, so plugins only ever see Konnekta types.

pub mod bodies;
pub mod identity;
pub mod interop;
pub mod packet;
pub mod types;

pub use identity::{DeviceType, IdentityBody, is_valid_device_id};
pub use packet::{MAX_PACKET_LEN, Packet, PacketError, PayloadTransferInfo};

use std::ops::RangeInclusive;

/// Version of the native Konnekta protocol.
pub const PROTOCOL_VERSION: u32 = 1;
/// Protocol version spoken by the KDE Connect and Flux adapters.
pub const V8_PROTOCOL_VERSION: u32 = 8;

/// Which protocol a link speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Protocol {
    /// Native Konnekta protocol.
    Konnekta,
    /// KDE Connect v8 interop.
    KdeConnect,
    /// Flux interop (Flux protocol 8).
    Flux,
}

impl Protocol {
    pub fn version(self) -> u32 {
        match self {
            Self::Konnekta => PROTOCOL_VERSION,
            Self::KdeConnect | Self::Flux => V8_PROTOCOL_VERSION,
        }
    }

    pub fn udp_port(self) -> u16 {
        match self {
            Self::Konnekta => 17160,
            Self::KdeConnect => 1716,
            Self::Flux => 12100,
        }
    }

    pub fn tcp_ports(self) -> RangeInclusive<u16> {
        match self {
            Self::Konnekta => 17160..=17168,
            Self::KdeConnect => 1716..=1764,
            Self::Flux => 12100..=12108,
        }
    }

    pub fn payload_ports(self) -> RangeInclusive<u16> {
        match self {
            Self::Konnekta => 17170..=17199,
            Self::KdeConnect => 1739..=1764,
            Self::Flux => 12070..=12099,
        }
    }

    pub fn identity_type(self) -> &'static str {
        match self {
            Self::Konnekta => types::IDENTITY,
            Self::KdeConnect => "kdeconnect.identity",
            Self::Flux => "flux.identity",
        }
    }

    /// Hex characters of the pairing verification key (64 bits; KDE
    /// Connect clients only show 8).
    pub fn key_len(self) -> usize {
        match self {
            Self::Konnekta | Self::Flux => 16,
            Self::KdeConnect => 8,
        }
    }

    /// Clear-text identity on TCP, then TLS with reversed roles.
    pub fn is_v8(self) -> bool {
        self != Self::Konnekta
    }

    /// The interop adapters, in a fixed order.
    pub const FOREIGN: [Protocol; 2] = [Self::KdeConnect, Self::Flux];

    /// mDNS service browsed for this protocol, if any.
    pub fn mdns_service(self) -> Option<&'static str> {
        match self {
            Self::Konnekta => Some(MDNS_SERVICE),
            Self::KdeConnect => None,
            Self::Flux => Some("_flux._udp.local."),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Konnekta => "Konnekta",
            Self::KdeConnect => "KDE Connect",
            Self::Flux => "Flux",
        }
    }
}

/// mDNS service type of the native protocol.
pub const MDNS_SERVICE: &str = "_konnekta._tcp.local.";
