//! LocalSend compatibility (off by default, Impostazioni › LocalSend).
//!
//! Speaks the LocalSend protocol v2 next to Konnekta's own: devices running
//! the LocalSend app find this one (multicast on 224.0.0.167:53317 and the
//! HTTP `register` fallback), send it files and texts, and receive files
//! from it. LocalSend devices are never paired: every incoming transfer is
//! shown to the user, who accepts or refuses it (unless "ricezione rapida"
//! is on), and nothing else of Konnekta is reachable through this port.
//!
//! Events (plugin `localsend`, no device): `status`, `peers`, `request`,
//! `requestClosed`, `transfer`, `received`, `text`.

pub mod http;

use crate::store::JsonStore;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use http::{Body, Stream, Target};
use konnekta_core::proto::{DeviceType, Packet, packet::now_millis};
use konnekta_core::util::{sanitize_filename, unique_path};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo};
use konnekta_crypto::LocalIdentity;
use konnekta_platform::{DesktopNotification, Platform};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::oneshot;
use tokio::task::AbortHandle;

pub const DEFAULT_PORT: u16 = 53317;
const MULTICAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
const VERSION: &str = "2.1";
const API: &str = "/api/localsend/v2";
/// How long a transfer request waits for the user's answer.
const ASK_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LsConfig {
    pub enabled: bool,
    /// Name shown on LocalSend devices (empty: this device's name).
    pub alias: Option<String>,
    pub port: u16,
    /// Encrypt with TLS, as LocalSend does by default.
    pub https: bool,
    /// Accept transfers without asking ("ricezione rapida").
    pub auto_accept: bool,
    /// Folder for received files (empty: the default download folder).
    pub destination: Option<PathBuf>,
    /// PIN the sender must type (empty: none).
    pub pin: Option<String>,
}

impl Default for LsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            alias: None,
            port: DEFAULT_PORT,
            https: true,
            auto_accept: false,
            destination: None,
            pin: None,
        }
    }
}

/// Device description exchanged by LocalSend (announce, register, prepare).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireInfo {
    alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_type: Option<String>,
    #[serde(default)]
    fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    download: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    announce: Option<bool>,
    /// Protocol v1 name of `announce`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    announcement: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileMeta {
    id: String,
    file_name: String,
    size: u64,
    #[serde(default)]
    file_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

#[derive(Deserialize)]
struct PrepareRequest {
    info: WireInfo,
    files: HashMap<String, FileMeta>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareResponse {
    session_id: String,
    #[serde(default)]
    files: HashMap<String, String>,
}

/// A LocalSend device seen on the network.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    /// Its fingerprint.
    pub id: String,
    pub alias: String,
    pub device_model: Option<String>,
    /// mobile, desktop, web, headless, server.
    pub device_type: String,
    pub ip: String,
    pub port: u16,
    pub https: bool,
    #[serde(skip)]
    seen: Option<Instant>,
}

