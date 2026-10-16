//! Secure link establishment and framing.
//!
//! KDE Connect v8 handshake:
//! 1. the device that *received* a UDP identity opens a TCP connection and
//!    sends its identity in clear text (non-secret metadata only);
//! 2. TLS starts with **inverted roles**: the TCP client is the TLS server;
//! 3. both sides re-send their identity *inside* TLS and the inner identity is
//!    the one we trust (it must match the certificate CN).

use anyhow::{Context, Result, bail};
use konnekta_crypto::{LocalIdentity, PeerExpectation, client_config, server_config};
use konnekta_proto::{IdentityBody, MAX_PACKET_LEN, Packet, types};
use rustls_pki_types::ServerName;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Identity packets are small; anything bigger before authentication is abuse.
pub const MAX_IDENTITY_LEN: usize = 16 * 1024;

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

pub fn identity_from_packet(p: &Packet) -> Result<IdentityBody> {
    if p.packet_type != types::IDENTITY {
        bail!("expected identity packet, got {}", p.packet_type);
    }
    let body: IdentityBody = p.body_as()?;
    body.validate().map_err(anyhow::Error::msg)?;
    if body.protocol_version < 8 {
        bail!(
            "protocol version {} not supported (need >= 8)",
            body.protocol_version
        );
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

/// Read the clear-text identity a connecting peer sends before TLS.
pub async fn read_plain_identity(stream: &mut TcpStream) -> Result<IdentityBody> {
    // Read byte-wise through a small BufReader we then discard: the peer must
    // not send anything else before the TLS ClientHello it waits for.
    let mut reader = BufReader::with_capacity(1, stream);
    let line = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        read_line_limited(&mut reader, MAX_IDENTITY_LEN),
    )
    .await
    .context("identity timeout")??
    .context("connection closed before identity")?;
    identity_from_packet(&Packet::from_line(&line)?)
}

/// Outgoing connection: TCP client, sends identity, becomes TLS **server**.
pub async fn connect_outgoing(
    local: &LocalIdentity,
    my_identity: &Packet,
    addr: SocketAddr,
    expect: PeerExpectation,
) -> Result<SecureStream> {
    let mut tcp = tokio::time::timeout(HANDSHAKE_TIMEOUT, TcpStream::connect(addr))
        .await
        .context("connect timeout")??;
    tune_socket(&tcp);
    tcp.write_all(&my_identity.to_line()).await?;
    let acceptor = TlsAcceptor::from(server_config(local, expect)?);
    let tls = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp))
        .await
        .context("tls timeout")??;
    Ok(TlsStream::Server(tls))
}

/// Incoming connection (identity already read): we become TLS **client**.
pub async fn accept_incoming(
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

pub fn peer_certificate(stream: &SecureStream) -> Option<Vec<u8>> {
    let (_, conn) = stream.get_ref();
    conn.peer_certificates()
        .and_then(|c| c.first())
        .map(|c| c.as_ref().to_vec())
}

/// Protocol v8: exchange identities over the encrypted channel.
pub async fn exchange_identity<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut BufReader<S>,
    my_identity: &Packet,
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
    identity_from_packet(&Packet::from_line(&line)?)
}

/// Read packets from an established link.
pub async fn read_packet<R: AsyncBufRead + Unpin>(r: &mut R) -> Result<Option<Packet>> {
    match read_line_limited(r, MAX_PACKET_LEN).await? {
        None => Ok(None),
        Some(line) if line.iter().all(|b| b.is_ascii_whitespace()) => Ok(Some(Packet::empty(""))),
        Some(line) => Ok(Some(Packet::from_line(&line)?)),
    }
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
}
