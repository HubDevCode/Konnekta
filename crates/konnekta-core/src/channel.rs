//! Secure side channels for real-time streams (audio, video, desktop).
//!
//! Same security as payloads: dedicated TCP socket, mutual TLS, both sides
//! pinned to the paired certificate. The producer of the stream listens, the
//! consumer connects.

use crate::device::DeviceHandle;
use anyhow::{Context as _, Result, bail};
use konnekta_crypto::{PeerExpectation, client_config, server_config};
use rustls_pki_types::ServerName;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub trait Channel: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Channel for T {}

pub type SecureChannel = Box<dyn Channel>;

/// A channel together with the USB port mapping it travels through.
struct Mapped<T> {
    io: T,
    _usb: crate::usb::Mapping,
}

impl<T: AsyncRead + Unpin> AsyncRead for Mapped<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Mapped<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }
}

fn boxed<T: Channel + 'static>(io: T, usb: Option<crate::usb::Mapping>) -> SecureChannel {
    match usb {
        Some(m) => Box::new(Mapped { io, _usb: m }),
        None => Box::new(io),
    }
}

/// Bytes the kernel may hold for a stream before the sender waits. Small on
/// purpose: a big buffer on a slow network turns into seconds of delay; a
/// small one makes the producer skip stale pictures instead.
const STREAM_SEND_BUFFER: usize = 256 * 1024;

fn tune(tcp: &TcpStream) {
    let _ = tcp.set_nodelay(true);
    let _ = socket2::SockRef::from(tcp).set_send_buffer_size(STREAM_SEND_BUFFER);
}

impl DeviceHandle {
    fn expectation(&self) -> PeerExpectation {
        PeerExpectation::Pinned {
            device_id: self.id().to_owned(),
            cert_der: (*self.peer_cert).clone(),
        }
    }

    /// Listen for one secure channel from this device. Returns the port to
    /// announce and a future resolving to the channel.
    pub async fn listen_channel(
        &self,
    ) -> Result<(
        u16,
        impl std::future::Future<Output = Result<SecureChannel>> + Send + 'static,
    )> {
        let core = self.inner.upgrade().context("core stopped")?;
        if !self.is_paired() {
            bail!("device not paired");
        }
        let bind = core.config.read().unwrap().bind_addr;
        let listener =
            crate::payload::bind_payload_port(bind, self.protocol.payload_ports()).await?;
        let (port, mapping) = self.announced_port(listener.local_addr()?.port()).await?;
        let acceptor = TlsAcceptor::from(server_config(&core.identity, self.expectation())?);
        let peer_ip = self.addr.ip();
        let fut = async move {
            let deadline = tokio::time::Instant::now() + crate::payload::ACCEPT_TIMEOUT;
            loop {
                let (tcp, from) = tokio::time::timeout_at(deadline, listener.accept())
                    .await
                    .context("peer never connected to stream channel")??;
                if !crate::usb::same_host(from.ip(), peer_ip) {
                    tracing::warn!(%from, "rejected stream connection from unexpected address");
                    continue;
                }
                tune(&tcp);
                let tls =
                    tokio::time::timeout(crate::link::HANDSHAKE_TIMEOUT, acceptor.accept(tcp))
                        .await
                        .context("stream tls timeout")??;
                return Ok(boxed(tls, mapping));
            }
        };
        Ok((port, fut))
    }

    /// Connect to a channel announced by this device.
    pub async fn connect_channel(&self, port: u16) -> Result<SecureChannel> {
        let core = self.inner.upgrade().context("core stopped")?;
        if !self.is_paired() {
            bail!("device not paired");
        }
        let (addr, mapping) = self.peer_port(port).await?;
        let tcp = tokio::time::timeout(crate::link::HANDSHAKE_TIMEOUT, TcpStream::connect(addr))
            .await
            .context("stream connect timeout")??;
        tune(&tcp);
        let connector = TlsConnector::from(client_config(&core.identity, self.expectation())?);
        let tls = tokio::time::timeout(
            crate::link::HANDSHAKE_TIMEOUT,
            connector.connect(ServerName::IpAddress(addr.ip().into()), tcp),
        )
        .await
        .context("stream tls timeout")??;
        Ok(boxed(tls, mapping))
    }
}
