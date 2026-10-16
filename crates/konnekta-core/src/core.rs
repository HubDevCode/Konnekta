//! The orchestrator: owns identity, trust store, links and plugins.

use crate::config::Config;
use crate::device::{DeviceHandle, DeviceInfo};
use crate::event::Event;
use crate::link::{self, SecureStream};
use crate::pairing::{self, PairState};
use crate::permissions::Permission;
use crate::plugin::{CoreHandle, Plugin, PluginInfo};
use crate::trust::{TrustStore, TrustedDevice};
use crate::{discovery, payload};
use anyhow::{Context, Result, bail};
use konnekta_crypto::{FileKeyStore, KeyStore, LocalIdentity, PeerExpectation, verification_key};
use konnekta_proto::bodies::PairBody;
use konnekta_proto::{IdentityBody, PROTOCOL_VERSION, Packet, types};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{broadcast, mpsc};
use tokio::task::AbortHandle;

const OUTGOING_RETRY: Duration = Duration::from_secs(5);
const SEND_QUEUE: usize = 512;

pub(crate) struct LinkEntry {
    handle: DeviceHandle,
    pair: PairState,
    tasks: Vec<AbortHandle>,
    denied: HashSet<Permission>,
}

pub(crate) struct Inner {
    pub(crate) identity: LocalIdentity,
    pub(crate) config: RwLock<Config>,
    pub(crate) data_dir: Option<PathBuf>,
    trust: Mutex<TrustStore>,
    links: Mutex<HashMap<String, LinkEntry>>,
    plugins: Vec<Arc<dyn Plugin>>,
    pub(crate) events: broadcast::Sender<Event>,
    tcp_port: AtomicU16,
    udp_port: AtomicU16,
    next_link_id: AtomicU64,
    udp: Mutex<Option<Arc<UdpSocket>>>,
    connecting: Mutex<HashMap<String, Instant>>,
    background: Mutex<Vec<AbortHandle>>,
}

/// Builder for [`Konnekta`].
pub struct KonnektaBuilder {
    config: Config,
    data_dir: Option<PathBuf>,
    identity: Option<LocalIdentity>,
    plugins: Vec<Arc<dyn Plugin>>,
}

impl KonnektaBuilder {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            data_dir: None,
            identity: None,
            plugins: Vec::new(),
        }
    }

    /// Persist identity, trusted devices and plugin data in `dir`.
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }

    /// Use an explicit identity (tests, mobile key stores).
    pub fn identity(mut self, id: LocalIdentity) -> Self {
        self.identity = Some(id);
        self
    }

    pub fn plugin(mut self, p: Arc<dyn Plugin>) -> Self {
        self.plugins.push(p);
        self
    }

    pub fn plugins(mut self, ps: impl IntoIterator<Item = Arc<dyn Plugin>>) -> Self {
        self.plugins.extend(ps);
        self
    }

    pub fn build(self) -> Result<Konnekta> {
        konnekta_crypto::install_crypto_provider();
        let identity = match (self.identity, &self.data_dir) {
            (Some(i), _) => i,
            (None, Some(dir)) => FileKeyStore::new(dir).load_or_create()?,
            (None, None) => LocalIdentity::generate(&konnekta_crypto::generate_device_id())?,
        };
        let trust = match &self.data_dir {
            Some(d) => TrustStore::open(d.join("devices.json"))?,
            None => TrustStore::in_memory(),
        };
        let mut seen = HashSet::new();
        for p in &self.plugins {
            if !seen.insert(p.info().id) {
                bail!("duplicate plugin id {}", p.info().id);
            }
        }
        let (events, _) = broadcast::channel(1024);
        let inner = Arc::new(Inner {
            identity,
            config: RwLock::new(self.config),
            data_dir: self.data_dir,
            trust: Mutex::new(trust),
            links: Mutex::new(HashMap::new()),
            plugins: self.plugins,
            events,
            tcp_port: AtomicU16::new(0),
            udp_port: AtomicU16::new(0),
            next_link_id: AtomicU64::new(1),
            udp: Mutex::new(None),
            connecting: Mutex::new(HashMap::new()),
            background: Mutex::new(Vec::new()),
        });
        Ok(Konnekta { inner })
    }
}

/// The running Konnekta instance. Cheap to clone.
#[derive(Clone)]
pub struct Konnekta {
    inner: Arc<Inner>,
}