impl Peer {
    fn target(&self) -> Target {
        Target {
            host: self.ip.clone(),
            port: self.port,
            https: self.https,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LsFile {
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub path: Option<PathBuf>,
    pub done: bool,
}

/// One transfer to or from a LocalSend device, as the UI shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LsTransfer {
    pub id: String,
    /// incoming / outgoing
    pub direction: &'static str,
    pub peer: String,
    /// waiting, inProgress, completed, rejected, failed, cancelled
    pub state: &'static str,
    pub total: u64,
    pub transferred: u64,
    pub files: Vec<LsFile>,
    pub error: Option<String>,
    pub started_at: i64,
}

/// A running send: its task and, once accepted, the receiver's session.
type Outgoing = (AbortHandle, Option<(Target, String)>);

struct InFile {
    meta: FileMeta,
    token: String,
    done: bool,
}

/// The accepted incoming session (LocalSend allows one at a time).
struct InSession {
    id: String,
    ip: IpAddr,
    sender: String,
    files: HashMap<String, InFile>,
}

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunState {
    running: bool,
    error: Option<String>,
}

pub struct LocalSendPlugin {
    platform: Platform,
    me: Weak<LocalSendPlugin>,
    core: OnceLock<CoreHandle>,
    pub config: JsonStore<LsConfig>,
    identity: OnceLock<LocalIdentity>,
    peers: Mutex<HashMap<String, Peer>>,
    transfers: Mutex<Vec<LsTransfer>>,
    session: Mutex<Option<InSession>>,
    /// A request is waiting for the user: others get 409 meanwhile.
    asking: Mutex<Option<String>>,
    answers: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    /// Outgoing transfers: id -> (task, receiver and its session to cancel).
    outgoing: Mutex<HashMap<String, Outgoing>>,
    tasks: Mutex<Vec<AbortHandle>>,
    socket: Mutex<Option<Arc<UdpSocket>>>,
    run: Mutex<RunState>,
}

impl LocalSendPlugin {
    pub fn new(platform: Platform) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            platform,
            me: me.clone(),
            core: OnceLock::new(),
            config: JsonStore::default(),
            identity: OnceLock::new(),
            peers: Mutex::default(),
            transfers: Mutex::default(),
            session: Mutex::default(),
            asking: Mutex::default(),
            answers: Mutex::default(),
            outgoing: Mutex::default(),
            tasks: Mutex::default(),
            socket: Mutex::default(),
            run: Mutex::default(),
        })
    }

    fn emit(&self, kind: &str, data: Value) {
        if let Some(c) = self.core.get() {
            c.emit_plugin(None, "localsend", kind, data);
        }
    }

    fn alias(&self) -> String {
        self.config
            .with(|c| c.alias.clone())
            .map(|a| a.trim().to_owned())
            .filter(|a| !a.is_empty())
            .or_else(|| self.core.get().map(|c| c.local_name()))
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| "Konnekta".into())
    }

    fn fingerprint(&self) -> String {
        self.identity
            .get()
            .map(|i| hex::encode_upper(Sha256::digest(&i.cert_der)))
            .unwrap_or_default()
    }

    fn info(&self, announce: Option<bool>) -> WireInfo {
        let cfg = self.config.get();
        let mobile = self.core.get().is_some_and(|c| {
            matches!(
                c.local_device_type(),
                DeviceType::Phone | DeviceType::Tablet
            )
        });
        WireInfo {
            alias: self.alias(),
            version: Some(VERSION.into()),
            device_model: Some(os_name().into()),
            device_type: Some(if mobile { "mobile" } else { "desktop" }.into()),
            fingerprint: self.fingerprint(),
            port: Some(cfg.port),
            protocol: Some(if cfg.https { "https" } else { "http" }.into()),
            download: Some(false),
            announce,
            announcement: announce,
        }
    }

    fn destination(&self) -> PathBuf {
        self.config
            .with(|c| c.destination.clone())
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| {
                self.core
                    .get()
                    .map(|c| c.download_dir())
                    .unwrap_or_else(|| PathBuf::from("."))
            })
    }

    /// Our own certificate for the HTTPS server, made once and kept.
    fn load_identity(&self, core: &CoreHandle) {
        const CN: &str = "konnekta-localsend";
        let dir = core.plugin_data_dir("localsend");
        let saved = dir.as_ref().and_then(|d| {
            let cert = std::fs::read(d.join("cert.der")).ok()?;
            let key = std::fs::read(d.join("key.der")).ok()?;
            LocalIdentity::from_parts(CN.into(), cert, key).ok()
        });
        let id = match saved {
            Some(i) => i,
            None => match LocalIdentity::generate(CN) {
                Ok(i) => {
                    if let Some(d) = &dir {
                        let _ =
                            konnekta_crypto::write_private_file(&d.join("cert.der"), &i.cert_der);
                        let _ =
                            konnekta_crypto::write_private_file(&d.join("key.der"), i.key_der());
                    }
                    i
                }
                Err(e) => {
                    tracing::error!("localsend: cannot create a certificate: {e}");
                    return;
                }
            },
        };
        let _ = self.identity.set(id);
    }

    // --- state the UI shows ----------------------------------------------------

    fn status(&self) -> Value {
        let cfg = self.config.get();
        let run = self.run.lock().unwrap().clone();
        json!({
            "enabled": cfg.enabled,
            "running": run.running,
            "error": run.error,
            "alias": self.alias(),
            "fingerprint": self.fingerprint(),
            "port": cfg.port,
            "https": cfg.https,
            "autoAccept": cfg.auto_accept,
            "pin": cfg.pin.as_deref().is_some_and(|p| !p.is_empty()),
            "ips": local_ipv4().iter().map(|ip| ip.to_string()).collect::<Vec<_>>(),
            "destination": self.destination(),
        })
    }

    fn set_run(&self, running: bool, error: Option<String>) {
        *self.run.lock().unwrap() = RunState { running, error };
        self.emit("status", self.status());
    }

    fn peer_list(&self) -> Vec<Peer> {
        let mut l: Vec<Peer> = self.peers.lock().unwrap().values().cloned().collect();
        l.sort_by_key(|p| p.alias.to_lowercase());
        l
    }

    fn emit_peers(&self) {
        self.emit("peers", json!({ "peers": self.peer_list() }));
    }

    fn add_peer(&self, info: &WireInfo, ip: IpAddr, fallback: Option<&Target>) -> Option<Peer> {
        if info.fingerprint.is_empty() || info.fingerprint == self.fingerprint() {
            return None;
        }
        let https = match info.protocol.as_deref() {
            Some(p) => p == "https",
            None => fallback.is_none_or(|t| t.https),
        };
        let peer = Peer {
            id: info.fingerprint.chars().take(128).collect(),
            alias: clean(&info.alias, 64).unwrap_or_else(|| "LocalSend".into()),
            device_model: info.device_model.as_deref().and_then(|m| clean(m, 64)),
            device_type: info
                .device_type
                .as_deref()
                .and_then(|t| clean(t, 16))
                .unwrap_or_else(|| "desktop".into()),
            ip: ip.to_string(),
            port: info
                .port
                .or(fallback.map(|t| t.port))
                .unwrap_or(DEFAULT_PORT),
            https,
            seen: Some(Instant::now()),
        };
        let changed = {
            let mut m = self.peers.lock().unwrap();
            m.retain(|_, p| {
                p.seen
                    .is_some_and(|s| s.elapsed() < Duration::from_secs(600))
            });
            let old = m.insert(peer.id.clone(), peer.clone());
            old.is_none_or(|o| o.alias != peer.alias || o.ip != peer.ip || o.port != peer.port)
        };
        if changed {
            self.emit_peers();
        }
        Some(peer)
    }

    fn put_transfer(&self, t: LsTransfer) {
        {
            let mut l = self.transfers.lock().unwrap();
            match l.iter_mut().find(|x| x.id == t.id) {
                Some(x) => *x = t.clone(),
                None => l.push(t.clone()),
            }
            if l.len() > 100 {
                l.remove(0);
            }
        }
        self.emit("transfer", serde_json::to_value(&t).unwrap_or_default());
    }

    fn update_transfer(&self, id: &str, f: impl FnOnce(&mut LsTransfer)) -> Option<LsTransfer> {
        let t = {
            let mut l = self.transfers.lock().unwrap();
            let t = l.iter_mut().find(|t| t.id == id)?;
            f(t);
            t.clone()
        };
        self.emit("transfer", serde_json::to_value(&t).unwrap_or_default());
        Some(t)
    }

    // --- lifecycle ---------------------------------------------------------------

    /// Stop the server and discovery, and start them again if enabled.
    fn restart(&self) {
        for t in self.tasks.lock().unwrap().drain(..) {
            t.abort();
        }
        *self.socket.lock().unwrap() = None;
        self.peers.lock().unwrap().clear();
        self.emit_peers();
        let Some(me) = self.me.upgrade() else { return };
        let cfg = self.config.get();
        if !cfg.enabled || self.identity.get().is_none() {
            self.set_run(false, None);
            return;
        }
        let task = tokio::spawn(async move { me.run(cfg).await });
        self.tasks.lock().unwrap().push(task.abort_handle());
    }

    async fn run(self: Arc<Self>, cfg: LsConfig) {
        let listener = match TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, cfg.port)))
            .await
        {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("localsend: cannot listen on port {}: {e}", cfg.port);
                self.set_run(
                    false,
                    Some(format!(
                        "La porta {} è già in uso (forse l'app LocalSend è aperta su questo dispositivo)",
                        cfg.port
                    )),
                );
                return;
            }
        };
        let tls = if cfg.https {
            match self.identity.get().map(konnekta_crypto::open_server_config) {
                Some(Ok(c)) => Some(tokio_rustls::TlsAcceptor::from(c)),
                _ => {
                    self.set_run(false, Some("Certificato HTTPS non disponibile".into()));
                    return;
                }
            }
        } else {
            None
        };
        let sock = match bind_multicast(cfg.port) {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => {
                // Still reachable by address and by the register scan.
                tracing::warn!("localsend: multicast unavailable: {e:#}");
                None
            }
        };
        *self.socket.lock().unwrap() = sock.clone();
        self.set_run(true, None);
        if let Some(sock) = sock {
            let me = self.clone();
            let s2 = sock.clone();
            let listen = tokio::spawn(async move { me.listen(s2).await });
            self.tasks.lock().unwrap().push(listen.abort_handle());
            let me = self.clone();
            let announce = tokio::spawn(async move {
                for wait in [100, 500, 2000] {
                    tokio::time::sleep(Duration::from_millis(wait)).await;
                    me.announce(&sock).await;
                }
            });
            self.tasks.lock().unwrap().push(announce.abort_handle());
        }
        self.serve(listener, tls).await;
    }

    // --- discovery ---------------------------------------------------------------

    async fn announce(&self, sock: &UdpSocket) {
        let port = self.config.with(|c| c.port);
        let msg = serde_json::to_vec(&self.info(Some(true))).unwrap_or_default();
        send_multicast(sock, &msg, port).await;
    }

    async fn listen(self: Arc<Self>, sock: Arc<UdpSocket>) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            };
            let Ok(info) = serde_json::from_slice::<WireInfo>(&buf[..n]) else {
                continue;
            };
            let Some(peer) = self.add_peer(&info, from.ip(), None) else {
                continue;
            };
            if info.announce.or(info.announcement) == Some(true) {
                // Answer by HTTP; multicast if the device cannot be reached.
                let me = self.clone();
                let sock = sock.clone();
                tokio::spawn(async move {
                    let body = serde_json::to_value(me.info(None)).unwrap_or_default();
                    let ok = http::request(
                        &peer.target(),
                        "POST",
                        &format!("{API}/register"),
                        Some(&body),
                        Duration::from_secs(3),
                    )
                    .await
                    .is_ok_and(|(s, _)| s == 200);
                    if !ok {
                        let msg = serde_json::to_vec(&me.info(Some(false))).unwrap_or_default();
                        let port = me.config.with(|c| c.port);
                        send_multicast(&sock, &msg, port).await;
                    }
                });
            }
        }
    }

    /// Ask one address for its info (`register`), as LocalSend's scan does.
    async fn register_to(&self, t: &Target, wait: Duration) -> Result<Peer> {
        let body = serde_json::to_value(self.info(None))?;
        let (status, data) =
            http::request(t, "POST", &format!("{API}/register"), Some(&body), wait).await?;
        if status != 200 {
            bail!("answer {status}");
        }
        let info: WireInfo = serde_json::from_slice(&data).context("not a LocalSend device")?;
        let ip: IpAddr = t.host.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        // Where we reached it is where it listens.
        let mut info = info;
        info.port = Some(t.port);
        info.protocol = Some(if t.https { "https" } else { "http" }.into());
        self.add_peer(&info, ip, Some(t))
            .context("this is the device itself")
    }

    /// Multicast announce, then every address of the local /24 networks
    /// (multicast is often blocked on hotspots and some routers).
    async fn scan(self: Arc<Self>) {
        let sock = self.socket.lock().unwrap().clone();
        if let Some(s) = &sock {
            self.announce(s).await;
        }
        let (port, https) = self.config.with(|c| (c.port, c.https));
        let limit = Arc::new(tokio::sync::Semaphore::new(48));
        let mut set = tokio::task::JoinSet::new();
        for ip in local_ipv4() {
            if !ip.is_private() {
                continue;
            }
            let o = ip.octets();
            for last in 1..255u8 {
                if last == o[3] {
                    continue;
                }
                let host = Ipv4Addr::new(o[0], o[1], o[2], last).to_string();
                let me = self.clone();
                let limit = limit.clone();
                set.spawn(async move {
                    let _permit = limit.acquire().await;
                    let t = Target { host, port, https };
                    let _ = me.register_to(&t, Duration::from_millis(1500)).await;
                });
            }
        }
        while set.join_next().await.is_some() {}
        self.emit("scanDone", json!({}));
    }

    // --- server ------------------------------------------------------------------

    async fn serve(self: Arc<Self>, listener: TcpListener, tls: Option<tokio_rustls::TlsAcceptor>) {
        let busy = Arc::new(tokio::sync::Semaphore::new(64));
        loop {
            let Ok((tcp, from)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
            let Ok(permit) = busy.clone().try_acquire_owned() else {
                continue;
            };
            let me = self.clone();
            let tls = tls.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let io: Box<dyn http::Io> = match tls {
                    Some(a) => {
                        match tokio::time::timeout(Duration::from_secs(10), a.accept(tcp)).await {
                            Ok(Ok(s)) => Box::new(s),
                            _ => return,
                        }
                    }
                    None => Box::new(tcp),
                };
                let mut s = BufReader::new(io);
                if let Err(e) = me.handle_http(&mut s, from.ip()).await {
                    tracing::debug!(%from, "localsend request failed: {e:#}");
                }
            });
        }
    }

    async fn handle_http(&self, s: &mut Stream, ip: IpAddr) -> Result<()> {
        let Some(head) =
            tokio::time::timeout(Duration::from_secs(15), http::read_request(s)).await??
        else {
            return Ok(());
        };
        let path = head.path().to_owned();
        let api = path.starts_with("/api/localsend/");
        match head.method.as_str() {
            "POST" if api && path.ends_with("/register") => {
                let raw = Body::new(s, &head).to_vec(64 * 1024).await?;
                let info: WireInfo = serde_json::from_slice(&raw).context("bad register body")?;
                self.add_peer(&info, ip, None);
                http::respond(s, 200, Some(&serde_json::to_value(self.info(None))?)).await
            }
            "GET" if api && path.ends_with("/info") => {
                http::respond(s, 200, Some(&serde_json::to_value(self.info(None))?)).await
            }
            "POST" if path == format!("{API}/prepare-upload") => {
                self.prepare_upload(s, &head, ip).await
            }
            "POST" if path == format!("{API}/upload") => self.upload(s, &head, ip).await,
            "POST" if path == format!("{API}/cancel") => {
                let sid = head.query("sessionId").unwrap_or_default();
                let gone = {
                    let mut sess = self.session.lock().unwrap();
                    match sess.as_ref() {
                        Some(x) if x.id == sid && x.ip == ip => sess.take(),
                        _ => None,
                    }
                };
                if let Some(x) = gone {
                    self.update_transfer(&x.id, |t| {
                        if t.state != "completed" {
                            t.state = "cancelled";
                            t.error = Some("annullato da chi inviava".into());
                        }
                    });
                }
                http::respond(s, 200, None).await
            }
            _ => http::respond(s, 404, None).await,
        }
    }

    async fn prepare_upload(&self, s: &mut Stream, head: &http::Head, ip: IpAddr) -> Result<()> {
        let cfg = self.config.get();
        if let Some(pin) = cfg.pin.as_deref().filter(|p| !p.is_empty())
            && head.query("pin").as_deref() != Some(pin)
        {
            return http::respond(s, 401, Some(&json!({ "message": "PIN required" }))).await;
        }
        let raw = Body::new(s, head).to_vec(4 << 20).await?;
        let Ok(req) = serde_json::from_slice::<PrepareRequest>(&raw) else {
            return http::respond(s, 400, Some(&json!({ "message": "invalid body" }))).await;
        };
        if req.files.is_empty() {
            return http::respond(s, 400, Some(&json!({ "message": "no files" }))).await;
        }
        let peer = self.add_peer(&req.info, ip, None);
        let from = peer
            .as_ref()
            .map(|p| p.alias.clone())
            .or_else(|| clean(&req.info.alias, 64))
            .unwrap_or_else(|| "LocalSend".into());

        // A message: LocalSend sends it as one text "file" with a preview.
        if req.files.len() == 1
            && let Some(f) = req.files.values().next()
            && f.file_type
                .as_deref()
                .is_some_and(|t| t.starts_with("text/"))
            && let Some(text) = f.preview.as_deref()
        {
            let text: String = text.chars().take(1_000_000).collect();
            self.emit("text", json!({ "from": from, "text": text }));
            let _ = self.platform.notifier.show(
                DesktopNotification {
                    id: "localsend-text".into(),
                    app_name: from.clone(),
                    title: "Messaggio da LocalSend".into(),
                    body: text.chars().take(200).collect(),
                    ..Default::default()
                },
                None,
            );
            return http::respond(s, 204, None).await;
        }

        let busy = self.session.lock().unwrap().is_some() || self.asking.lock().unwrap().is_some();
        if busy {
            return http::respond(
                s,
                409,
                Some(&json!({ "message": "blocked by another session" })),
            )
            .await;
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut metas: Vec<FileMeta> = req.files.into_values().collect();
        metas.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        let files: Vec<LsFile> = metas
            .iter()
            .map(|m| LsFile {
                name: sanitize_filename(&m.file_name),
                size: m.size,
                mime: m
                    .file_type
                    .clone()
                    .unwrap_or_else(|| mime_of(&m.file_name).into()),
                path: None,
                done: false,
            })
            .collect();
        let total = metas.iter().map(|m| m.size).sum();

        if !cfg.auto_accept {
            let (tx, rx) = oneshot::channel();
            *self.asking.lock().unwrap() = Some(id.clone());
            self.answers.lock().unwrap().insert(id.clone(), tx);
            self.emit(
                "request",
                json!({
                    "id": id,
                    "from": from,
                    "deviceModel": peer.as_ref().and_then(|p| p.device_model.clone()),
                    "deviceType": peer.as_ref().map(|p| p.device_type.clone()),
                    "ip": ip.to_string(),
                    "files": files,
                    "total": total,
                }),
            );
            self.notify_request(&id, &from, &files);
            let answer = tokio::time::timeout(ASK_TIMEOUT, rx).await;
            self.answers.lock().unwrap().remove(&id);
            *self.asking.lock().unwrap() = None;
            self.emit("requestClosed", json!({ "id": id }));
            if !matches!(answer, Ok(Ok(true))) {
                return http::respond(s, 403, Some(&json!({ "message": "rejected" }))).await;
            }
        }

        let mut tokens = serde_json::Map::new();
        let mut map = HashMap::new();
        for m in metas {
            let token = uuid::Uuid::new_v4().simple().to_string();
            tokens.insert(m.id.clone(), json!(token));
            map.insert(
                m.id.clone(),
                InFile {
                    meta: m,
                    token,
                    done: false,
                },
            );
        }
        *self.session.lock().unwrap() = Some(InSession {
            id: id.clone(),
            ip,
            sender: from.clone(),
            files: map,
        });
        self.put_transfer(LsTransfer {
            id: id.clone(),
            direction: "incoming",
            peer: from,
            state: "inProgress",
            total,
            transferred: 0,
            files,
            error: None,
            started_at: now_millis(),
        });
        http::respond(s, 200, Some(&json!({ "sessionId": id, "files": tokens }))).await
    }

    fn notify_request(&self, id: &str, from: &str, files: &[LsFile]) {
        let body = match files {
            [one] => format!("{} ({})", one.name, human(one.size)),
            many => format!(
                "{} file ({})",
                many.len(),
                human(many.iter().map(|f| f.size).sum())
            ),
        };
        let me = self.me.clone();
        let id = id.to_owned();
        let _ = self.platform.notifier.show(
            DesktopNotification {
                id: format!("localsend-ask-{id}"),
                app_name: from.into(),
                title: "Vuole inviarti dei file (LocalSend)".into(),
                body,
                actions: vec![
                    ("accept".into(), "Accetta".into()),
                    ("reject".into(), "Rifiuta".into()),
                ],
                ..Default::default()
            },
            Some(Box::new(move |a| {
                if let Some(me) = me.upgrade() {
                    me.answer(&id, a == "accept");
                }
            })),
        );
    }

    fn answer(&self, id: &str, accept: bool) -> bool {
        match self.answers.lock().unwrap().remove(id) {
            Some(tx) => tx.send(accept).is_ok(),
            None => false,
        }
    }

    async fn upload(&self, s: &mut Stream, head: &http::Head, ip: IpAddr) -> Result<()> {
        let (sid, fid, token) = match (
            head.query("sessionId"),
            head.query("fileId"),
            head.query("token"),
        ) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => {
                return http::respond(s, 400, Some(&json!({ "message": "missing parameters" })))
                    .await;
            }
        };
        let meta = {
            let sess = self.session.lock().unwrap();
            match sess.as_ref() {
                Some(x) if x.id == sid && x.ip == ip => match x.files.get(&fid) {
                    Some(f) if f.token == token && !f.done => {
                        Some((f.meta.clone(), x.sender.clone()))
                    }
                    _ => None,
                },
                Some(_) => None,
                None => None,
            }
        };
        let Some((meta, from)) = meta else {
            return http::respond(
                s,
                403,
                Some(&json!({ "message": "invalid token or session" })),
            )
            .await;
        };
        let dir = self.destination();
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("cannot create {}", dir.display()))?;
        let name = sanitize_filename(&meta.file_name);
        let part = unique_path(&dir, &format!(".{name}.part"));
        let mut file = tokio::fs::File::create(&part).await?;
        let mut body = Body::new(s, head);
        let mut buf = vec![0u8; 256 * 1024];
        let mut got = 0u64;
        let mut hasher = meta.sha256.as_ref().map(|_| Sha256::new());
        let mut last = Instant::now();
        let base = self
            .transfers
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.id == sid)
            .map(|t| t.transferred)
            .unwrap_or(0);
        let r: Result<()> = async {
            loop {
                let n = tokio::time::timeout(Duration::from_secs(60), body.read(&mut buf))
                    .await
                    .context("the sender stopped")??;
                if n == 0 {
                    break;
                }
                got += n as u64;
                if got > meta.size {
                    bail!("more data than announced");
                }
                if let Some(h) = &mut hasher {
                    h.update(&buf[..n]);
                }
                file.write_all(&buf[..n]).await?;
                if last.elapsed() > Duration::from_millis(250) {
                    last = Instant::now();
                    // Other files of the session may be arriving too.
                    let now = base + got;
                    self.update_transfer(&sid, |t| t.transferred = t.transferred.max(now));
                }
            }
            file.flush().await?;
            if got != meta.size {
                bail!("file incomplete ({got} of {} bytes)", meta.size);
            }
            Ok(())
        }
        .await;
        drop(file);
        if let Err(e) = r {
            let _ = tokio::fs::remove_file(&part).await;
            self.fail_incoming(&sid, &e.to_string());
            return Err(e);
        }
        if let (Some(h), Some(expected)) = (hasher, meta.sha256.as_deref())
            && !hex::encode(h.finalize()).eq_ignore_ascii_case(expected)
        {
            let _ = tokio::fs::remove_file(&part).await;
            self.fail_incoming(&sid, "controllo di integrità non riuscito");
            return http::respond(s, 422, Some(&json!({ "message": "checksum mismatch" }))).await;
        }
        let dest = unique_path(&dir, &name);
        tokio::fs::rename(&part, &dest).await?;
        if let Some(ms) = meta
            .metadata
            .as_ref()
            .and_then(|m| m.get("modified"))
            .and_then(Value::as_str)
            .and_then(parse_rfc3339_ms)
            && let Ok(f) = std::fs::File::options().write(true).open(&dest)
        {
            let _ = f.set_modified(std::time::UNIX_EPOCH + Duration::from_millis(ms));
        }
        let mime = meta
            .file_type
            .clone()
            .filter(|t| t.contains('/'))
            .unwrap_or_else(|| mime_of(&name).into());
        let finished = {
            let mut sess = self.session.lock().unwrap();
            let all = match sess.as_mut() {
                Some(x) if x.id == sid => {
                    if let Some(f) = x.files.get_mut(&fid) {
                        f.done = true;
                    }
                    x.files.values().all(|f| f.done)
                }
                _ => false,
            };
            if all {
                *sess = None;
            }
            all
        };
        let shown = dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(name.clone());
        let t = self.update_transfer(&sid, |t| {
            if let Some(f) = t.files.iter_mut().find(|f| f.name == name && !f.done) {
                f.done = true;
                f.path = Some(dest.clone());
                f.name = shown.clone();
            }
            t.transferred = t.files.iter().filter(|f| f.done).map(|f| f.size).sum();
            if finished {
                t.state = "completed";
                t.transferred = t.total;
            }
        });
        self.emit(
            "received",
            json!({ "transferId": sid, "from": from, "name": shown, "path": dest, "mime": mime, "size": meta.size }),
        );
        if finished && let Some(t) = t {
            self.notify_received(&t);
        }
        http::respond(s, 200, None).await
    }

    fn fail_incoming(&self, sid: &str, error: &str) {
        let mut sess = self.session.lock().unwrap();
        if sess.as_ref().is_some_and(|x| x.id == sid) {
            *sess = None;
        }
        drop(sess);
        self.update_transfer(sid, |t| {
            t.state = "failed";
            t.error = Some(error.into());
        });
    }

    fn notify_received(&self, t: &LsTransfer) {
        let files: Vec<&LsFile> = t.files.iter().filter(|f| f.path.is_some()).collect();
        let (body, target) = match files.as_slice() {
            [one] => (one.name.clone(), one.path.clone()),
            many => (format!("{} file", many.len()), None),
        };
        let folder = self.destination();
        let launcher = self.platform.launcher.clone();
        let mut actions = vec![];
        if target.is_some() {
            actions.push(("default".to_string(), "Apri".to_string()));
            actions.push(("open".to_string(), "Apri".to_string()));
        }
        actions.push(("folder".to_string(), "Mostra nella cartella".to_string()));
        let _ = self.platform.notifier.show(
            DesktopNotification {
                id: format!("localsend-done-{}", t.id),
                app_name: t.peer.clone(),
                title: "File ricevuti (LocalSend)".into(),
                body,
                actions,
                ..Default::default()
            },
            Some(Box::new(move |a| {
                let what = match a.as_str() {
                    "open" | "default" => target.clone(),
                    "folder" => Some(folder.clone()),
                    _ => None,
                };
                if let Some(p) = what
                    && let Ok(h) = tokio::runtime::Handle::try_current()
                {
                    h.spawn(async move {
                        let _ = launcher.open(&p.to_string_lossy()).await;
                    });
                }
            })),
        );
    }

    // --- sending -----------------------------------------------------------------

    async fn send(
        self: Arc<Self>,
        tid: String,
        peer: Peer,
        items: Vec<(PathBuf, FileMeta)>,
        pin: Option<String>,
    ) -> Result<Value> {
        let t = peer.target();
        let files: serde_json::Map<String, Value> = items
            .iter()
            .map(|(_, m)| (m.id.clone(), serde_json::to_value(m).unwrap_or_default()))
            .collect();
        let body = json!({ "info": self.info(None), "files": files });
        let q = pin
            .filter(|p| !p.is_empty())
            .map(|p| format!("?pin={}", http::encode(&p)))
            .unwrap_or_default();
        let (status, data) = http::request(
            &t,
            "POST",
            &format!("{API}/prepare-upload{q}"),
            Some(&body),
            Duration::from_secs(600),
        )
        .await?;
        match status {
            200 => {}
            204 => {
                self.update_transfer(&tid, |x| {
                    x.state = "completed";
                    x.transferred = x.total;
                    x.files.iter_mut().for_each(|f| f.done = true);
                });
                return Ok(json!({ "state": "completed" }));
            }
            401 => {
                self.update_transfer(&tid, |x| {
                    x.state = "failed";
                    x.error = Some("serve il PIN".into());
                });
                bail!("PIN richiesto: {} chiede un PIN", peer.alias)
            }
            403 => {
                self.update_transfer(&tid, |x| x.state = "rejected");
                bail!("{} ha rifiutato", peer.alias)
            }
            409 => bail!("{} sta già ricevendo altri file", peer.alias),
            429 => bail!("troppe richieste: riprova tra poco"),
            s => bail!("{} ha risposto con un errore ({s})", peer.alias),
        }
        let r: PrepareResponse = serde_json::from_slice(&data).context("risposta non valida")?;
        if let Some(e) = self.outgoing.lock().unwrap().get_mut(&tid) {
            e.1 = Some((t.clone(), r.session_id.clone()));
        }
        self.update_transfer(&tid, |x| x.state = "inProgress");
        let mut done_bytes = 0u64;
        for (i, (path, meta)) in items.iter().enumerate() {
            let Some(token) = r.files.get(&meta.id) else {
                continue; // the receiver skipped this file
            };
            let q = format!(
                "{API}/upload?sessionId={}&fileId={}&token={}",
                http::encode(&r.session_id),
                http::encode(&meta.id),
                http::encode(token)
            );
            let me = self.clone();
            let id = tid.clone();
            let mut last = Instant::now();
            let base = done_bytes;
            let (status, _) = http::upload(&t, &q, path, move |sent| {
                if last.elapsed() > Duration::from_millis(250) {
                    last = Instant::now();
                    me.update_transfer(&id, |x| x.transferred = base + sent);
                }
            })
            .await?;
            if status != 200 {
                bail!("{} ha rifiutato {} ({status})", peer.alias, meta.file_name);
            }
            done_bytes += meta.size;
            self.update_transfer(&tid, |x| {
                x.transferred = done_bytes;
                if let Some(f) = x.files.get_mut(i) {
                    f.done = true;
                }
            });
        }
        self.update_transfer(&tid, |x| {
            x.state = "completed";
            x.transferred = x.total;
        });
        Ok(json!({ "state": "completed" }))
    }

    async fn start_send(self: Arc<Self>, args: Value) -> Result<Value> {
        let peer_id = args
            .get("peerId")
            .and_then(Value::as_str)
            .context("missing peerId")?;
        let peer = self
            .peers
            .lock()
            .unwrap()
            .get(peer_id)
            .cloned()
            .context("dispositivo LocalSend non più visibile: aggiorna la lista")?;
        let mut items = Vec::new();
        if let Some(text) = args
            .get("text")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        {
            items.push((
                PathBuf::new(),
                FileMeta {
                    id: uuid::Uuid::new_v4().simple().to_string(),
                    file_name: format!("{}.txt", uuid::Uuid::new_v4().simple()),
                    size: text.len() as u64,
                    file_type: Some("text/plain".into()),
                    sha256: None,
                    preview: Some(text.to_owned()),
                    metadata: None,
                },
            ));
        }
        let paths: Vec<PathBuf> = crate::args(args.get("paths").cloned().unwrap_or(json!([])))?;
        for p in paths {
            collect_files(&p, None, &mut items)?;
        }
        if items.is_empty() {
            bail!("niente da inviare");
        }
        let text_only = items.iter().all(|(p, _)| p.as_os_str().is_empty());
        if !text_only && items.iter().any(|(p, _)| p.as_os_str().is_empty()) {
            bail!("invia il testo da solo, senza file");
        }
        let pin = args.get("pin").and_then(Value::as_str).map(str::to_owned);
        let tid = uuid::Uuid::new_v4().simple().to_string();
        self.put_transfer(LsTransfer {
            id: tid.clone(),
            direction: "outgoing",
            peer: peer.alias.clone(),
            state: "waiting",
            total: items.iter().map(|(_, m)| m.size).sum(),
            transferred: 0,
            files: items
                .iter()
                .map(|(p, m)| LsFile {
                    name: if p.as_os_str().is_empty() {
                        "Testo".into()
                    } else {
                        m.file_name.clone()
                    },
                    size: m.size,
                    mime: m.file_type.clone().unwrap_or_default(),
                    path: (!p.as_os_str().is_empty()).then(|| p.clone()),
                    done: false,
                })
                .collect(),
            error: None,
            started_at: now_millis(),
        });
        let me = self.clone();
        let id = tid.clone();
        let task = tokio::spawn(async move { me.send(id, peer, items, pin).await });
        self.outgoing
            .lock()
            .unwrap()
            .insert(tid.clone(), (task.abort_handle(), None));
        let r = task.await;
        self.outgoing.lock().unwrap().remove(&tid);
        match r {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => {
                self.update_transfer(&tid, |x| {
                    if x.state == "waiting" || x.state == "inProgress" {
                        x.state = "failed";
                        x.error = Some(format!("{e:#}"));
                    }
                });
                Err(e)
            }
            Err(_) => {
                self.update_transfer(&tid, |x| x.state = "cancelled");
                Ok(json!({ "state": "cancelled" }))
            }
        }
    }

    async fn cancel(&self, id: &str) -> Result<()> {
        let out = self.outgoing.lock().unwrap().remove(id);
        if let Some((task, remote)) = out {
            task.abort();
            self.update_transfer(id, |x| x.state = "cancelled");
            if let Some((t, sid)) = remote {
                let _ = http::request(
                    &t,
                    "POST",
                    &format!("{API}/cancel?sessionId={}", http::encode(&sid)),
                    None,
                    Duration::from_secs(3),
                )
                .await;
            }
            return Ok(());
        }
        if self.answer(id, false) {
            return Ok(());
        }
        let gone = {
            let mut s = self.session.lock().unwrap();
            if s.as_ref().is_some_and(|x| x.id == id) {
                s.take()
            } else {
                None
            }
        };
        if gone.is_some() {
            self.update_transfer(id, |x| x.state = "cancelled");
        }
        Ok(())
    }
}

