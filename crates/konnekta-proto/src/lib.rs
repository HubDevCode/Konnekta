//! Konnekta wire protocol.
//!
//! The base format is the KDE Connect protocol (version 8): newline-delimited
//! JSON packets of the form `{"id": <ms>, "type": "<type>", "body": {...}}`,
//! optionally followed by an out-of-band payload transferred over a separate
//! TLS socket announced in `payloadTransferInfo`.
//!
//! Packet types starting with `kdeconnect.` are standard and interoperate with
//! KDE Connect / GSConnect / Zorin Connect. Types starting with `konnekta.` are
//! proprietary extensions and are only sent to peers that advertise them in
//! their identity capabilities.

pub mod bodies;
pub mod identity;
pub mod packet;
pub mod types;

pub use identity::{DeviceType, IdentityBody, is_valid_device_id};
pub use packet::{Packet, PacketError, PayloadTransferInfo, MAX_PACKET_LEN};

/// KDE Connect protocol version implemented.
pub const PROTOCOL_VERSION: u32 = 8;
/// UDP discovery port and first TCP port (KDE Connect standard).
pub const DEFAULT_PORT: u16 = 1716;
/// Last TCP port tried when 1716 is busy.
pub const MAX_PORT: u16 = 1764;
/// Port range used for payload (file / stream) servers.
pub const PAYLOAD_PORT_MIN: u16 = 1739;
pub const PAYLOAD_PORT_MAX: u16 = 1764;
