//! Out-of-band payload transfer over a dedicated mutually authenticated TLS
//! socket (KDE Connect compatible: the sender listens, the receiver connects
//! as TLS client; both pin the paired certificate).
//!
//! Konnekta extension (only when both peers advertise
//! [`RESUME_CAPABILITY`]): right after the handshake the receiver sends an
//! 8-byte big-endian offset and the sender skips that many bytes, allowing
//! interrupted transfers to resume.

use anyhow::{Context, Result, bail};
use konnekta_crypto::{LocalIdentity, PeerExpectation, client_config, server_config};
use konnekta_proto::{PAYLOAD_PORT_MAX, PAYLOAD_PORT_MIN};
use rustls_pki_types::ServerName;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub const RESUME_CAPABILITY: &str = "konnekta.share.resume";
pub const ACCEPT_TIMEOUT: Duration = Duration::from_secs(60);
const CHUNK: usize = 64 * 1024;

pub type Progress = Arc<dyn Fn(u64, u64) + Send + Sync>;

pub enum PayloadSource {
    File(PathBuf),
    Bytes(Vec<u8>),
}

impl PayloadSource {
    pub async fn size(&self) -> Result<u64> {
        Ok(match self {
            Self::File(p) => tokio::fs::metadata(p).await?.len(),
            Self::Bytes(b) => b.len() as u64,
        })
    }

    async fn open_at(&self, offset: u64) -> Result<Box<dyn AsyncRead + Send + Unpin>> {
        Ok(match self {
            Self::File(p) => {
                let mut f = tokio::fs::File::open(p).await?;
                f.seek(std::io::SeekFrom::Start(offset)).await?;
                Box::new(f)
            }
            Self::Bytes(b) => {
                let start = (offset as usize).min(b.len());
                Box::new(std::io::Cursor::new(b[start..].to_vec()))
            }
        })
    }
}

async fn bind_payload_port(bind: IpAddr) -> Result<TcpListener> {
    for port in PAYLOAD_PORT_MIN..=PAYLOAD_PORT_MAX {
        if let Ok(l) = TcpListener::bind(SocketAddr::new(bind, port)).await {
            return Ok(l);
        }
    }
    // Fallback to an ephemeral port (works between Konnekta peers).
    Ok(TcpListener::bind(SocketAddr::new(bind, 0)).await?)
}

async fn copy_exact<R, W>(
    r: &mut R,
    w: &mut W,
    len: u64,
    base: u64,
    total: u64,
    progress: &Option<Progress>,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    while done < len {
        let want = ((len - done) as usize).min(CHUNK);
        let n = r.read(&mut buf[..want]).await?;
        if n == 0 {
            bail!("payload truncated after {} of {} bytes", base + done, total);
        }
        w.write_all(&buf[..n]).await?;
        done += n as u64;
        if let Some(p) = progress {
            p(base + done, total);
        }
    }
    w.flush().await?;
    Ok(())
}

/// Start a one-shot payload server. Returns the port to announce and a task
/// that completes when the transfer finished (or failed / timed out).
#[allow(clippy::too_many_arguments)]
pub async fn serve(
    local: &LocalIdentity,
    peer_cert: Vec<u8>,
    peer_id: String,
    peer_ip: IpAddr,
    bind: IpAddr,
    source: PayloadSource,
    resume: bool,
    progress: Option<Progress>,
) -> Result<(u16, tokio::task::JoinHandle<Result<()>>)> {
    let listener = bind_payload_port(bind).await?;
    let port = listener.local_addr()?.port();
    let acceptor = TlsAcceptor::from(server_config(
        local,
        PeerExpectation::Pinned {
            device_id: peer_id,
            cert_der: peer_cert,
        },
    )?);
    let total = source.size().await?;
    let task = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + ACCEPT_TIMEOUT;
        let tcp = loop {
            let (tcp, from) = tokio::time::timeout_at(deadline, listener.accept())
                .await
                .context("payload receiver never connected")??;
            // Only the device the payload is meant for may connect.
            if from.ip() == peer_ip || peer_ip.is_unspecified() {
                break tcp;
            }
            tracing::warn!(%from, "rejected payload connection from unexpected address");
        };
        drop(listener);
        let _ = tcp.set_nodelay(true);
        let mut tls = tokio::time::timeout(crate::link::HANDSHAKE_TIMEOUT, acceptor.accept(tcp))
            .await
            .context("payload tls timeout")??;
        let offset = if resume {
            let o = tokio::time::timeout(crate::link::HANDSHAKE_TIMEOUT, tls.read_u64())
                .await
                .context("resume offset timeout")??;
            o.min(total)
        } else {
            0
        };
        let mut reader = source.open_at(offset).await?;
        copy_exact(
            &mut reader,
            &mut tls,
            total - offset,
            offset,
            total,
            &progress,
        )
        .await?;
        tls.shutdown().await.ok();
        Ok(())
    });
    Ok((port, task))
}