impl Konnekta {
    pub fn builder(config: Config) -> KonnektaBuilder {
        KonnektaBuilder::new(config)
    }

    pub fn handle(&self) -> CoreHandle {
        CoreHandle {
            inner: Arc::downgrade(&self.inner),
        }
    }

    pub fn device_id(&self) -> &str {
        &self.inner.identity.device_id
    }

    pub fn fingerprint(&self) -> String {
        self.inner.identity.fingerprint()
    }

    pub fn tcp_port(&self) -> u16 {
        self.inner.tcp_port.load(Ordering::Relaxed)
    }

    pub fn udp_port(&self) -> u16 {
        self.inner.udp_port.load(Ordering::Relaxed)
    }

    pub fn config(&self) -> Config {
        self.inner.config.read().unwrap().clone()
    }

    /// Update configuration (device name, download dir...). Network settings
    /// take effect on restart.
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) -> Result<()> {
        let cfg = {
            let mut c = self.inner.config.write().unwrap();
            f(&mut c);
            c.clone()
        };
        if let Some(d) = &self.inner.data_dir {
            cfg.save(&d.join("config.json"))?;
        }
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    pub fn plugins(&self) -> Vec<(PluginInfo, bool)> {
        let disabled = self.inner.config.read().unwrap().disabled_plugins.clone();
        self.inner
            .plugins
            .iter()
            .map(|p| {
                let i = p.info();
                let enabled = !disabled.contains(i.id);
                (i, enabled)
            })
            .collect()
    }

    /// Start listening and discovery.
    pub async fn start(&self) -> Result<()> {
        let inner = &self.inner;
        let cfg = self.config();

        // TCP listener: first free port in 1716..=1764 (or ephemeral if 0).
        let listener = if cfg.tcp_port == 0 {
            TcpListener::bind(SocketAddr::new(cfg.bind_addr, 0)).await?
        } else {
            let mut l = None;
            for port in cfg.tcp_port..=konnekta_proto::MAX_PORT.max(cfg.tcp_port) {
                if let Ok(s) = TcpListener::bind(SocketAddr::new(cfg.bind_addr, port)).await {
                    l = Some(s);
                    break;
                }
            }
            l.context("no free TCP port in 1716-1764")?
        };
        inner
            .tcp_port
            .store(listener.local_addr()?.port(), Ordering::Relaxed);
        tracing::info!(port = self.tcp_port(), device_id = %self.device_id(), "listening");

        let weak = Arc::downgrade(inner);
        let accept = tokio::spawn(async move {
            loop {
                let Ok((tcp, from)) = listener.accept().await else {
                    continue;
                };
                let Some(inner) = weak.upgrade() else { break };
                tokio::spawn(async move {
                    if let Err(e) = inner.clone().handle_incoming(tcp).await {
                        tracing::debug!(%from, "incoming connection failed: {e:#}");
                    }
                });
            }
        });
        inner.background.lock().unwrap().push(accept.abort_handle());

        // UDP discovery.
        let udp_addr =
            SocketAddr::new(cfg.bind_addr, if cfg.listen_udp { cfg.udp_port } else { 0 });
        match discovery::bind_udp(udp_addr) {
            Ok(sock) => {
                let sock = Arc::new(sock);
                inner
                    .udp_port
                    .store(sock.local_addr()?.port(), Ordering::Relaxed);
                *inner.udp.lock().unwrap() = Some(sock.clone());
                if cfg.listen_udp {
                    let weak = Arc::downgrade(inner);
                    let s = sock.clone();
                    let t = tokio::spawn(async move {
                        let mut buf = vec![0u8; 16 * 1024];
                        loop {
                            let Ok((n, from)) = s.recv_from(&mut buf).await else {
                                continue;
                            };
                            let Some(inner) = weak.upgrade() else { break };
                            if let Some(id) = discovery::parse_announcement(&buf[..n]) {
                                inner.on_announcement(id, from);
                            }
                        }
                    });
                    inner.background.lock().unwrap().push(t.abort_handle());
                }
                if cfg.announce {
                    let weak = Arc::downgrade(inner);
                    let interval = cfg.announce_interval_secs;
                    let t = tokio::spawn(async move {
                        loop {
                            let Some(inner) = weak.upgrade() else { break };
                            inner.announce().await;
                            drop(inner);
                            if interval == 0 {
                                break;
                            }
                            tokio::time::sleep(Duration::from_secs(interval)).await;
                        }
                    });
                    inner.background.lock().unwrap().push(t.abort_handle());
                }
            }
            Err(e) => tracing::warn!("UDP discovery unavailable on {udp_addr}: {e}"),
        }

        for p in &inner.plugins {
            p.clone().start(self.handle()).await;
        }
        Ok(())
    }

    /// Stop all network activity.
    pub fn shutdown(&self) {
        for t in self.inner.background.lock().unwrap().drain(..) {
            t.abort();
        }
        let links: Vec<_> = self.inner.links.lock().unwrap().drain().collect();
        for (_, l) in links {
            for t in l.tasks {
                t.abort();
            }
        }
    }

    /// Broadcast our identity now.
    pub async fn refresh(&self) {
        self.inner.announce().await;
    }

    /// Ask a device at a known address to connect to us (like KDE Connect's
    /// "add device by IP"): sends our identity to `addr` (UDP port).
    pub async fn connect_to(&self, addr: SocketAddr) -> Result<()> {
        let sock = self
            .inner
            .udp
            .lock()
            .unwrap()
            .clone()
            .context("UDP socket not bound")?;
        sock.send_to(&self.inner.identity_packet().to_line(), addr)
            .await?;
        Ok(())
    }

    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.inner.device_infos()
    }

    pub fn device(&self, id: &str) -> Option<DeviceInfo> {
        self.devices().into_iter().find(|d| d.id == id)
    }

    pub fn device_handle(&self, id: &str) -> Option<DeviceHandle> {
        self.inner.paired_handle(id)
    }

    pub async fn request_pair(&self, id: &str) -> Result<String> {
        self.inner.clone().request_pair(id).await
    }

    pub async fn accept_pair(&self, id: &str) -> Result<()> {
        self.inner.clone().accept_pair(id).await
    }

    pub async fn reject_pair(&self, id: &str) -> Result<()> {
        self.inner.clone().reject_pair(id).await
    }

    pub async fn unpair(&self, id: &str) -> Result<()> {
        self.inner.clone().unpair(id, true).await
    }

    pub fn set_permission(&self, id: &str, perm: Permission, granted: bool) -> Result<()> {
        let ok = self.inner.trust.lock().unwrap().update(id, |d| {
            if granted {
                d.permissions.insert(perm);
            } else {
                d.permissions.remove(&perm);
            }
        });
        if !ok {
            bail!("device not paired");
        }
        if let Some(e) = self.inner.links.lock().unwrap().get_mut(id) {
            e.denied.remove(&perm);
        }
        self.inner.emit_updated(id);
        Ok(())
    }

    pub fn set_device_plugin_enabled(&self, id: &str, plugin: &str, enabled: bool) -> Result<()> {
        let ok = self.inner.trust.lock().unwrap().update(id, |d| {
            if enabled {
                d.disabled_plugins.remove(plugin);
            } else {
                d.disabled_plugins.insert(plugin.to_owned());
            }
        });
        if !ok {
            bail!("device not paired");
        }
        self.inner.emit_updated(id);
        Ok(())
    }

    pub fn set_plugin_enabled(&self, plugin: &str, enabled: bool) -> Result<()> {
        if !self.inner.plugins.iter().any(|p| p.info().id == plugin) {
            bail!("unknown plugin {plugin}");
        }
        self.update_config(|c| {
            if enabled {
                c.disabled_plugins.remove(plugin);
            } else {
                c.disabled_plugins.insert(plugin.to_owned());
            }
        })
    }

    /// Invoke a plugin action from the local UI.
    pub async fn plugin_action(
        &self,
        device_id: Option<&str>,
        plugin: &str,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        self.inner
            .plugin_action(device_id, plugin, action, args)
            .await
    }
}

