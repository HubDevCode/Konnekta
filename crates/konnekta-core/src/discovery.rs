//! UDP discovery (KDE Connect compatible).
//!
//! The identity broadcast only contains public metadata (device id, name,
//! type, capabilities, TCP port). Receiving one never grants anything: it only
//! causes a TCP connection whose TLS handshake and pairing are then enforced.

use konnekta_proto::{IdentityBody, Packet, types};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::SocketAddr;
use tokio::net::UdpSocket;

pub fn bind_udp(addr: SocketAddr) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    // Allow coexistence with a KDE Connect daemon on the same machine.
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    socket.set_reuse_port(true)?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    UdpSocket::from_std(socket.into())
}

/// Parse an UDP datagram into a validated identity.
pub fn parse_announcement(data: &[u8]) -> Option<IdentityBody> {
    if data.len() > crate::link::MAX_IDENTITY_LEN {
        return None;
    }
    let packet = Packet::from_line(data).ok()?;
    if packet.packet_type != types::IDENTITY {
        return None;
    }
    let body: IdentityBody = packet.body_as().ok()?;
    body.validate().ok()?;
    body.tcp_port?;
    Some(body)
}

pub async fn announce(socket: &UdpSocket, identity: &Packet, targets: &[SocketAddr]) {
    let data = identity.to_line();
    for t in targets {
        if let Err(e) = socket.send_to(&data, t).await {
            tracing::debug!("announce to {t} failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_valid_announcement() {
        let p = Packet::new(
            types::IDENTITY,
            json!({"deviceId":"0123456789abcdef0123456789abcdef","deviceName":"Phone",
                   "deviceType":"phone","protocolVersion":8,"tcpPort":1716,
                   "incomingCapabilities":[],"outgoingCapabilities":[]}),
        );
        assert!(parse_announcement(&p.to_line()).is_some());
    }

    #[test]
    fn rejects_announcement_without_port_or_bad_id() {
        let p = Packet::new(
            types::IDENTITY,
            json!({"deviceId":"0123456789abcdef0123456789abcdef","deviceName":"P","protocolVersion":8}),
        );
        assert!(parse_announcement(&p.to_line()).is_none());
        let p = Packet::new(
            types::IDENTITY,
            json!({"deviceId":"x","deviceName":"P","protocolVersion":8,"tcpPort":1}),
        );
        assert!(parse_announcement(&p.to_line()).is_none());
    }
}
