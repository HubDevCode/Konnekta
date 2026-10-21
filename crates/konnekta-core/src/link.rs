//! Link establishment and framing for both protocols.
//!
//! **Native Konnekta**: TCP, then TLS immediately with normal roles (the
//! dialer is the TLS client). Both certificates are required. The identity
//! is exchanged only *inside* TLS, so nothing about the device travels in
//! clear text on the connection. A listener cannot know who connects before
//! the handshake, so it accepts any certificate there
//! ([`PeerExpectation::Deferred`]) and the core checks it against the
//! announced identity and the trust store before the link carries anything.
//!
//! **Interop adapters** (KDE Connect, Flux): the "v8" handshake — clear text
//! identity, then TLS with inverted roles, then identity again inside TLS.
//! Packet types and bodies are translated here, at the boundary.

use anyhow::{Context, Result, bail};
use konnekta_crypto::{LocalIdentity, PeerExpectation, client_config, server_config};
use konnekta_proto::{IdentityBody, Packet, Protocol, interop};
use rustls_pki_types::ServerName;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Identity packets are small; anything bigger before authentication is abuse.
pub const MAX_IDENTITY_LEN: usize = 16 * 1024;
/// Line limit until the device is paired.
pub const MAX_UNPAIRED_LINE: usize = 64 * 1024;
/// Line limit for paired devices.
pub const MAX_PAIRED_LINE: usize = konnekta_proto::MAX_PACKET_LEN;

pub type SecureStream = TlsStream<TcpStream>;

/// Read one `\n` terminated line, refusing lines longer than `max`.
pub async fn read_line_limited<R: AsyncBufRead + Unpin>(
    r: &mut R,
    max: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    loop {
        let buf = r.fill_buf().await?;
        if buf.is_empty() {
            return if out.is_empty() {
                Ok(None)
            } else {
                Err(std::io::ErrorKind::UnexpectedEof.into())
            };
        }
        if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            out.extend_from_slice(&buf[..pos]);
            r.consume(pos + 1);
            if out.len() > max {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "line too long",
                ));
            }
            return Ok(Some(out));
        }
        let n = buf.len();
        out.extend_from_slice(buf);
        r.consume(n);
        if out.len() > max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "line too long",
            ));
        }
    }
}

/// Parse and validate an identity packet of `proto`. Capabilities of
/// foreign peers are translated to Konnekta types.
pub fn identity_from_packet(p: &Packet, proto: Protocol) -> Result<IdentityBody> {
    if p.packet_type != proto.identity_type() {
        bail!("expected {} identity, got {}", proto.label(), p.packet_type);
    }
    let mut body: IdentityBody = p.body_as()?;
    body.validate().map_err(anyhow::Error::msg)?;
    match proto {
        Protocol::KdeConnect | Protocol::Flux => {
            if body.protocol_version < 8 {
                bail!(
                    "{} protocol {} not supported (need 8)",
                    proto.label(),
                    body.protocol_version
                );
            }
            body.incoming_capabilities =
                interop::caps_from_wire(proto, &body.incoming_capabilities);
            body.outgoing_capabilities =
                interop::caps_from_wire(proto, &body.outgoing_capabilities);
        }
        Protocol::Konnekta => {
            if body.protocol_version != konnekta_proto::PROTOCOL_VERSION {
                bail!("Konnekta protocol {} not supported", body.protocol_version);
            }
        }
    }
    Ok(body)
}

pub fn tune_socket(stream: &TcpStream) {
    let _ = stream.set_nodelay(true);
    let sock = socket2::SockRef::from(stream);
    let ka = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(10))
        .with_interval(Duration::from_secs(5));
    let _ = sock.set_tcp_keepalive(&ka);
}

async fn tcp_connect(addr: SocketAddr) -> Result<TcpStream> {
    let tcp = tokio::time::timeout(HANDSHAKE_TIMEOUT, TcpStream::connect(addr))
        .await
        .context("connect timeout")??;
    tune_socket(&tcp);
    Ok(tcp)
}

// --- native -----------------------------------------------------------------------

/// Dial a device with the native protocol (we are TLS client).
pub async fn dial_native(
    local: &LocalIdentity,
    addr: SocketAddr,
    expect: PeerExpectation,
) -> Result<SecureStream> {
    let tcp = tcp_connect(addr).await?;
    let connector = TlsConnector::from(client_config(local, expect)?);
    let tls = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        connector.connect(ServerName::IpAddress(addr.ip().into()), tcp),
    )
    .await
    .context("tls timeout")??;
    Ok(TlsStream::Client(tls))
}

/// Accept a native connection (we are TLS server, peer unknown until the
/// identity arrives inside TLS).
pub async fn accept_native(local: &LocalIdentity, tcp: TcpStream) -> Result<SecureStream> {
    tune_socket(&tcp);
    let acceptor = TlsAcceptor::from(server_config(local, PeerExpectation::Deferred)?);
    let tls = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp))
        .await
        .context("tls timeout")??;
    Ok(TlsStream::Server(tls))
}

// --- Interop adapters (v8 handshake) ------------------------------------------------