#[async_trait]
impl Plugin for LocalSendPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "localsend",
            name: "LocalSend",
            description: "Invia e ricevi file con le app LocalSend sulla stessa rete (si attiva nelle impostazioni)",
            permission: Permission::Files,
            incoming: &[],
            outgoing: &[],
        }
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        self.config.attach(&core, "localsend", "config.json");
        self.load_identity(&core);
        let _ = self.core.set(core);
        self.restart();
    }

    async fn handle_packet(&self, _dev: &DeviceHandle, _p: Packet) -> Result<()> {
        Ok(())
    }

    async fn handle_action(
        &self,
        _dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        let me = self.me.upgrade().context("plugin dropped")?;
        match action {
            "status" => Ok(self.status()),
            "getConfig" => Ok(serde_json::to_value(self.config.get())?),
            "setConfig" => {
                let mut v = serde_json::to_value(self.config.get())?;
                if let (Some(o), Some(new)) = (v.as_object_mut(), args.as_object()) {
                    for (k, val) in new {
                        o.insert(k.clone(), val.clone());
                    }
                }
                let mut c: LsConfig = crate::args(v)?;
                if c.port == 0 {
                    c.port = DEFAULT_PORT;
                }
                if let Some(a) = &c.alias
                    && (a.chars().count() > 64 || a.chars().any(char::is_control))
                {
                    bail!("nome non valido");
                }
                let restart = self
                    .config
                    .with(|o| o.enabled != c.enabled || o.port != c.port || o.https != c.https);
                self.config.update(|o| *o = c);
                if restart {
                    self.restart();
                } else {
                    self.emit("status", self.status());
                }
                Ok(self.status())
            }
            "peers" => Ok(serde_json::to_value(self.peer_list())?),
            "scan" => {
                if !self.run.lock().unwrap().running {
                    bail!("LocalSend non è attivo: attivalo nelle impostazioni");
                }
                tokio::spawn(me.scan());
                Ok(Value::Null)
            }
            "addPeer" => {
                let addr = args
                    .get("address")
                    .and_then(Value::as_str)
                    .context("missing address")?
                    .trim()
                    .to_owned();
                let (host, port) = match addr.rsplit_once(':') {
                    Some((h, p)) if !h.contains(':') => {
                        (h.to_owned(), p.parse().context("porta non valida")?)
                    }
                    _ => (addr.clone(), self.config.with(|c| c.port)),
                };
                let https = self.config.with(|c| c.https);
                let mut last = None;
                for proto in [https, !https] {
                    let t = Target {
                        host: host.clone(),
                        port,
                        https: proto,
                    };
                    match self.register_to(&t, Duration::from_secs(4)).await {
                        Ok(p) => return Ok(serde_json::to_value(p)?),
                        Err(e) => last = Some(e),
                    }
                }
                Err(last
                    .unwrap_or_else(|| anyhow::anyhow!("not reachable"))
                    .context(format!("nessun dispositivo LocalSend su {addr}")))
            }
            "send" => me.start_send(args).await,
            "cancel" => {
                self.cancel(
                    args.get("id")
                        .and_then(Value::as_str)
                        .context("missing id")?,
                )
                .await?;
                Ok(Value::Null)
            }
            "accept" | "reject" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .context("missing id")?;
                if !self.answer(id, action == "accept") {
                    bail!("la richiesta non è più valida");
                }
                Ok(Value::Null)
            }
            "transfers" => Ok(serde_json::to_value(
                self.transfers.lock().unwrap().clone(),
            )?),
            "clearFinished" => {
                self.transfers
                    .lock()
                    .unwrap()
                    .retain(|t| matches!(t.state, "waiting" | "inProgress"));
                Ok(Value::Null)
            }
            _ => bail!("unsupported action '{action}'"),
        }
    }
}

