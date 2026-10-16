//! Secure side channels for real-time streams (audio, video, desktop).
//!
//! Same security as payloads: dedicated TCP socket, mutual TLS, both sides
//! pinned to the paired certificate. The producer of the stream listens, the
//! consumer connects.

use crate::device::DeviceHandle;
use anyhow::{Context, Result, bail};
use konnekta_crypto::{PeerExpectation, client_config, server_config};
use rustls_pki_types::ServerName;
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub trait Channel: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Channel for T {}

pub type SecureChannel = Box<dyn Channel>;

fn tune(tcp: &TcpStream) {
    let _ = tcp.set_nodelay(true);
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
        let port = listener.local_addr()?.port();
        let acceptor = TlsAcceptor::from(server_config(&core.identity, self.expectation())?);
        let peer_ip = self.addr.ip();
        let fut = async move {
            let deadline = tokio::time::Instant::now() + crate::payload::ACCEPT_TIMEOUT;
            loop {
                let (tcp, from) = tokio::time::timeout_at(deadline, listener.accept())
                    .await
                    .context("peer never connected to stream channel")??;
                if from.ip() != peer_ip {
                    tracing::warn!(%from, "rejected stream connection from unexpected address");
                    continue;
                }
                tune(&tcp);
                let tls =
                    tokio::time::timeout(crate::link::HANDSHAKE_TIMEOUT, acceptor.accept(tcp))
                        .await
                        .context("stream tls timeout")??;
                return Ok(Box::new(tls) as SecureChannel);
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
        let addr = SocketAddr::new(self.addr.ip(), port);
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
        Ok(Box::new(tls))
    }
}