/// Download a payload announced by the peer.
#[allow(clippy::too_many_arguments)]
pub async fn fetch<W: AsyncWrite + Unpin>(
    local: &LocalIdentity,
    peer_cert: Vec<u8>,
    peer_id: String,
    addr: SocketAddr,
    total: u64,
    resume_from: Option<u64>,
    out: &mut W,
    progress: Option<Progress>,
) -> Result<()> {
    let tcp = tokio::time::timeout(crate::link::HANDSHAKE_TIMEOUT, TcpStream::connect(addr))
        .await
        .context("payload connect timeout")??;
    let _ = tcp.set_nodelay(true);
    let connector = TlsConnector::from(client_config(
        local,
        PeerExpectation::Pinned {
            device_id: peer_id,
            cert_der: peer_cert,
        },
    )?);
    let mut tls = tokio::time::timeout(
        crate::link::HANDSHAKE_TIMEOUT,
        connector.connect(ServerName::IpAddress(addr.ip().into()), tcp),
    )
    .await
    .context("payload tls timeout")??;
    let offset = match resume_from {
        Some(o) => {
            let o = o.min(total);
            tls.write_u64(o).await?;
            tls.flush().await?;
            o
        }
        None => 0,
    };
    copy_exact(&mut tls, out, total - offset, offset, total, &progress).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use konnekta_crypto::generate_device_id;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn transfer_and_resume() {
        konnekta_crypto::install_crypto_provider();
        let a = LocalIdentity::generate(&generate_device_id()).unwrap();
        let b = LocalIdentity::generate(&generate_device_id()).unwrap();
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let lo = IpAddr::V4(Ipv4Addr::LOCALHOST);

        // Full transfer a -> b.
        let (port, task) = serve(
            &a,
            b.cert_der.clone(),
            b.device_id.clone(),
            lo,
            lo,
            PayloadSource::Bytes(data.clone()),
            false,
            None,
        )
        .await
        .unwrap();
        let mut out = Vec::new();
        fetch(
            &b,
            a.cert_der.clone(),
            a.device_id.clone(),
            SocketAddr::new(lo, port),
            data.len() as u64,
            None,
            &mut out,
            None,
        )
        .await
        .unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(out, data);

        // Resume from 100k.
        let (port, task) = serve(
            &a,
            b.cert_der.clone(),
            b.device_id.clone(),
            lo,
            lo,
            PayloadSource::Bytes(data.clone()),
            true,
            None,
        )
        .await
        .unwrap();
        let mut out = data[..100_000].to_vec();
        fetch(
            &b,
            a.cert_der.clone(),
            a.device_id.clone(),
            SocketAddr::new(lo, port),
            data.len() as u64,
            Some(100_000),
            &mut out,
            None,
        )
        .await
        .unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(out, data);
    }

    #[tokio::test]
    async fn wrong_device_cannot_fetch() {
        konnekta_crypto::install_crypto_provider();
        let a = LocalIdentity::generate(&generate_device_id()).unwrap();
        let b = LocalIdentity::generate(&generate_device_id()).unwrap();
        let evil = LocalIdentity::generate(&b.device_id).unwrap();
        let lo = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let (port, task) = serve(
            &a,
            b.cert_der.clone(),
            b.device_id.clone(),
            lo,
            lo,
            PayloadSource::Bytes(vec![1; 1000]),
            false,
            None,
        )
        .await
        .unwrap();
        let mut out = Vec::new();
        let r = fetch(
            &evil,
            a.cert_der.clone(),
            a.device_id.clone(),
            SocketAddr::new(lo, port),
            1000,
            None,
            &mut out,
            None,
        )
        .await;
        assert!(r.is_err() || out.is_empty());
        assert!(task.await.unwrap().is_err());
    }
}