// --- helpers -----------------------------------------------------------------------

/// The files under `p` (folders are walked; LocalSend keeps the relative
/// path in the file name). Symbolic links are skipped.
fn collect_files(p: &Path, prefix: Option<&str>, out: &mut Vec<(PathBuf, FileMeta)>) -> Result<()> {
    let meta =
        std::fs::symlink_metadata(p).with_context(|| format!("cannot read {}", p.display()))?;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let rel = match prefix {
        Some(pre) => format!("{pre}/{name}"),
        None => name,
    };
    if meta.is_dir() {
        for e in std::fs::read_dir(p)? {
            collect_files(&e?.path(), Some(&rel), out)?;
        }
    } else if meta.is_file() {
        let modified = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok());
        out.push((
            p.to_owned(),
            FileMeta {
                id: uuid::Uuid::new_v4().simple().to_string(),
                file_type: Some(mime_of(&rel).into()),
                file_name: rel,
                size: meta.len(),
                sha256: None,
                preview: None,
                metadata: modified.map(|d| json!({ "modified": rfc3339(d.as_secs()) })),
            },
        ));
    }
    Ok(())
}

fn clean(s: &str, max: usize) -> Option<String> {
    let s: String = s.chars().filter(|c| !c.is_control()).take(max).collect();
    let s = s.trim().to_owned();
    (!s.is_empty()).then_some(s)
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "linux" => "Linux",
        "android" => "Android",
        "windows" => "Windows",
        "macos" => "macOS",
        "ios" => "iOS",
        _ => "Konnekta",
    }
}