impl Inner {
    pub(crate) async fn plugin_action(
        &self,
        device_id: Option<&str>,
        plugin: &str,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        let p = self
            .plugins
            .iter()
            .find(|p| p.info().id == plugin)
            .cloned()
            .with_context(|| format!("unknown plugin {plugin}"))?;
        if self
            .config
            .read()
            .unwrap()
            .disabled_plugins
            .contains(plugin)
        {
            bail!("plugin {plugin} is disabled");
        }
        let dev = match device_id {
            Some(id) => Some(
                self.paired_handle(id)
                    .context("device not connected or not paired")?,
            ),
            None => None,
        };
        p.handle_action(dev.as_ref(), action, args).await
    }

    fn capabilities(&self) -> (Vec<String>, Vec<String>) {
        let disabled = self.config.read().unwrap().disabled_plugins.clone();
        let mut inc = BTreeSet::new();
        let mut out = BTreeSet::new();
        for p in &self.plugins {
            let i = p.info();
            if disabled.contains(i.id) {
                continue;
            }
            inc.extend(i.incoming.iter().map(|s| s.to_string()));
            out.extend(i.outgoing.iter().map(|s| s.to_string()));
        }
        inc.insert(payload::RESUME_CAPABILITY.to_string());
        (inc.into_iter().collect(), out.into_iter().collect())
    }