/// Read the clear-text identity a KDE Connect or Flux peer sends before TLS.
pub async fn read_plain_identity(stream: &mut TcpStream, proto: Protocol) -> Result<IdentityBody> {
    // Capacity 1: never read past the identity line into the TLS stream.
    let mut reader = BufReader::with_capacity(1, stream);
    let line = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        read_line_limited(&mut reader, MAX_IDENTITY_LEN),
    )
    .await
    .context("identity timeout")??
    .context("connection closed before identity")?;
    identity_from_packet(&Packet::from_line(&line)?, proto)
}

/// v8: TCP client sends its identity, then becomes TLS **server**.
pub async fn dial_v8(
    local: &LocalIdentity,
    my_identity: &Packet,
    addr: SocketAddr,
    expect: PeerExpectation,
) -> Result<SecureStream> {
    let mut tcp = tcp_connect(addr).await?;
    tcp.write_all(&my_identity.to_line()).await?;
    let acceptor = TlsAcceptor::from(server_config(local, expect)?);
    let tls = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp))
        .await
        .context("tls timeout")??;
    Ok(TlsStream::Server(tls))
}

/// v8: identity already read from the TCP client; we become TLS **client**.
pub async fn accept_v8(
    local: &LocalIdentity,
    tcp: TcpStream,
    expect: PeerExpectation,
) -> Result<SecureStream> {
    tune_socket(&tcp);
    let ip = tcp.peer_addr()?.ip();
    let connector = TlsConnector::from(client_config(local, expect)?);
    let tls = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        connector.connect(ServerName::IpAddress(ip.into()), tcp),
    )
    .await
    .context("tls timeout")??;
    Ok(TlsStream::Client(tls))
}

// --- common ------------------------------------------------------------------------

pub fn peer_certificate(stream: &SecureStream) -> Option<Vec<u8>> {
    let (_, conn) = stream.get_ref();
    conn.peer_certificates()
        .and_then(|c| c.first())
        .map(|c| c.as_ref().to_vec())
}

/// Exchange identities over the encrypted channel.
pub async fn exchange_identity<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut BufReader<S>,
    my_identity: &Packet,
    proto: Protocol,
) -> Result<IdentityBody> {
    stream.get_mut().write_all(&my_identity.to_line()).await?;
    stream.get_mut().flush().await?;
    let line = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        read_line_limited(stream, MAX_IDENTITY_LEN),
    )
    .await
    .context("identity timeout")??
    .context("closed during identity exchange")?;
    identity_from_packet(&Packet::from_line(&line)?, proto)
}

/// Read one packet (with the limit for the current trust level) and map its
/// type to the canonical name. Returns `Ok(Some(None))` for packets to skip.
pub async fn read_packet<R: AsyncBufRead + Unpin>(
    r: &mut R,
    proto: Protocol,
    max: usize,
) -> Result<Option<Option<Packet>>> {
    let Some(line) = read_line_limited(r, max).await? else {
        return Ok(None);
    };
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(Some(None));
    }
    let mut p = Packet::from_line(&line)?;
    if proto.is_v8() {
        match interop::from_wire(proto, &p.packet_type) {
            Some(t) => {
                p.packet_type = t.to_owned();
                interop::body_in(proto, t, &mut p.body, konnekta_proto::packet::now_millis());
            }
            None => {
                tracing::debug!(packet = %p.packet_type, "ignoring {} packet without Konnekta equivalent", proto.label());
                return Ok(Some(None));
            }
        }
    }
    Ok(Some(Some(p)))
}

/// Map a Konnekta packet to the wire form of `proto` (None = not sendable).
pub fn to_wire(mut p: Packet, proto: Protocol) -> Option<Packet> {
    if proto.is_v8() {
        let wire = interop::to_wire(proto, &p.packet_type)?;
        interop::body_out(proto, &p.packet_type, &mut p.body);
        p.packet_type = wire;
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn line_limit() {
        let data = b"short\nthis line is definitely too long\n".to_vec();
        let mut r = BufReader::new(&data[..]);
        assert_eq!(
            read_line_limited(&mut r, 10).await.unwrap().unwrap(),
            b"short"
        );
        assert!(read_line_limited(&mut r, 10).await.is_err());
    }

    #[tokio::test]
    async fn eof_mid_line_is_error() {
        let data = b"partial".to_vec();
        let mut r = BufReader::new(&data[..]);
        assert!(read_line_limited(&mut r, 100).await.is_err());
        let empty: &[u8] = b"";
        let mut r = BufReader::new(empty);
        assert!(read_line_limited(&mut r, 100).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn kde_packets_are_translated() {
        let data = b"{\"id\":1,\"type\":\"kdeconnect.ping\",\"body\":{}}\n{\"id\":2,\"type\":\"kdeconnect.sftp\",\"body\":{}}\n".to_vec();
        let mut r = BufReader::new(&data[..]);
        let p = read_packet(&mut r, Protocol::KdeConnect, 1000)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(p.packet_type, konnekta_proto::types::PING);
        assert!(
            read_packet(&mut r, Protocol::KdeConnect, 1000)
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        assert!(
            to_wire(
                Packet::empty(konnekta_proto::types::DECK),
                Protocol::KdeConnect
            )
            .is_none()
        );
        assert_eq!(
            to_wire(
                Packet::empty(konnekta_proto::types::PING),
                Protocol::KdeConnect
            )
            .unwrap()
            .packet_type,
            "kdeconnect.ping"
        );
    }
}
