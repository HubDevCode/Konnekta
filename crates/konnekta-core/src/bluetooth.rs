//! Links over Bluetooth (RFCOMM).
//!
//! One RFCOMM connection between the computer and the phone carries many
//! logical TCP-like streams (a small multiplexer), because BlueZ and Android
//! allow only one connection per device and service. The computer connects
//! to the phone's service ([`SERVICE_UUID`]); the phone's Kotlin code just
//! pipes its RFCOMM socket to a loopback port of the core ([`serve`]).
//!
//! Above the multiplexer nothing changes: the computer asks for a loopback
//! address that leads to a port of the phone ([`Session::forward`]) and dials
//! it like any other address, so TLS, pinning, pairing and permissions are
//! the same as on the network. Ports the computer listens on (payloads,
//! stream channels) are reached by the phone on its own loopback, where the
//! multiplexer opens a listener on request ([`Session::reverse`]).
//!
//! Frame: `kind u8 | stream u32 | length u16 | payload`. The computer opens
//! even stream numbers, the phone odd ones.

use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

/// Service the phone offers (Android `listenUsingRfcommWithServiceRecord`).
pub const SERVICE_UUID: &str = "6b6f6e6e-656b-7461-8000-4b4f4e4e454b";

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const CHUNK: usize = 4096;
const HEADER: usize = 7;
/// Frames waiting to be written, shared by every stream: when RFCOMM is
/// slow the streams wait here and take turns.
const OUT_QUEUE: usize = 32;
/// Chunks a stream may have waiting for its local socket.
const IN_QUEUE: usize = 64;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

mod kind {
    /// u16 port: connect to 127.0.0.1:port and attach the stream.
    pub const OPEN: u8 = 1;
    pub const DATA: u8 = 2;
    /// The sender will write no more (half close).
    pub const CLOSE: u8 = 3;
    /// u16 listen port, u16 target port: listen on the loopback and open
    /// each connection towards `target` on the other side.
    pub const LISTEN: u8 = 4;
    /// u16 listen port: stop listening.
    pub const UNLISTEN: u8 = 5;
    /// Answer to LISTEN (stream = request number): 1 done, 0 refused.
    pub const LISTENED: u8 = 6;
}

/// Ports the other side may ask us to connect to.
pub type OpenPolicy = Arc<dyn Fn(u16) -> bool + Send + Sync>;

/// What the phone lets the computer reach: the Konnekta listeners.
pub fn phone_policy() -> OpenPolicy {
    use konnekta_proto::Protocol;
    Arc::new(|port| {
        let p = Protocol::Konnekta;
        p.tcp_ports().contains(&port) || p.payload_ports().contains(&port)
    })
}

struct Frame {
    kind: u8,
    stream: u32,
    data: Vec<u8>,
}

struct Shared {
    out: mpsc::Sender<Frame>,
    streams: Mutex<HashMap<u32, mpsc::Sender<Option<Vec<u8>>>>>,
    next_stream: AtomicU32,
    next_request: AtomicU32,
    /// LISTEN requests waiting for an answer.
    pending: Mutex<HashMap<u32, tokio::sync::oneshot::Sender<bool>>>,
    /// Listeners we opened because the other side asked: port -> task.
    listeners: Mutex<HashMap<u16, AbortHandle>>,
    /// Target ports (on our side) the other side may open after our LISTEN.
    reversed: Mutex<HashSet<u16>>,
    policy: OpenPolicy,
    closed: AtomicBool,
}

/// A live multiplexed Bluetooth connection to one device.
#[derive(Clone)]
pub struct Session {
    shared: Arc<Shared>,
    /// Bluetooth address of the peer, when we connected to it.
    address: Option<Arc<str>>,
}

/// A mapping removed when dropped (same role as the adb ones).
pub struct Guard(Option<Box<dyn FnOnce() + Send>>);

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

impl Session {
    /// Run the multiplexer over `io`. `initiator` picks the stream numbers
    /// (the computer); `policy` says which of our ports the peer may open.
    pub fn start<T>(io: T, initiator: bool, policy: OpenPolicy) -> Session
    where
        T: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (out_tx, mut out_rx) = mpsc::channel::<Frame>(OUT_QUEUE);
        let shared = Arc::new(Shared {
            out: out_tx,
            streams: Mutex::default(),
            next_stream: AtomicU32::new(if initiator { 2 } else { 1 }),
            next_request: AtomicU32::new(1),
            pending: Mutex::default(),
            listeners: Mutex::default(),
            reversed: Mutex::default(),
            policy,
            closed: AtomicBool::new(false),
        });
        let (mut rd, mut wr) = tokio::io::split(io);