fn human(n: u64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1 << 20 => format!("{} KB", n / 1024),
        n if n < 1 << 30 => format!("{:.1} MB", n as f64 / 1048576.0),
        n => format!("{:.1} GB", n as f64 / 1073741824.0),
    }
}

/// MIME type from the extension (LocalSend shows an icon per type).
pub fn mime_of(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "apk" => "application/vnd.android.package-archive",
        "txt" | "log" | "md" => "text/plain",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "json" => "application/json",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        _ => "application/octet-stream",
    }
}

fn rfc3339(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// Milliseconds since the epoch of `YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]`.
fn parse_rfc3339_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &s[19..];
    let mut ms = 0i64;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
        rest = &frac[digits.len()..];
        ms = format!("{:0<3}", &digits[..digits.len().min(3)])
            .parse()
            .unwrap_or(0);
    }
    let offset = match rest.as_bytes().first() {
        Some(b'+') | Some(b'-') if rest.len() >= 6 => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (rest[1..3].parse::<i64>().ok()? * 3600 + rest[4..6].parse::<i64>().ok()? * 60)
        }
        _ => 0,
    };
    let (yy, mm) = if mo <= 2 {
        (y - 1, mo + 9)
    } else {
        (y, mo - 3)
    };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86400 + h * 3600 + mi * 60 + se - offset;
    u64::try_from(secs * 1000 + ms).ok()
}