    pub(crate) fn identity_packet(&self) -> Packet {
        let cfg = self.config.read().unwrap();
        let (incoming, outgoing) = self.capabilities();
        Packet::new(
            types::IDENTITY,
            IdentityBody {
                device_id: self.identity.device_id.clone(),
                device_name: cfg.device_name.clone(),
                device_type: cfg.device_type,
                protocol_version: PROTOCOL_VERSION,
                incoming_capabilities: incoming,
                outgoing_capabilities: outgoing,
                tcp_port: Some(self.tcp_port.load(Ordering::Relaxed)),
            },
        )
    }

    async fn announce(&self) {
        let sock = self.udp.lock().unwrap().clone();
        if let Some(sock) = sock {
            let targets = self.config.read().unwrap().broadcast_targets.clone();
            discovery::announce(&sock, &self.identity_packet(), &targets).await;
        }
    }

    fn expectation_for(&self, device_id: &str) -> PeerExpectation {
        match self.trust.lock().unwrap().get(device_id) {
            Some(t) => PeerExpectation::Pinned {
                device_id: device_id.to_owned(),
                cert_der: t.cert_der(),
            },
            None => PeerExpectation::Unpaired {
                device_id: device_id.to_owned(),
            },
        }
    }

    fn unpaired_link_count(&self) -> usize {
        self.links
            .lock()
            .unwrap()
            .values()
            .filter(|l| !l.pair.is_paired())
            .count()
    }

    fn admit_unpaired(&self, device_id: &str) -> bool {
        self.trust.lock().unwrap().is_trusted(device_id)
            || self.unpaired_link_count() < self.config.read().unwrap().max_unpaired_links
    }

    fn on_announcement(self: &Arc<Self>, id: IdentityBody, from: SocketAddr) {
        if id.device_id == self.identity.device_id {
            return;
        }
        if self.links.lock().unwrap().contains_key(&id.device_id) {
            return;
        }
        {
            let mut c = self.connecting.lock().unwrap();
            if let Some(t) = c.get(&id.device_id)
                && t.elapsed() < OUTGOING_RETRY
            {
                return;
            }
            c.insert(id.device_id.clone(), Instant::now());
        }
        if !self.admit_unpaired(&id.device_id) {
            tracing::debug!("too many unpaired links, ignoring {}", id.device_id);
            return;
        }
        let Some(port) = id.tcp_port else { return };
        let addr = SocketAddr::new(from.ip(), port);
        let this = self.clone();
        tokio::spawn(async move {
            let expect = this.expectation_for(&id.device_id);
            let me = this.identity_packet();
            match link::connect_outgoing(&this.identity, &me, addr, expect).await {
                Ok(tls) => {
                    if let Err(e) = this.clone().establish(tls, id, addr).await {
                        tracing::debug!(%addr, "link setup failed: {e:#}");
                    }
                }
                Err(e) => this.report_tls_failure(&id.device_id, &e),
            }
        });
    }

    async fn handle_incoming(self: Arc<Self>, mut tcp: TcpStream) -> Result<()> {
        let addr = tcp.peer_addr()?;
        let id = link::read_plain_identity(&mut tcp).await?;
        if id.device_id == self.identity.device_id {
            bail!("connection from ourselves");
        }
        if !self.admit_unpaired(&id.device_id) {
            bail!("too many unpaired links");
        }
        let expect = self.expectation_for(&id.device_id);
        match link::accept_incoming(&self.identity, tcp, expect).await {
            Ok(tls) => self.establish(tls, id, addr).await,
            Err(e) => {
                self.report_tls_failure(&id.device_id, &e);
                Err(e)
            }
        }
    }