        let writer = tokio::spawn(async move {
            while let Some(f) = out_rx.recv().await {
                let mut buf = Vec::with_capacity(HEADER + f.data.len());
                buf.push(f.kind);
                buf.extend_from_slice(&f.stream.to_be_bytes());
                buf.extend_from_slice(&(f.data.len() as u16).to_be_bytes());
                buf.extend_from_slice(&f.data);
                if wr.write_all(&buf).await.is_err() || wr.flush().await.is_err() {
                    break;
                }
            }
        });

        let s = shared.clone();
        tokio::spawn(async move {
            let res = read_loop(&mut rd, &s).await;
            if let Err(e) = res {
                tracing::debug!("bluetooth session ended: {e:#}");
            }
            s.shutdown();
            writer.abort();
        });
        Session {
            shared,
            address: None,
        }
    }

    pub fn with_address(mut self, address: &str) -> Self {
        self.address = Some(address.into());
        self
    }

    pub fn address(&self) -> Option<&str> {
        self.address.as_deref()
    }

    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Relaxed)
    }

    /// A loopback address that leads to `remote` on the peer. Every
    /// connection to it opens a new stream.
    pub async fn forward(&self, remote: u16) -> Result<(SocketAddr, Guard)> {
        let listener = TcpListener::bind(SocketAddr::new(LOOPBACK, 0)).await?;
        let addr = listener.local_addr()?;
        let shared = self.shared.clone();
        let task = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let shared = shared.clone();
                tokio::spawn(async move { shared.open_to(remote, tcp).await });
            }
        });
        let abort = task.abort_handle();
        Ok((addr, Guard(Some(Box::new(move || abort.abort())))))
    }

    /// Make our `local` port reachable from the peer: it connects to the
    /// returned port on its own loopback.
    pub async fn reverse(&self, local: u16) -> Result<(u16, Guard)> {
        self.shared.reversed.lock().unwrap().insert(local);
        for _ in 0..8 {
            let port = 20000 + rand_below(40000) as u16;
            let req = self.shared.next_request.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = tokio::sync::oneshot::channel();
            self.shared.pending.lock().unwrap().insert(req, tx);
            let mut data = port.to_be_bytes().to_vec();
            data.extend_from_slice(&local.to_be_bytes());
            data.extend_from_slice(&req.to_be_bytes());
            self.shared.send(kind::LISTEN, 0, data).await;
            let ok = matches!(tokio::time::timeout(REPLY_TIMEOUT, rx).await, Ok(Ok(true)));
            self.shared.pending.lock().unwrap().remove(&req);
            if ok {
                let shared = self.shared.clone();
                let guard = Guard(Some(Box::new(move || {
                    shared.reversed.lock().unwrap().remove(&local);
                    let _ = shared.out.try_send(Frame {
                        kind: kind::UNLISTEN,
                        stream: 0,
                        data: port.to_be_bytes().to_vec(),
                    });
                })));
                return Ok((port, guard));
            }
        }
        self.shared.reversed.lock().unwrap().remove(&local);
        bail!("the phone could not open a port for us")
    }
}

fn rand_below(n: u32) -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(n);
    (h.finish() % n as u64) as u32
}

impl Shared {
    fn new_stream(&self) -> u32 {
        self.next_stream.fetch_add(2, Ordering::Relaxed)
    }

    async fn send(&self, kind: u8, stream: u32, data: Vec<u8>) {
        let _ = self.out.send(Frame { kind, stream, data }).await;
    }

