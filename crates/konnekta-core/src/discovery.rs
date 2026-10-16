//! Discovery: UDP broadcast for both protocols, mDNS for the native one.
//!
//! An announcement only contains public metadata (id, name, type, TCP port).
//! Receiving one never grants anything: it only causes an outgoing connection
//! whose TLS handshake and pairing rules are then enforced.

use konnekta_proto::{DeviceType, IdentityBody, Packet, Protocol, kde, types};
use socket2::{Domain, Protocol as SockProto, Socket, Type};
use std::net::SocketAddr;
use tokio::net::UdpSocket;

pub fn bind_udp(addr: SocketAddr) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(SockProto::UDP))?;
    socket.set_reuse_address(true)?;
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    socket.set_reuse_port(true)?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    UdpSocket::from_std(socket.into())
}

/// A device seen on the network.
#[derive(Debug, Clone)]
pub struct Announcement {
    pub protocol: Protocol,
    pub device_id: String,
    pub device_name: String,
    pub device_type: DeviceType,
    pub addr: SocketAddr,
}

/// Parse a UDP datagram of either protocol.
pub fn parse_announcement(data: &[u8], from: SocketAddr) -> Option<Announcement> {
    if data.len() > crate::link::MAX_IDENTITY_LEN {
        return None;
    }
    let packet = Packet::from_line(data).ok()?;
    let protocol = match packet.packet_type.as_str() {
        types::IDENTITY => Protocol::Konnekta,
        kde::IDENTITY => Protocol::KdeConnect,
        _ => return None,
    };
    let body: IdentityBody = packet.body_as().ok()?;
    body.validate().ok()?;
    let port = body.tcp_port?;
    if !protocol.tcp_ports().contains(&port) && port < 1024 {
        return None;
    }
    Some(Announcement {
        protocol,
        device_id: body.device_id,
        device_name: body.device_name,
        device_type: body.device_type,
        addr: SocketAddr::new(from.ip(), port),
    })
}

/// Minimal native announcement: no capabilities, just what is needed to dial.
pub fn native_announcement(id: &str, name: &str, ty: DeviceType, tcp_port: u16) -> Packet {
    Packet::new(
        types::IDENTITY,
        IdentityBody {
            device_id: id.into(),
            device_name: name.into(),
            device_type: ty,
            protocol_version: konnekta_proto::PROTOCOL_VERSION,
            incoming_capabilities: vec![],
            outgoing_capabilities: vec![],
            tcp_port: Some(tcp_port),
            app: Some("Konnekta".into()),
            app_version: None,
        },
    )
}

pub async fn announce(socket: &UdpSocket, packet: &Packet, targets: &[SocketAddr]) {
    let data = packet.to_line();
    for t in targets {
        if let Err(e) = socket.send_to(&data, t).await {
            tracing::debug!("announce to {t} failed: {e}");
        }
    }
}

/// mDNS advertisement + browsing for `_konnekta._tcp`.
pub mod mdns {
    use super::*;
    use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

    pub struct Mdns {
        daemon: ServiceDaemon,
        fullname: Option<String>,
    }

    impl Mdns {
        pub fn start() -> anyhow::Result<Self> {
            Ok(Self {
                daemon: ServiceDaemon::new()?,
                fullname: None,
            })
        }

        pub fn advertise(
            &mut self,
            id: &str,
            name: &str,
            ty: DeviceType,
            port: u16,
        ) -> anyhow::Result<()> {
            let host = format!("{id}.local.");
            let ty_s = serde_json::to_value(ty)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            let props = [
                ("id", id),
                ("name", name),
                ("type", ty_s.as_str()),
                ("v", "1"),
            ];
            let info = ServiceInfo::new(
                konnekta_proto::MDNS_SERVICE,
                id,
                &host,
                "",
                port,
                &props[..],
            )?
            .enable_addr_auto();
            self.fullname = Some(info.get_fullname().to_owned());
            self.daemon.register(info)?;
            Ok(())
        }

        /// Browse and forward resolved devices.
        pub fn browse(
            &self,
            tx: tokio::sync::mpsc::UnboundedSender<Announcement>,
        ) -> anyhow::Result<()> {
            let rx = self.daemon.browse(konnekta_proto::MDNS_SERVICE)?;
            std::thread::Builder::new()
                .name("konnekta-mdns".into())
                .spawn(move || {
                    while let Ok(ev) = rx.recv() {
                        let ServiceEvent::ServiceResolved(info) = ev else {
                            continue;
                        };
                        let get = |k: &str| info.get_property_val_str(k).map(str::to_owned);
                        let (Some(id), Some(name)) = (get("id"), get("name")) else {
                            continue;
                        };
                        if !konnekta_proto::is_valid_device_id(&id) {
                            continue;
                        }
                        let device_type = get("type")
                            .and_then(|t| serde_json::from_value(serde_json::Value::String(t)).ok())
                            .unwrap_or_default();
                        for ip in info.get_addresses() {
                            let a = Announcement {
                                protocol: Protocol::Konnekta,
                                device_id: id.clone(),
                                device_name: name.chars().take(64).collect(),
                                device_type,
                                addr: SocketAddr::new(ip.to_ip_addr(), info.get_port()),
                            };
                            if tx.send(a).is_err() {
                                return;
                            }
                        }
                    }
                })?;
            Ok(())
        }
    }

    impl Drop for Mdns {
        fn drop(&mut self) {
            if let Some(f) = &self.fullname {
                let _ = self.daemon.unregister(f);
            }
            let _ = self.daemon.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::{IpAddr, Ipv4Addr};

    const FROM: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 9)), 5000);

    #[test]
    fn parses_both_protocols() {
        let p = native_announcement(
            "0123456789abcdef0123456789abcdef",
            "Phone",
            DeviceType::Phone,
            17160,
        );
        let a = parse_announcement(&p.to_line(), FROM).unwrap();
        assert_eq!(a.protocol, Protocol::Konnekta);
        assert_eq!(a.addr.port(), 17160);
        let k = Packet::new(
            kde::IDENTITY,
            json!({"deviceId":"0123456789abcdef0123456789abcdef","deviceName":"P","protocolVersion":8,"tcpPort":1716}),
        );
        assert_eq!(
            parse_announcement(&k.to_line(), FROM).unwrap().protocol,
            Protocol::KdeConnect
        );
    }

    #[test]
    fn rejects_bad_announcements() {
        let p = Packet::new(
            types::IDENTITY,
            json!({"deviceId":"x","deviceName":"P","protocolVersion":1,"tcpPort":17160}),
        );
        assert!(parse_announcement(&p.to_line(), FROM).is_none());
        let p = Packet::new(
            types::IDENTITY,
            json!({"deviceId":"0123456789abcdef0123456789abcdef","deviceName":"P","protocolVersion":1}),
        );
        assert!(parse_announcement(&p.to_line(), FROM).is_none());
        let p = Packet::new(types::PING, json!({}));
        assert!(parse_announcement(&p.to_line(), FROM).is_none());
    }
}