    fn report_tls_failure(&self, device_id: &str, e: &anyhow::Error) {
        let msg = format!("{e:#}");
        if self.trust.lock().unwrap().is_trusted(device_id)
            && (msg.contains("certificate") || msg.contains("Certificate"))
        {
            let _ = self.events.send(Event::SecurityWarning {
                device_id: Some(device_id.to_owned()),
                message: "A device claiming to be this paired device presented a different \
                          certificate. The connection was refused. If the device was \
                          reinstalled, unpair and pair it again."
                    .into(),
            });
        }
        tracing::debug!(device_id, "tls failed: {msg}");
    }

    async fn establish(
        self: Arc<Self>,
        tls: SecureStream,
        plain: IdentityBody,
        addr: SocketAddr,
    ) -> Result<()> {
        let peer_cert = link::peer_certificate(&tls).context("peer sent no certificate")?;
        let mut stream = BufReader::new(tls);
        let me = self.identity_packet();
        let identity = link::exchange_identity(&mut stream, &me).await?;
        if identity.device_id != plain.device_id {
            bail!("identity changed during handshake");
        }
        // The certificate CN was already checked against plain.device_id by
        // the TLS verifier, so the inner identity is bound to the certificate.

        let (rd, mut wr) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::channel::<Packet>(SEND_QUEUE);
        let link_id = self.next_link_id.fetch_add(1, Ordering::Relaxed);
        let handle = DeviceHandle {
            inner: Arc::downgrade(&self),
            link_id,
            tx,
            identity: Arc::new(identity.clone()),
            addr,
            peer_cert: Arc::new(peer_cert),
        };
        let device_id = identity.device_id.clone();

        let writer = tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                if wr.write_all(&p.to_line()).await.is_err() || wr.flush().await.is_err() {
                    break;
                }
            }
            let _ = wr.shutdown().await;
        });

        let this = self.clone();
        let rid = device_id.clone();
        let reader = tokio::spawn(async move {
            let mut rd = BufReader::new(rd);
            loop {
                match link::read_packet(&mut rd).await {
                    Ok(Some(p)) => this.dispatch(&rid, link_id, p).await,
                    Ok(None) => break,
                    Err(e) => {
                        tracing::debug!(device = %rid, "link read error: {e:#}");
                        break;
                    }
                }
            }
            this.on_link_closed(&rid, link_id).await;
        });

        let trusted = self.trust.lock().unwrap().is_trusted(&device_id);
        let entry = LinkEntry {
            handle: handle.clone(),
            pair: if trusted {
                PairState::Paired
            } else {
                PairState::NotPaired
            },
            tasks: vec![reader.abort_handle(), writer.abort_handle()],
            denied: HashSet::new(),
        };
        let old = self.links.lock().unwrap().insert(device_id.clone(), entry);
        if let Some(old) = old {
            for t in old.tasks {
                t.abort();
            }
        }
        if trusted {
            self.trust.lock().unwrap().update(&device_id, |d| {
                d.name = identity.device_name.clone();
                d.device_type = identity.device_type;
                d.last_address = Some(addr.ip());
            });
        }
        tracing::info!(device = %identity.device_name, %addr, paired = trusted, "device connected");
        if let Some(info) = self.device_info(&device_id) {
            let _ = self.events.send(Event::DeviceConnected { device: info });
        }
        if trusted {
            self.notify_plugins_connected(&handle).await;
        }
        Ok(())
    }

    async fn on_link_closed(&self, device_id: &str, link_id: u64) {
        let removed = {
            let mut links = self.links.lock().unwrap();
            match links.get(device_id) {
                Some(l) if l.handle.link_id == link_id => links.remove(device_id),
                _ => None,
            }
        };
        if let Some(entry) = removed {
            for t in &entry.tasks {
                t.abort();
            }
            tracing::info!(device = %device_id, "device disconnected");
            let _ = self.events.send(Event::DeviceDisconnected {
                device_id: device_id.to_owned(),
            });
            if entry.pair.is_paired() {
                for p in &self.plugins {
                    p.on_disconnected(device_id).await;
                }
            }
        }
    }

    async fn notify_plugins_connected(&self, handle: &DeviceHandle) {
        let disabled_global = self.config.read().unwrap().disabled_plugins.clone();
        let disabled_dev = self
            .trust
            .lock()
            .unwrap()
            .get(handle.id())
            .map(|d| d.disabled_plugins.clone())
            .unwrap_or_default();
        for p in &self.plugins {
            let i = p.info();
            if disabled_global.contains(i.id) || disabled_dev.contains(i.id) {
                continue;
            }
            if self.has_permission(handle.id(), i.permission) {
                p.on_connected(handle).await;
            }
        }
    }

    pub(crate) fn is_paired(&self, id: &str) -> bool {
        self.links
            .lock()
            .unwrap()
            .get(id)
            .map(|l| l.pair.is_paired())
            .unwrap_or(false)
            && self.trust.lock().unwrap().is_trusted(id)
    }

    pub(crate) fn has_permission(&self, id: &str, p: Permission) -> bool {
        self.trust
            .lock()
            .unwrap()
            .get(id)
            .map(|d| d.permissions.contains(&p))
            .unwrap_or(false)
    }

    pub(crate) fn permission_for_outgoing(&self, packet_type: &str) -> Option<Permission> {
        self.plugins
            .iter()
            .map(|p| p.info())
            .find(|i| i.outgoing.contains(&packet_type))
            .map(|i| i.permission)
    }

    pub(crate) fn paired_handle(&self, id: &str) -> Option<DeviceHandle> {
        if !self.trust.lock().unwrap().is_trusted(id) {
            return None;
        }
        self.links
            .lock()
            .unwrap()
            .get(id)
            .filter(|l| l.pair.is_paired())
            .map(|l| l.handle.clone())
    }

    pub(crate) fn paired_handles(&self) -> Vec<DeviceHandle> {
        let ids: Vec<String> = self.links.lock().unwrap().keys().cloned().collect();
        ids.iter().filter_map(|id| self.paired_handle(id)).collect()
    }

    fn device_info(&self, id: &str) -> Option<DeviceInfo> {
        self.device_infos().into_iter().find(|d| d.id == id)
    }

    fn device_infos(&self) -> Vec<DeviceInfo> {
        let links = self.links.lock().unwrap();
        let trust = self.trust.lock().unwrap();
        let mut out = Vec::new();
        for t in trust.all() {
            let link = links.get(&t.device_id);
            out.push(DeviceInfo {
                id: t.device_id.clone(),
                name: t.name.clone(),
                device_type: t.device_type,
                paired: true,
                connected: link.is_some(),
                pair_state: link.map(|l| l.pair.as_str()).unwrap_or("paired").into(),
                address: link.map(|l| l.handle.addr.ip().to_string()),
                fingerprint: Some(konnekta_crypto::cert_fingerprint(&t.cert_der())),
                permissions: t.permissions.clone(),
                disabled_plugins: t.disabled_plugins.clone(),
                incoming_capabilities: link
                    .map(|l| l.handle.identity.incoming_capabilities.clone())
                    .unwrap_or_default(),
                outgoing_capabilities: link
                    .map(|l| l.handle.identity.outgoing_capabilities.clone())
                    .unwrap_or_default(),
            });
        }
        for (id, l) in links.iter() {
            if trust.is_trusted(id) {
                continue;
            }
            out.push(DeviceInfo {
                id: id.clone(),
                name: l.handle.identity.device_name.clone(),
                device_type: l.handle.identity.device_type,
                paired: false,
                connected: true,
                pair_state: l.pair.as_str().into(),
                address: Some(l.handle.addr.ip().to_string()),
                fingerprint: Some(konnekta_crypto::cert_fingerprint(&l.handle.peer_cert)),
                permissions: BTreeSet::new(),
                disabled_plugins: BTreeSet::new(),
                incoming_capabilities: l.handle.identity.incoming_capabilities.clone(),
                outgoing_capabilities: l.handle.identity.outgoing_capabilities.clone(),
            });
        }
        out
    }

    fn emit_updated(&self, id: &str) {
        if let Some(info) = self.device_info(id) {
            let _ = self.events.send(Event::DeviceUpdated { device: info });
        }
    }

    fn link_handle(&self, id: &str) -> Option<(DeviceHandle, PairState)> {
        self.links
            .lock()
            .unwrap()
            .get(id)
            .map(|l| (l.handle.clone(), l.pair))
    }

    fn set_pair_state(&self, id: &str, state: PairState) {
        if let Some(l) = self.links.lock().unwrap().get_mut(id) {
            l.pair = state;
        }
    }

    // --- dispatch --------------------------------------------------------------

    async fn dispatch(self: &Arc<Self>, device_id: &str, link_id: u64, packet: Packet) {
        let Some((handle, state)) = self.link_handle(device_id) else {
            return;
        };
        if handle.link_id != link_id || packet.packet_type.is_empty() {
            return;
        }
        match packet.packet_type.as_str() {
            types::PAIR => {
                self.clone().handle_pair(handle, state, packet).await;
                return;
            }
            types::IDENTITY => return,
            _ => {}
        }
        if !state.is_paired() || !self.trust.lock().unwrap().is_trusted(device_id) {
            tracing::warn!(device = %device_id, packet = %packet.packet_type, "dropping packet from unpaired device");
            // Tell the peer we consider it unpaired so its state resyncs.
            let _ = handle
                .tx
                .send(Packet::new(
                    types::PAIR,
                    PairBody {
                        pair: false,
                        timestamp: None,
                    },
                ))
                .await;
            return;
        }
        let disabled_global = self.config.read().unwrap().disabled_plugins.clone();
        let (perms, disabled_dev) = {
            let trust = self.trust.lock().unwrap();
            let d = trust.get(device_id);
            (
                d.map(|d| d.permissions.clone()).unwrap_or_default(),
                d.map(|d| d.disabled_plugins.clone()).unwrap_or_default(),
            )
        };
        let mut handled = false;
        for p in &self.plugins {
            let info = p.info();
            if !info.incoming.contains(&packet.packet_type.as_str()) {
                continue;
            }
            handled = true;
            if disabled_global.contains(info.id) || disabled_dev.contains(info.id) {
                tracing::debug!(plugin = info.id, "plugin disabled, dropping packet");
                continue;
            }
            if !perms.contains(&info.permission) {
                self.warn_denied(device_id, info.permission, &packet.packet_type);
                continue;
            }
            if let Err(e) = p.handle_packet(&handle, packet.clone()).await {
                tracing::warn!(plugin = info.id, "packet handling failed: {e:#}");
            }
        }
        if !handled {
            tracing::debug!(packet = %packet.packet_type, "no plugin for packet type");
        }
    }

    fn warn_denied(&self, device_id: &str, perm: Permission, packet_type: &str) {
        let first = self
            .links
            .lock()
            .unwrap()
            .get_mut(device_id)
            .map(|l| l.denied.insert(perm))
            .unwrap_or(false);
        tracing::info!(device = %device_id, ?perm, packet_type, "permission denied");
        if first {
            let _ = self.events.send(Event::SecurityWarning {
                device_id: Some(device_id.to_owned()),
                message: format!(
                    "Blocked a '{}' request: permission \"{}\" is not granted for this device.",
                    packet_type,
                    perm.label()
                ),
            });
        }
    }

    // --- pairing -----------------------------------------------------------------

    fn vkey(&self, handle: &DeviceHandle, ts: u64) -> String {
        verification_key(&self.identity.cert_der, &handle.peer_cert, ts).unwrap_or_default()
    }

    async fn send_pair(&self, handle: &DeviceHandle, pair: bool, ts: Option<u64>) {
        let _ = handle
            .tx
            .send(Packet::new(
                types::PAIR,
                PairBody {
                    pair,
                    timestamp: ts,
                },
            ))
            .await;
    }

    async fn handle_pair(self: Arc<Self>, handle: DeviceHandle, state: PairState, packet: Packet) {
        let Ok(body) = packet.body_as::<PairBody>() else {
            return;
        };
        let id = handle.id().to_owned();
        if body.pair {
            match state {
                PairState::RequestedByUs { .. } => self.complete_pairing(&handle).await,
                PairState::Paired => self.send_pair(&handle, true, None).await,
                PairState::NotPaired | PairState::RequestedByPeer { .. } => {
                    let Some(ts) = body.timestamp else {
                        self.send_pair(&handle, false, None).await;
                        return;
                    };
                    if !pairing::timestamp_is_fresh(ts, pairing::now_secs()) {
                        self.send_pair(&handle, false, None).await;
                        let _ = self.events.send(Event::PairRejected {
                            device_id: id,
                            reason: "clocks differ too much between devices".into(),
                        });
                        return;
                    }
                    self.set_pair_state(
                        &id,
                        PairState::RequestedByPeer {
                            timestamp: ts,
                            since: Instant::now(),
                        },
                    );
                    let _ = self.events.send(Event::PairRequested {
                        device_id: id.clone(),
                        device_name: handle.name().to_owned(),
                        verification_key: self.vkey(&handle, ts),
                    });
                    self.spawn_pair_timeout(id, ts);
                }
            }
        } else {
            match state {
                PairState::RequestedByUs { .. } => {
                    self.set_pair_state(&id, PairState::NotPaired);
                    let _ = self.events.send(Event::PairRejected {
                        device_id: id,
                        reason: "rejected by the other device".into(),
                    });
                }
                PairState::RequestedByPeer { .. } => {
                    self.set_pair_state(&id, PairState::NotPaired);
                    let _ = self.events.send(Event::PairRejected {
                        device_id: id,
                        reason: "cancelled by the other device".into(),
                    });
                }
                PairState::Paired | PairState::NotPaired => {
                    if self.trust.lock().unwrap().is_trusted(&id) {
                        let _ = self.unpair(&id, false).await;
                    }
                }
            }
        }
    }

    fn spawn_pair_timeout(self: &Arc<Self>, id: String, ts: u64) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(pairing::PAIR_TIMEOUT).await;
            let Some(this) = weak.upgrade() else { return };
            let expired = matches!(
                this.link_handle(&id).map(|(_, s)| s),
                Some(PairState::RequestedByUs { timestamp, .. } | PairState::RequestedByPeer { timestamp, .. }) if timestamp == ts
            );
            if expired {
                this.set_pair_state(&id, PairState::NotPaired);
                let _ = this.events.send(Event::PairRejected {
                    device_id: id,
                    reason: "pairing request timed out".into(),
                });
            }
        });
    }

    async fn complete_pairing(self: &Arc<Self>, handle: &DeviceHandle) {
        let id = handle.id().to_owned();
        self.trust.lock().unwrap().insert(TrustedDevice {
            device_id: id.clone(),
            name: handle.name().to_owned(),
            device_type: handle.identity.device_type,
            certificate: TrustedDevice::encode_cert(&handle.peer_cert),
            permissions: Permission::defaults(),
            disabled_plugins: BTreeSet::new(),
            paired_at: konnekta_proto::packet::now_millis(),
            last_address: Some(handle.addr.ip()),
        });
        self.set_pair_state(&id, PairState::Paired);
        tracing::info!(device = %handle.name(), "paired");
        let _ = self.events.send(Event::Paired {
            device_id: id.clone(),
        });
        self.emit_updated(&id);
        self.notify_plugins_connected(handle).await;
    }

    async fn request_pair(self: Arc<Self>, id: &str) -> Result<String> {
        let (handle, state) = self.link_handle(id).context("device not connected")?;
        match state {
            PairState::Paired => bail!("already paired"),
            PairState::RequestedByPeer { .. } => {
                self.accept_pair(id).await?;
                return Ok(String::new());
            }
            _ => {}
        }
        let ts = pairing::now_secs();
        self.set_pair_state(
            id,
            PairState::RequestedByUs {
                timestamp: ts,
                since: Instant::now(),
            },
        );
        self.send_pair(&handle, true, Some(ts)).await;
        let key = self.vkey(&handle, ts);
        let _ = self.events.send(Event::PairOutgoing {
            device_id: id.to_owned(),
            verification_key: key.clone(),
        });
        self.spawn_pair_timeout(id.to_owned(), ts);
        Ok(key)
    }

    async fn accept_pair(self: Arc<Self>, id: &str) -> Result<()> {
        let (handle, state) = self.link_handle(id).context("device not connected")?;
        let PairState::RequestedByPeer { .. } = state else {
            bail!("no pending pairing request from this device");
        };
        if state.expired() {
            self.set_pair_state(id, PairState::NotPaired);
            bail!("pairing request expired");
        }
        self.send_pair(&handle, true, None).await;
        self.complete_pairing(&handle).await;
        Ok(())
    }

    async fn reject_pair(self: Arc<Self>, id: &str) -> Result<()> {
        let (handle, _) = self.link_handle(id).context("device not connected")?;
        self.send_pair(&handle, false, None).await;
        self.set_pair_state(id, PairState::NotPaired);
        Ok(())
    }

    async fn unpair(self: &Arc<Self>, id: &str, notify_peer: bool) -> Result<()> {
        let removed = self.trust.lock().unwrap().remove(id).is_some();
        if let Some((handle, _)) = self.link_handle(id) {
            if notify_peer {
                self.send_pair(&handle, false, None).await;
            }
            self.set_pair_state(id, PairState::NotPaired);
        }
        if removed {
            tracing::info!(device = %id, "unpaired");
            for p in &self.plugins {
                p.on_disconnected(id).await;
            }
            let _ = self.events.send(Event::Unpaired {
                device_id: id.to_owned(),
            });
        }
        Ok(())
    }
}