    /// Open a stream towards `port` on the peer and pipe `tcp` to it. The
    /// stream is registered first: the peer may answer at once.
    async fn open_to(self: &Arc<Self>, port: u16, tcp: TcpStream) {
        let id = self.new_stream();
        let (tx, rx) = mpsc::channel(IN_QUEUE);
        self.streams.lock().unwrap().insert(id, tx);
        let sent = self
            .out
            .send(Frame {
                kind: kind::OPEN,
                stream: id,
                data: port.to_be_bytes().to_vec(),
            })
            .await;
        if sent.is_err() {
            self.streams.lock().unwrap().remove(&id);
            return;
        }
        self.attach(id, tcp, rx);
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::Relaxed);
        for (_, tx) in self.streams.lock().unwrap().drain() {
            let _ = tx.try_send(None);
        }
        for (_, h) in self.listeners.lock().unwrap().drain() {
            h.abort();
        }
        self.pending.lock().unwrap().clear();
    }

    /// The peer opened stream `id` towards our `port`.
    fn accept_open(self: &Arc<Self>, id: u32, port: u16) {
        let allowed = (self.policy)(port) || self.reversed.lock().unwrap().contains(&port);
        // Data may arrive before the socket is connected: queue it now.
        let (tx, rx) = mpsc::channel(IN_QUEUE);
        self.streams.lock().unwrap().insert(id, tx);
        let this = self.clone();
        tokio::spawn(async move {
            let tcp = if allowed {
                tokio::time::timeout(
                    CONNECT_TIMEOUT,
                    TcpStream::connect(SocketAddr::new(LOOPBACK, port)),
                )
                .await
                .ok()
                .and_then(Result::ok)
            } else {
                tracing::warn!(port, "bluetooth peer asked for a port it may not reach");
                None
            };
            match tcp {
                Some(tcp) => this.attach(id, tcp, rx),
                None => {
                    this.streams.lock().unwrap().remove(&id);
                    this.send(kind::CLOSE, id, Vec::new()).await;
                }
            }
        });
    }

    /// Pipe `tcp` to stream `id` in both directions; `inbox_rx` is what the
    /// peer sends on it.
    fn attach(
        self: &Arc<Self>,
        id: u32,
        tcp: TcpStream,
        mut inbox_rx: mpsc::Receiver<Option<Vec<u8>>>,
    ) {
        let _ = tcp.set_nodelay(true);
        let (mut rd, mut wr) = tcp.into_split();
        let this = self.clone();
        tokio::spawn(async move {
            while let Some(chunk) = inbox_rx.recv().await {
                match chunk {
                    Some(c) => {
                        if wr.write_all(&c).await.is_err() {
                            this.send(kind::CLOSE, id, Vec::new()).await;
                            break;
                        }
                    }
                    None => {
                        let _ = wr.shutdown().await;
                        break;
                    }
                }
            }
            this.streams.lock().unwrap().remove(&id);
        });
        let this = self.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; CHUNK];
            loop {
                match rd.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => this.send(kind::DATA, id, buf[..n].to_vec()).await,
                }
            }
            this.send(kind::CLOSE, id, Vec::new()).await;
        });
    }

    /// The peer asked us to listen on `listen` and open `target` on its side.
    async fn accept_listen(self: &Arc<Self>, req: u32, listen: u16, target: u16) {
        let bound = if self.listeners.lock().unwrap().contains_key(&listen) {
            None
        } else {
            TcpListener::bind(SocketAddr::new(LOOPBACK, listen))
                .await
                .ok()
        };
        let ok = bound.is_some();
        if let Some(listener) = bound {
            let this = self.clone();
            let task = tokio::spawn(async move {
                while let Ok((tcp, _)) = listener.accept().await {
                    let this = this.clone();
                    tokio::spawn(async move { this.open_to(target, tcp).await });
                }
            });
            self.listeners
                .lock()
                .unwrap()
                .insert(listen, task.abort_handle());
        }
        self.send(kind::LISTENED, req, vec![ok as u8]).await;
    }
}