fn local_ipv4() -> Vec<Ipv4Addr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() {
            IpAddr::V4(v4) if !v4.is_link_local() => Some(v4),
            _ => None,
        })
        .collect()
}

fn bind_multicast(port: u16) -> Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    let _ = s.set_reuse_port(true);
    s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())
        .with_context(|| format!("cannot bind UDP {port}"))?;
    s.set_multicast_loop_v4(true)?;
    let mut joined = false;
    for ip in local_ipv4() {
        joined |= s.join_multicast_v4(&MULTICAST, &ip).is_ok();
    }
    if !joined {
        s.join_multicast_v4(&MULTICAST, &Ipv4Addr::UNSPECIFIED)?;
    }
    s.set_nonblocking(true)?;
    Ok(UdpSocket::from_std(s.into())?)
}

/// Send on every IPv4 interface, so devices on any of them hear us.
async fn send_multicast(sock: &UdpSocket, msg: &[u8], port: u16) {
    let dest = SocketAddr::from((MULTICAST, port));
    let ifaces = local_ipv4();
    if ifaces.is_empty() {
        let _ = sock.send_to(msg, dest).await;
        return;
    }
    for ip in ifaces {
        if socket2::SockRef::from(sock)
            .set_multicast_if_v4(&ip)
            .is_ok()
        {
            let _ = sock.send_to(msg, dest).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(
            parse_rfc3339_ms("2023-11-14T22:13:20Z"),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2023-11-14T23:13:20.5+01:00"),
            Some(1_700_000_000_500)
        );
        assert_eq!(parse_rfc3339_ms("bogus"), None);
    }

    #[test]
    fn wire_info_accepts_v1_and_v2() {
        let v2: WireInfo = serde_json::from_str(
            r#"{"alias":"Pixel","version":"2.1","deviceModel":"Google","deviceType":"mobile","fingerprint":"AB","port":53317,"protocol":"https","download":false,"announce":true}"#,
        )
        .unwrap();
        assert_eq!(v2.announce, Some(true));
        let v1: WireInfo =
            serde_json::from_str(r#"{"alias":"Old","fingerprint":"x","announcement":true}"#)
                .unwrap();
        assert_eq!(v1.announcement, Some(true));
    }

    fn plugin_with(dir: &Path) -> Arc<LocalSendPlugin> {
        let p = LocalSendPlugin::new(Platform::unsupported());
        let _ = p
            .identity
            .set(LocalIdentity::generate("konnekta-localsend").unwrap());
        p.config.update(|c| c.destination = Some(dir.to_owned()));
        p
    }

    /// A LocalSend sender (our client) and receiver (our server) over HTTP and HTTPS.
    #[tokio::test]
    async fn send_and_receive_between_two_instances() {
        for https in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let rx = plugin_with(dir.path());
            rx.config.update(|c| {
                c.auto_accept = true;
                c.https = https;
            });
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let tls = https.then(|| {
                tokio_rustls::TlsAcceptor::from(
                    konnekta_crypto::open_server_config(rx.identity.get().unwrap()).unwrap(),
                )
            });
            tokio::spawn(rx.clone().serve(listener, tls));

            let tx = plugin_with(dir.path());
            let peer = tx
                .register_to(
                    &Target {
                        host: "127.0.0.1".into(),
                        port,
                        https,
                    },
                    Duration::from_secs(5),
                )
                .await
                .unwrap();
            assert_eq!(peer.port, port);

            let src = tempfile::tempdir().unwrap();
            let f = src.path().join("ciao.txt");
            std::fs::write(&f, b"ciao LocalSend").unwrap();
            let r = tx
                .clone()
                .start_send(json!({ "peerId": peer.id, "paths": [f] }))
                .await
                .unwrap();
            assert_eq!(r["state"], "completed");
            assert_eq!(
                std::fs::read(dir.path().join("ciao.txt")).unwrap(),
                b"ciao LocalSend"
            );
            let done = rx.transfers.lock().unwrap()[0].clone();
            assert_eq!(done.state, "completed");

            // A message needs no upload.
            let r = tx
                .clone()
                .start_send(json!({ "peerId": peer.id, "text": "hello" }))
                .await
                .unwrap();
            assert_eq!(r["state"], "completed");
        }
    }

    #[tokio::test]
    async fn refused_without_answer_and_pin() {
        let dir = tempfile::tempdir().unwrap();
        let rx = plugin_with(dir.path());
        rx.config.update(|c| {
            c.https = false;
            c.pin = Some("1234".into());
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(rx.clone().serve(listener, None));
        let tx = plugin_with(dir.path());
        let peer = tx
            .register_to(
                &Target {
                    host: "127.0.0.1".into(),
                    port,
                    https: false,
                },
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        let src = tempfile::tempdir().unwrap();
        let f = src.path().join("a.bin");
        std::fs::write(&f, [1u8; 10]).unwrap();
        let e = tx
            .clone()
            .start_send(json!({ "peerId": peer.id, "paths": [&f] }))
            .await
            .unwrap_err();
        assert!(e.to_string().contains("PIN"));

        // Right PIN, then the user refuses.
        let rx2 = rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(20)).await;
                let id = rx2.asking.lock().unwrap().clone();
                if let Some(id) = id {
                    rx2.answer(&id, false);
                    break;
                }
            }
        });
        let e = tx
            .clone()
            .start_send(json!({ "peerId": peer.id, "paths": [&f], "pin": "1234" }))
            .await
            .unwrap_err();
        assert!(e.to_string().contains("rifiutato"), "{e}");
    }
}