async fn read_loop<R: AsyncRead + Unpin>(rd: &mut R, s: &Arc<Shared>) -> Result<()> {
    let mut head = [0u8; HEADER];
    loop {
        rd.read_exact(&mut head).await.context("bluetooth closed")?;
        let kind = head[0];
        let stream = u32::from_be_bytes([head[1], head[2], head[3], head[4]]);
        let len = u16::from_be_bytes([head[5], head[6]]) as usize;
        let mut data = vec![0u8; len];
        rd.read_exact(&mut data).await.context("bluetooth closed")?;
        let u16_at = |i: usize| data.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
        match kind {
            kind::OPEN => match u16_at(0) {
                Some(port) => s.accept_open(stream, port),
                None => bail!("malformed OPEN"),
            },
            kind::DATA | kind::CLOSE => {
                let tx = s.streams.lock().unwrap().get(&stream).cloned();
                if let Some(tx) = tx {
                    let item = (kind == kind::DATA).then_some(data);
                    // Waits when the local socket is slow: one slow stream
                    // holds the others back (no per-stream windows).
                    let _ = tx.send(item).await;
                }
            }
            kind::LISTEN => match (u16_at(0), u16_at(2), data.get(4..8)) {
                (Some(listen), Some(target), Some(r)) => {
                    let req = u32::from_be_bytes([r[0], r[1], r[2], r[3]]);
                    let s = s.clone();
                    tokio::spawn(async move { s.accept_listen(req, listen, target).await });
                }
                _ => bail!("malformed LISTEN"),
            },
            kind::UNLISTEN => {
                if let Some(port) = u16_at(0)
                    && let Some(h) = s.listeners.lock().unwrap().remove(&port)
                {
                    h.abort();
                }
            }
            kind::LISTENED => {
                if let Some(tx) = s.pending.lock().unwrap().remove(&stream) {
                    let _ = tx.send(data.first() == Some(&1));
                }
            }
            _ => {}
        }
    }
}

/// Phone side: accept RFCOMM bridges from the Android app on a loopback port
/// and run a multiplexer on each. Returns the port.
pub async fn serve(policy: OpenPolicy) -> Result<(u16, AbortHandle)> {
    let listener = TcpListener::bind(SocketAddr::new(LOOPBACK, 0)).await?;
    let port = listener.local_addr()?.port();
    let task = tokio::spawn(async move {
        while let Ok((tcp, from)) = listener.accept().await {
            if !from.ip().is_loopback() {
                continue;
            }
            let _ = tcp.set_nodelay(true);
            Session::start(tcp, false, policy.clone());
        }
    });
    Ok((port, task.abort_handle()))
}

/// Phones paired with this computer that may offer Konnekta over Bluetooth.
#[derive(Debug, Clone)]
pub struct Peer {
    pub address: String,
    pub name: String,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use bluer::rfcomm::{Profile, ProfileHandle, Role};
    use futures_util::StreamExt;

    pub struct Bluez {
        /// Keeps the D-Bus connection (and the registered profile) alive.
        _session: bluer::Session,
        adapter: bluer::Adapter,
        profile: tokio::sync::Mutex<ProfileHandle>,
        uuid: bluer::Uuid,
    }

    pub async fn open() -> Result<Bluez> {
        let session = bluer::Session::new().await?;
        let adapter = session.default_adapter().await?;
        if !adapter.is_powered().await.unwrap_or(false) {
            bail!("bluetooth adapter is off");
        }
        let uuid: bluer::Uuid = SERVICE_UUID.parse().context("uuid")?;
        let profile = session
            .register_profile(Profile {
                uuid,
                name: Some("Konnekta".into()),
                role: Some(Role::Client),
                require_authentication: Some(false),
                require_authorization: Some(false),
                auto_connect: Some(false),
                ..Default::default()
            })
            .await?;
        Ok(Bluez {
            _session: session,
            adapter,
            profile: tokio::sync::Mutex::new(profile),
            uuid,
        })
    }

    impl Bluez {
        /// Paired phones.
        pub async fn peers(&self) -> Vec<Peer> {
            let mut out = Vec::new();
            let Ok(addrs) = self.adapter.device_addresses().await else {
                return out;
            };
            for a in addrs {
                let Ok(d) = self.adapter.device(a) else {
                    continue;
                };
                if !d.is_paired().await.unwrap_or(false) {
                    continue;
                }
                let phone = d.icon().await.ok().flatten().as_deref() == Some("phone")
                    || d.class()
                        .await
                        .ok()
                        .flatten()
                        .is_some_and(|c| (c >> 8) & 0x1f == 2);
                if !phone {
                    continue;
                }
                out.push(Peer {
                    address: a.to_string(),
                    name: d.alias().await.unwrap_or_default(),
                });
            }
            out
        }

        /// Connect to the Konnekta service of `address` and start the
        /// multiplexer on it.
        pub async fn connect(&self, address: &str) -> Result<Session> {
            let addr: bluer::Address = address.parse().context("bluetooth address")?;
            let dev = self.adapter.device(addr)?;
            let mut profile = self.profile.lock().await;
            let connecting = dev.connect_profile(&self.uuid);
            let accepting = async {
                loop {
                    let req = tokio::time::timeout(CONNECT_TIMEOUT * 3, profile.next())
                        .await
                        .context("no bluetooth connection")?
                        .context("profile closed")?;
                    if req.device() == addr {
                        return Ok::<_, anyhow::Error>(req.accept()?);
                    }
                    req.reject(bluer::rfcomm::ReqError::Rejected);
                }
            };
            let (stream, connected) = tokio::join!(accepting, connecting);
            let stream = stream?;
            connected.context("the phone does not offer Konnekta over bluetooth")?;
            Ok(Session::start(stream, true, Arc::new(|_| false)).with_address(address))
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::Bluez;

/// Bluetooth is only driven from Linux computers for now.
#[cfg(not(target_os = "linux"))]
pub struct Bluez;

#[cfg(not(target_os = "linux"))]
impl Bluez {
    pub async fn peers(&self) -> Vec<Peer> {
        Vec::new()
    }
    pub async fn connect(&self, _address: &str) -> Result<Session> {
        bail!("bluetooth is not available on this system")
    }
}

#[cfg(not(target_os = "linux"))]
pub async fn open_bluez() -> Result<Bluez> {
    bail!("bluetooth is not available on this system")
}

#[cfg(target_os = "linux")]
pub async fn open_bluez() -> Result<Bluez> {
    linux::open().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    /// An echo server on the "phone" loopback, reached through two sessions
    /// joined by an in-memory pipe.
    #[tokio::test]
    async fn forward_reaches_a_phone_port() {
        let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let echo_port = echo.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = echo.accept().await {
                tokio::spawn(async move {
                    let (mut r, mut w) = s.split();
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
            }
        });
        let (a, b) = duplex(1 << 16);
        let pc = Session::start(a, true, Arc::new(|_| false));
        let _phone = Session::start(b, false, Arc::new(move |p| p == echo_port));

        let (addr, _g) = pc.forward(echo_port).await.unwrap();
        let mut c = TcpStream::connect(addr).await.unwrap();
        let payload: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        let w = payload.clone();
        let (mut r, mut wr) = c.split();
        let write = async {
            wr.write_all(&w).await.unwrap();
            wr.shutdown().await.unwrap();
        };
        let mut got = Vec::new();
        let read = async { r.read_to_end(&mut got).await.unwrap() };
        tokio::join!(write, read);
        assert_eq!(got, payload);
    }

    #[tokio::test]
    async fn reverse_lets_the_phone_reach_the_computer() {
        let svc = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let svc_port = svc.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = svc.accept().await {
                tokio::spawn(async move {
                    let _ = s.write_all(b"hello from the computer").await;
                });
            }
        });
        let (a, b) = duplex(1 << 16);
        let pc = Session::start(a, true, Arc::new(|_| false));
        let _phone = Session::start(b, false, Arc::new(|_| false));

        let (port, _g) = pc.reverse(svc_port).await.unwrap();
        let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut got = String::new();
        c.read_to_string(&mut got).await.unwrap();
        assert_eq!(got, "hello from the computer");
    }

    #[tokio::test]
    async fn ports_outside_the_policy_are_refused() {
        let secret = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let secret_port = secret.local_addr().unwrap().port();
        let hit = Arc::new(AtomicBool::new(false));
        let h = hit.clone();
        tokio::spawn(async move {
            if secret.accept().await.is_ok() {
                h.store(true, Ordering::Relaxed);
            }
        });
        let (a, b) = duplex(1 << 16);
        let pc = Session::start(a, true, Arc::new(|_| false));
        let _phone = Session::start(b, false, Arc::new(|_| false));

        let (addr, _g) = pc.forward(secret_port).await.unwrap();
        let mut c = TcpStream::connect(addr).await.unwrap();
        let mut buf = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(2), c.read_to_end(&mut buf)).await;
        assert!(!hit.load(Ordering::Relaxed));
    }
}
