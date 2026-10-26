//! The orchestrator: owns identity, trust store, links and plugins, and runs
//! the network side of both protocols.

use crate::config::Config;
use crate::device::{DeviceHandle, DeviceInfo};
use crate::discovery::{self, Announcement};
use crate::event::Event;
use crate::link::{self, SecureStream};
use crate::pairing::{self, Admission, PairState, RequestGuard};
use crate::permissions::Permission;
use crate::plugin::{CoreHandle, Plugin, PluginInfo};
use crate::transport::Transport;
use crate::trust::{TrustStore, TrustedDevice};
use crate::usb;
use crate::{bluetooth, device::Route};
use anyhow::{Context, Result, bail};
use konnekta_crypto::{FileKeyStore, KeyStore, LocalIdentity, PeerExpectation, verification_key};
use konnekta_proto::bodies::PairBody;
use konnekta_proto::{IdentityBody, Packet, Protocol, interop, types};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{broadcast, mpsc};
use tokio::task::AbortHandle;

const DIAL_INTERVAL: Duration = Duration::from_secs(5);
const SEND_QUEUE: usize = 512;
const JANITOR_INTERVAL: Duration = Duration::from_secs(2);
const USB_INTERVAL: Duration = Duration::from_secs(3);
/// Wait after a failed USB dial (phone without Konnekta running).
const USB_RETRY: Duration = Duration::from_secs(30);
const BT_INTERVAL: Duration = Duration::from_secs(3);
/// Wait after a failed Bluetooth dial (phone away, Konnekta not running).
/// Short: a phone that opens Konnekta also dials us, but an older app only
/// waits to be dialed.
const BT_RETRY: Duration = Duration::from_secs(8);
/// A device announced on the network this recently counts as reachable there.
const SEEN_ON_NETWORK: Duration = Duration::from_secs(150);

pub(crate) struct LinkEntry {
    handle: DeviceHandle,
    pair: PairState,
    tasks: Vec<AbortHandle>,
    denied: HashSet<Permission>,
    created: Instant,
    /// A pair request was seen on this link (it is not idle).
    pair_requested: bool,
}

#[derive(Default, Clone, Copy)]
struct Ports {
    tcp: u16,
    udp: u16,
    /// (tcp, udp) of each interop adapter, by [`adapter_index`].
    adapters: [(u16, u16); 2],
}

/// Slot of an interop adapter in per-adapter arrays.
fn adapter_index(p: Protocol) -> usize {
    match p {
        Protocol::KdeConnect => 0,
        Protocol::Flux => 1,
        Protocol::Konnekta => unreachable!("the native protocol is not an adapter"),
    }
}

pub(crate) struct Inner {
    pub(crate) identity: LocalIdentity,
    pub(crate) config: RwLock<Config>,
    pub(crate) data_dir: Option<PathBuf>,
    trust: Mutex<TrustStore>,
    links: Mutex<HashMap<String, LinkEntry>>,
    plugins: Vec<Arc<dyn Plugin>>,
    pub(crate) events: broadcast::Sender<Event>,
    ports: Mutex<Ports>,
    next_link_id: AtomicU64,
    udp: Mutex<Option<Arc<UdpSocket>>>,
    adapter_udp: Mutex<[Option<Arc<UdpSocket>>; 2]>,
    dialing: Mutex<HashMap<String, Instant>>,
    pending: Mutex<HashMap<IpAddr, usize>>,
    discovered: Mutex<HashMap<String, (Announcement, Instant)>>,
    guard: Mutex<RequestGuard>,
    background: Mutex<Vec<AbortHandle>>,
    mdns: Mutex<Option<discovery::mdns::Mdns>>,
    /// adb forward of the link of each phone attached by USB, by serial.
    usb_forwards: Mutex<HashMap<String, usb::Mapping>>,
    /// Bluetooth connections to paired phones, by address.
    bt_sessions: Mutex<HashMap<String, bluetooth::Session>>,
    /// Device behind each Bluetooth address, once a link came up.
    bt_known: Mutex<HashMap<String, String>>,
    /// BlueZ, opened on first use.
    bluez: tokio::sync::Mutex<Option<Arc<bluetooth::Bluez>>>,
    /// Phone: loopback port where the Android app bridges Bluetooth.
    bt_port: Mutex<Option<u16>>,
    /// Device behind each adb serial, once a link came up, and the serials
    /// attached now.
    usb_known: Mutex<HashMap<String, String>>,
    usb_attached: Mutex<HashSet<String>>,
    /// Ways each device says it can reach us (`konnekta.transports`), and
    /// what we last told it.
    remote_transports: Mutex<HashMap<String, Vec<Transport>>>,
    sent_transports: Mutex<HashMap<String, Vec<Transport>>>,
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
        let mut trust = match &self.data_dir {
            Some(d) => TrustStore::open(d.join("devices.json"))?,
            None => TrustStore::in_memory(),
        };
        let mut config = self.config;
        migrate_plugin_ids(self.data_dir.as_deref(), &mut config, &mut trust);
        let mut seen = HashSet::new();
        for p in &self.plugins {
            if !seen.insert(p.info().id) {
                bail!("duplicate plugin id {}", p.info().id);
            }
        }
        let (events, _) = broadcast::channel(1024);
        let inner = Arc::new(Inner {
            identity,
            config: RwLock::new(config),
            data_dir: self.data_dir,
            trust: Mutex::new(trust),
            links: Mutex::new(HashMap::new()),
            plugins: self.plugins,
            events,
            ports: Mutex::default(),
            next_link_id: AtomicU64::new(1),
            udp: Mutex::new(None),
            adapter_udp: Mutex::new([None, None]),
            dialing: Mutex::default(),
            pending: Mutex::default(),
            discovered: Mutex::default(),
            guard: Mutex::default(),
            background: Mutex::default(),
            mdns: Mutex::new(None),
            usb_forwards: Mutex::default(),
            bt_sessions: Mutex::default(),
            bt_known: Mutex::default(),
            bluez: tokio::sync::Mutex::new(None),
            bt_port: Mutex::new(None),
            usb_known: Mutex::default(),
            usb_attached: Mutex::default(),
            remote_transports: Mutex::default(),
            sent_transports: Mutex::default(),
        });
        Ok(Konnekta { inner })
    }
}

/// Plugin ids renamed when Konnekta got its own names (old, new).
const RENAMED_PLUGINS: &[(&str, &str)] = &[
    ("mpris", "media"),
    ("mousepad", "input"),
    ("runcommand", "commands"),
    ("telephony", "calls"),
    ("findmyphone", "finddevice"),
];

/// Move the data folder and the "disabled" marks of renamed plugins, once.
fn migrate_plugin_ids(dir: Option<&std::path::Path>, config: &mut Config, trust: &mut TrustStore) {
    let rename = |set: &mut std::collections::BTreeSet<String>| {
        let mut changed = false;
        for (old, new) in RENAMED_PLUGINS {
            if set.remove(*old) {
                set.insert((*new).to_owned());
                changed = true;
            }
        }
        changed
    };
    if rename(&mut config.disabled_plugins)
        && let Some(d) = dir
        && let Err(e) = config.save(&d.join("config.json"))
    {
        tracing::warn!("cannot save migrated config: {e:#}");
    }
    let ids: Vec<String> = trust
        .all()
        .filter(|t| {
            RENAMED_PLUGINS
                .iter()
                .any(|(old, _)| t.disabled_plugins.contains(*old))
        })
        .map(|t| t.device_id.clone())
        .collect();
    for id in ids {
        trust.update(&id, |t| {
            rename(&mut t.disabled_plugins);
        });
    }
    if let Some(plugins) = dir.map(|d| d.join("plugins")) {
        for (old, new) in RENAMED_PLUGINS {
            let (from, to) = (plugins.join(old), plugins.join(new));
            if from.is_dir() && !to.exists() {
                match std::fs::rename(&from, &to) {
                    Ok(()) => tracing::info!("moved plugin data {old} -> {new}"),
                    Err(e) => tracing::warn!("cannot move plugin data {old}: {e}"),
                }
            }
        }
    }
}

/// The running Konnekta instance. Cheap to clone.
#[derive(Clone)]
pub struct Konnekta {
    inner: Arc<Inner>,
}

/// Counts a connection in handshake; released on drop.
struct PendingGuard {
    inner: Arc<Inner>,
    ip: IpAddr,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let mut p = self.inner.pending.lock().unwrap();
        if let Some(n) = p.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                p.remove(&self.ip);
            }
        }
    }
}

async fn bind_tcp(bind: IpAddr, first: u16, last: u16) -> Result<TcpListener> {
    if first == 0 {
        return Ok(TcpListener::bind(SocketAddr::new(bind, 0)).await?);
    }
    for port in first..=last.max(first) {
        if let Ok(l) = TcpListener::bind(SocketAddr::new(bind, port)).await {
            return Ok(l);
        }
    }
    bail!("no free TCP port in {first}-{last}")
}

/// Parse `host`, `host:port`, `[v6]:port`.
pub fn parse_host_port(s: &str, default_port: u16) -> (String, u16) {
    let s = s.trim();
    if let Ok(sa) = s.parse::<SocketAddr>() {
        return (sa.ip().to_string(), sa.port());
    }
    if let Ok(ip) = s.parse::<IpAddr>() {
        return (ip.to_string(), default_port);
    }
    match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => match p.parse() {
            Ok(port) => (h.to_owned(), port),
            Err(_) => (s.to_owned(), default_port),
        },
        _ => (s.to_owned(), default_port),
    }
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

    /// Our certificate (DER, public).
    pub fn certificate_der(&self) -> Vec<u8> {
        self.inner.identity.cert_der.clone()
    }

    pub fn fingerprint(&self) -> String {
        self.inner.identity.fingerprint()
    }

    /// Native TCP port.
    pub fn tcp_port(&self) -> u16 {
        self.inner.ports.lock().unwrap().tcp
    }

    /// Phone: loopback port where the Android app bridges Bluetooth.
    pub fn bluetooth_port(&self) -> Option<u16> {
        *self.inner.bt_port.lock().unwrap()
    }

    /// Native UDP port.
    pub fn udp_port(&self) -> u16 {
        self.inner.ports.lock().unwrap().udp
    }

    /// Ports (tcp, udp) of an interop adapter; 0 when disabled.
    pub fn adapter_ports(&self, p: Protocol) -> (u16, u16) {
        self.inner.ports.lock().unwrap().adapters[adapter_index(p)]
    }

    pub fn config(&self) -> Config {
        self.inner.config.read().unwrap().clone()
    }

    /// Update configuration. Network settings take effect on restart.
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

    fn spawn_bg<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let t = tokio::spawn(fut);
        self.inner.background.lock().unwrap().push(t.abort_handle());
    }

    /// Start listening, discovery and the reconnect loop.
    pub async fn start(&self) -> Result<()> {
        let inner = &self.inner;
        let cfg = self.config();

        // Native protocol.
        let native_range = Protocol::Konnekta.tcp_ports();
        let listener = bind_tcp(cfg.bind_addr, cfg.tcp_port, *native_range.end()).await?;
        inner.ports.lock().unwrap().tcp = listener.local_addr()?.port();
        self.spawn_bg(accept_loop(
            Arc::downgrade(inner),
            listener,
            Protocol::Konnekta,
        ));

        // Interop adapters (opt-in).
        let mut protos = vec![(Protocol::Konnekta, cfg.udp_port)];
        for p in Protocol::FOREIGN {
            let Some(a) = cfg.adapter(p).filter(|a| a.enabled) else {
                continue;
            };
            match bind_tcp(cfg.bind_addr, a.tcp_port, *p.tcp_ports().end()).await {
                Ok(l) => {
                    inner.ports.lock().unwrap().adapters[adapter_index(p)].0 =
                        l.local_addr()?.port();
                    self.spawn_bg(accept_loop(Arc::downgrade(inner), l, p));
                    protos.push((p, a.udp_port));
                }
                Err(e) => tracing::warn!("{} interop unavailable: {e:#}", p.label()),
            }
        }
        tracing::info!(
            port = self.tcp_port(),
            kde = cfg.kde_connect.enabled,
            flux = cfg.flux.enabled,
            device_id = %self.device_id(),
            "listening"
        );

        // UDP discovery per protocol.
        for (proto, port) in protos {
            let addr = SocketAddr::new(cfg.bind_addr, if cfg.listen_udp { port } else { 0 });
            match discovery::bind_udp(addr) {
                Ok(sock) => {
                    let sock = Arc::new(sock);
                    let local = sock.local_addr()?.port();
                    {
                        let mut p = inner.ports.lock().unwrap();
                        match proto {
                            Protocol::Konnekta => p.udp = local,
                            p2 => p.adapters[adapter_index(p2)].1 = local,
                        }
                    }
                    match proto {
                        Protocol::Konnekta => *inner.udp.lock().unwrap() = Some(sock.clone()),
                        p => {
                            inner.adapter_udp.lock().unwrap()[adapter_index(p)] = Some(sock.clone())
                        }
                    }
                    if cfg.listen_udp {
                        let weak = Arc::downgrade(inner);
                        self.spawn_bg(async move {
                            let mut buf = vec![0u8; 16 * 1024];
                            loop {
                                let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                                    continue;
                                };
                                let Some(inner) = weak.upgrade() else { break };
                                if let Some(a) = discovery::parse_announcement(&buf[..n], from) {
                                    inner.on_announcement(a);
                                }
                            }
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("{} UDP discovery unavailable on {addr}: {e}", proto.label())
                }
            }
        }
        if cfg.announce {
            let weak = Arc::downgrade(inner);
            let interval = cfg.announce_interval_secs;
            self.spawn_bg(async move {
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
        }

        // mDNS.
        if cfg.mdns {
            match discovery::mdns::Mdns::start() {
                Ok(mut m) => {
                    let (tx, mut rx) = mpsc::unbounded_channel();
                    // Native service, plus Flux's when its adapter is on
                    // (Flux apps find computers through it).
                    for p in [Protocol::Konnekta, Protocol::Flux] {
                        if !cfg.speaks(p) {
                            continue;
                        }
                        let port = match p {
                            Protocol::Konnekta => self.tcp_port(),
                            p => self.adapter_ports(p).0,
                        };
                        if cfg.announce
                            && let Err(e) = m.advertise(
                                p,
                                self.device_id(),
                                &cfg.device_name,
                                cfg.device_type,
                                port,
                            )
                        {
                            tracing::warn!("{} mDNS advertisement failed: {e:#}", p.label());
                        }
                        if let Err(e) = m.browse(p, tx.clone()) {
                            tracing::warn!("{} mDNS browse failed: {e:#}", p.label());
                        }
                    }
                    *inner.mdns.lock().unwrap() = Some(m);
                    let weak = Arc::downgrade(inner);
                    self.spawn_bg(async move {
                        while let Some(a) = rx.recv().await {
                            let Some(inner) = weak.upgrade() else { break };
                            inner.on_announcement(a);
                        }
                    });
                }
                Err(e) => tracing::warn!("mDNS unavailable: {e:#}"),
            }
        }

        // Reconnect to paired devices (outgoing only: works behind a firewall
        // that blocks inbound connections, and through Tailscale).
        let weak = Arc::downgrade(inner);
        let every = Duration::from_secs(cfg.reconnect_interval_secs.max(1));
        self.spawn_bg(async move {
            loop {
                let Some(inner) = weak.upgrade() else { break };
                inner.clone().dial_paired().await;
                drop(inner);
                tokio::time::sleep(every).await;
            }
        });

        // Phones attached by USB cable (the computer runs adb).
        if cfg!(not(any(target_os = "android", target_os = "ios"))) {
            let weak = Arc::downgrade(inner);
            self.spawn_bg(async move {
                loop {
                    let Some(inner) = weak.upgrade() else { break };
                    inner.usb_tick().await;
                    drop(inner);
                    tokio::time::sleep(USB_INTERVAL).await;
                }
            });
        }

        // Phones reached over Bluetooth: the computer dials, the phone's
        // Android app bridges its RFCOMM socket to a loopback port of the core.
        if cfg!(target_os = "android") {
            match bluetooth::serve(bluetooth::phone_policy()).await {
                Ok((port, abort)) => {
                    *inner.bt_port.lock().unwrap() = Some(port);
                    inner.background.lock().unwrap().push(abort);
                }
                Err(e) => tracing::warn!("bluetooth bridge: {e:#}"),
            }
        } else if cfg!(target_os = "linux") {
            let weak = Arc::downgrade(inner);
            self.spawn_bg(async move {
                loop {
                    let Some(inner) = weak.upgrade() else { break };
                    inner.bt_tick().await;
                    drop(inner);
                    tokio::time::sleep(BT_INTERVAL).await;
                }
            });
        }

        // Pairing timeouts and idle unpaired links.
        let weak = Arc::downgrade(inner);
        self.spawn_bg(async move {
            loop {
                tokio::time::sleep(JANITOR_INTERVAL).await;
                let Some(inner) = weak.upgrade() else { break };
                inner.janitor().await;
            }
        });

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
        *self.inner.mdns.lock().unwrap() = None;
        let links: Vec<_> = self.inner.links.lock().unwrap().drain().collect();
        for (_, l) in links {
            for t in l.tasks {
                t.abort();
            }
        }
    }

    /// Announce ourselves now and dial paired devices.
    pub async fn refresh(&self) {
        self.inner.announce().await;
        self.inner.dialing.lock().unwrap().clear();
        self.inner.clone().dial_paired().await;
    }

    /// Connect to a device at a known address (`ip`, `ip:port`, host name).
    /// Native: dial it directly. With the adapters on, also dial a Flux
    /// device there and ask a KDE Connect device to connect back.
    pub async fn connect_to(&self, address: &str) -> Result<()> {
        let (host, port) = parse_host_port(address, *Protocol::Konnekta.tcp_ports().start());
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
            .await
            .with_context(|| format!("cannot resolve {host}"))?
            .collect();
        let addr = *addrs.first().context("no address")?;
        let cfg = self.config();
        if cfg.kde_connect.enabled {
            self.inner
                .ask_dial_back(Protocol::KdeConnect, addr.ip())
                .await;
        }
        let mut targets = vec![(Protocol::Konnekta, addr)];
        if cfg.flux.enabled {
            targets.push((
                Protocol::Flux,
                SocketAddr::new(addr.ip(), *Protocol::Flux.tcp_ports().start()),
            ));
        }
        for (proto, addr) in targets {
            let inner = self.inner.clone();
            tokio::spawn(async move {
                if let Err(e) = inner.clone().dial(proto, None, addr).await {
                    tracing::info!(%addr, "manual {} connection failed: {e:#}", proto.label());
                }
            });
        }
        Ok(())
    }

    /// Send our announcement to one UDP address (tests, networks without
    /// broadcast). `protocol` selects which identity is sent.
    pub async fn announce_to(&self, addr: SocketAddr, protocol: Protocol) -> Result<()> {
        let (sock, packet) = match protocol {
            Protocol::Konnekta => (
                self.inner.udp.lock().unwrap().clone(),
                self.inner.native_announcement(),
            ),
            p => (
                self.inner.adapter_udp.lock().unwrap()[adapter_index(p)].clone(),
                self.inner.identity_packet(p),
            ),
        };
        sock.context("UDP socket not bound")?
            .send_to(&packet.to_line(), addr)
            .await?;
        Ok(())
    }

    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.inner.device_infos()
    }

    pub fn device(&self, id: &str) -> Option<DeviceInfo> {
        self.devices().into_iter().find(|d| d.id == id)
    }

    /// Devices seen on the network that are not connected.
    pub fn discovered(&self) -> Vec<Value> {
        let links = self.inner.links.lock().unwrap();
        self.inner
            .discovered
            .lock()
            .unwrap()
            .values()
            .filter(|(a, _)| !links.contains_key(&a.device_id))
            .map(|(a, t)| {
                serde_json::json!({
                    "id": a.device_id, "name": a.device_name, "deviceType": a.device_type,
                    "protocol": a.protocol, "address": a.addr.to_string(), "secondsAgo": t.elapsed().as_secs(),
                })
            })
            .collect()
    }

    pub fn device_handle(&self, id: &str) -> Option<DeviceHandle> {
        self.inner.paired_handle(id)
    }

    pub async fn request_pair(&self, id: &str) -> Result<String> {
        self.inner.clone().request_pair(id).await
    }

    /// Accept an incoming request, or confirm a request the device accepted.
    /// `key`, when given, must match the key of the open pairing.
    pub async fn accept_pair(&self, id: &str, key: Option<&str>) -> Result<()> {
        self.inner.clone().accept_pair(id, key).await
    }

    pub async fn reject_pair(&self, id: &str, key: Option<&str>) -> Result<()> {
        self.inner.clone().reject_pair(id, key).await
    }

    pub async fn unpair(&self, id: &str) -> Result<()> {
        self.inner.clone().unpair(id, true).await
    }

    /// Choose how to reach a paired device; `None` = the fastest available.
    pub fn set_preferred_transport(&self, id: &str, t: Option<Transport>) -> Result<()> {
        self.inner.set_preferred_transport(id, t)
    }

    pub fn set_permission(&self, id: &str, perm: Permission, granted: bool) -> Result<()> {
        let ok = self.inner.trust.lock().unwrap().update(id, |d| {
            if granted {
                d.permissions.insert(perm);
            } else {
                d.permissions.remove(&perm);
            }
            d.permissions_changed_at = konnekta_proto::packet::now_millis();
        });
        if !ok {
            bail!("device not paired");
        }
        if let Some(e) = self.inner.links.lock().unwrap().get_mut(id) {
            e.denied.remove(&perm);
        }
        self.inner.emit_updated(id);
        self.inner.send_permissions(id);
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

    /// Addresses dialed when the device is away (Tailscale, VPN, ...).
    pub fn set_extra_addresses(&self, id: &str, addresses: Vec<String>) -> Result<()> {
        let clean: Vec<String> = addresses
            .into_iter()
            .map(|a| a.trim().to_owned())
            .filter(|a| {
                !a.is_empty()
                    && a.len() <= 255
                    && !a.chars().any(|c| c.is_whitespace() || c.is_control())
            })
            .take(8)
            .collect();
        if !self
            .inner
            .trust
            .lock()
            .unwrap()
            .update(id, |d| d.extra_addresses = clean)
        {
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

async fn accept_loop(weak: std::sync::Weak<Inner>, listener: TcpListener, proto: Protocol) {
    loop {
        let Ok((tcp, from)) = listener.accept().await else {
            continue;
        };
        let Some(inner) = weak.upgrade() else { break };
        let Some(guard) = inner.clone().admit_pending(from.ip()) else {
            tracing::debug!(%from, "too many connections in handshake, dropping");
            continue;
        };
        tokio::spawn(async move {
            let r = match proto {
                Protocol::Konnekta => inner.clone().handle_incoming_native(tcp, from).await,
                p => inner.clone().handle_incoming_v8(tcp, from, p).await,
            };
            drop(guard);
            if let Err(e) = r {
                tracing::debug!(%from, "incoming {} connection failed: {e:#}", proto.label());
            }
        });
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

    fn admit_pending(self: Arc<Self>, ip: IpAddr) -> Option<PendingGuard> {
        let limits = self.config.read().unwrap().limits.clone();
        {
            let mut p = self.pending.lock().unwrap();
            let total: usize = p.values().sum();
            let here = p.get(&ip).copied().unwrap_or(0);
            if total >= limits.pending_connections || here >= limits.pending_connections_per_addr {
                return None;
            }
            *p.entry(ip).or_default() += 1;
        }
        Some(PendingGuard { inner: self, ip })
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
        inc.insert(crate::payload::RESUME_CAPABILITY.to_string());
        (inc.into_iter().collect(), out.into_iter().collect())
    }

    /// Full identity, sent inside TLS (and also in clear for the adapters).
    pub(crate) fn identity_packet(&self, proto: Protocol) -> Packet {
        let cfg = self.config.read().unwrap();
        let (incoming, outgoing) = self.capabilities();
        let ports = *self.ports.lock().unwrap();
        let (incoming, outgoing, port) = match proto {
            Protocol::Konnekta => (incoming, outgoing, ports.tcp),
            p => (
                interop::caps_to_wire(p, &incoming),
                interop::caps_to_wire(p, &outgoing),
                ports.adapters[adapter_index(p)].0,
            ),
        };
        Packet::new(
            proto.identity_type(),
            IdentityBody {
                device_id: self.identity.device_id.clone(),
                device_name: cfg.device_name.clone(),
                device_type: cfg.device_type,
                protocol_version: proto.version(),
                incoming_capabilities: incoming,
                outgoing_capabilities: outgoing,
                tcp_port: Some(port),
                app: (proto != Protocol::KdeConnect).then(|| "Konnekta".into()),
                app_version: (proto != Protocol::KdeConnect)
                    .then(|| env!("CARGO_PKG_VERSION").into()),
                usb: false,
                bluetooth: false,
            },
        )
    }

    fn native_announcement(&self) -> Packet {
        let cfg = self.config.read().unwrap();
        discovery::native_announcement(
            &self.identity.device_id,
            &cfg.device_name,
            cfg.device_type,
            self.ports.lock().unwrap().tcp,
        )
    }

    async fn announce(&self) {
        let (targets, cfg) = {
            let c = self.config.read().unwrap();
            (discovery::expand_targets(&c.broadcast_targets), c.clone())
        };
        let udp = self.udp.lock().unwrap().clone();
        if let Some(sock) = udp {
            let port = Protocol::Konnekta.udp_port();
            let t: Vec<SocketAddr> = targets
                .iter()
                .map(|ip| SocketAddr::new(*ip, port))
                .collect();
            discovery::announce(&sock, &self.native_announcement(), &t).await;
        }
        for p in Protocol::FOREIGN.into_iter().filter(|p| cfg.speaks(*p)) {
            let udp = self.adapter_udp.lock().unwrap()[adapter_index(p)].clone();
            if let Some(sock) = udp {
                let t: Vec<SocketAddr> = targets
                    .iter()
                    .map(|ip| SocketAddr::new(*ip, p.udp_port()))
                    .collect();
                discovery::announce(&sock, &self.identity_packet(p), &t).await;
            }
        }
    }

    /// KDE Connect devices connect back after receiving our identity.
    async fn ask_dial_back(&self, p: Protocol, ip: IpAddr) {
        let sock = self.adapter_udp.lock().unwrap()[adapter_index(p)].clone();
        if let Some(sock) = sock {
            let _ = sock
                .send_to(
                    &self.identity_packet(p).to_line(),
                    SocketAddr::new(ip, p.udp_port()),
                )
                .await;
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

    fn should_dial(&self, key: &str) -> bool {
        let mut d = self.dialing.lock().unwrap();
        if d.get(key).is_some_and(|t| t.elapsed() < DIAL_INTERVAL) {
            return false;
        }
        d.insert(key.to_owned(), Instant::now());
        true
    }

    fn on_announcement(self: &Arc<Self>, a: Announcement) {
        if a.device_id == self.identity.device_id {
            return;
        }
        if !self.config.read().unwrap().speaks(a.protocol) {
            return;
        }
        {
            let max = self.config.read().unwrap().limits.discovered_devices;
            let mut d = self.discovered.lock().unwrap();
            if !d.contains_key(&a.device_id)
                && d.len() >= max
                && let Some(oldest) = d
                    .iter()
                    .min_by_key(|(_, (_, t))| *t)
                    .map(|(k, _)| k.clone())
            {
                d.remove(&oldest);
            }
            d.insert(a.device_id.clone(), (a.clone(), Instant::now()));
        }
        if !self.wants(&a.device_id, Transport::Wifi) {
            return;
        }
        // The dial still has to pass the certificate pin, so discovery can
        // never redirect a paired device to an impostor.
        if !self.should_dial(&a.device_id) {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this
                .dial(a.protocol, Some(a.device_id.clone()), a.addr)
                .await
            {
                tracing::debug!(addr = %a.addr, "dial failed: {e:#}");
            }
        });
    }

    /// Dial `addr`. `expected` is the device id when known.
    async fn dial(
        self: Arc<Self>,
        proto: Protocol,
        expected: Option<String>,
        addr: SocketAddr,
    ) -> Result<()> {
        let expect = match &expected {
            Some(id) => self.expectation_for(id),
            None => PeerExpectation::Deferred,
        };
        let tls = match proto {
            Protocol::Konnekta => link::dial_native(&self.identity, addr, expect).await,
            p => {
                let me = self.identity_packet(p);
                link::dial_v8(&self.identity, &me, addr, expect).await
            }
        };
        match tls {
            Ok(tls) => self.establish(tls, proto, expected, addr, true, None).await,
            Err(e) => {
                if let Some(id) = &expected {
                    self.report_tls_failure(id, &e);
                }
                Err(e)
            }
        }
    }

    /// Dial the phones attached by USB that have no USB link yet.
    async fn usb_tick(self: &Arc<Self>) {
        if !self.config.read().unwrap().usb || usb::find_adb().is_none() {
            return;
        }
        let serials = usb::devices().await;
        self.usb_forwards
            .lock()
            .unwrap()
            .retain(|s, _| s.starts_with("bt:") || serials.contains(s));
        *self.usb_attached.lock().unwrap() = serials.iter().cloned().collect();
        for serial in serials {
            let linked = self
                .links
                .lock()
                .unwrap()
                .values()
                .any(|l| matches!(&l.handle.route, Some(Route::Adb(s)) if **s == *serial));
            // A known phone already on a better (or the chosen) way stays there.
            let known = self.usb_known.lock().unwrap().get(&serial).cloned();
            let better = known.is_some_and(|id| !self.wants(&id, Transport::Usb));
            let key = format!("usb:{serial}");
            if linked || better || !self.should_dial(&key) {
                continue;
            }
            let this = self.clone();
            tokio::spawn(async move {
                if let Err(e) = this.clone().dial_usb(serial.into()).await {
                    tracing::debug!(%key, "USB dial failed: {e:#}");
                    this.dialing
                        .lock()
                        .unwrap()
                        .insert(key, Instant::now() + USB_RETRY);
                }
            });
        }
    }

    /// Reach the phone's listener through an adb forward and dial it.
    async fn dial_usb(self: Arc<Self>, serial: Arc<str>) -> Result<()> {
        let mut last = None;
        for port in Protocol::Konnekta.tcp_ports().take(3) {
            let (addr, mapping) = usb::forward(&serial, port).await?;
            match link::dial_native(&self.identity, addr, PeerExpectation::Deferred).await {
                Ok(tls) => {
                    self.usb_forwards
                        .lock()
                        .unwrap()
                        .insert(serial.to_string(), mapping);
                    return self
                        .establish(
                            tls,
                            Protocol::Konnekta,
                            None,
                            addr,
                            true,
                            Some(Route::Adb(serial)),
                        )
                        .await;
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.context("no port")?)
    }

    /// Connect to paired phones over Bluetooth and dial their listener.
    async fn bt_tick(self: &Arc<Self>) {
        if !self.config.read().unwrap().bluetooth {
            return;
        }
        let bluez = {
            let mut g = self.bluez.lock().await;
            if g.is_none() {
                match bluetooth::open_bluez().await {
                    Ok(b) => {
                        let b = Arc::new(b);
                        self.clone().take_bt_incoming(b.clone());
                        *g = Some(b);
                    }
                    Err(e) => {
                        tracing::debug!("bluetooth unavailable: {e:#}");
                        return;
                    }
                }
            }
            g.clone().unwrap()
        };
        self.bt_sessions
            .lock()
            .unwrap()
            .retain(|_, s| !s.is_closed());
        for peer in bluez.peers().await {
            let key = format!("bt:{}", peer.address);
            // Unknown phones are tried (they may run Konnekta); a known one
            // only when Bluetooth would be better than its current link.
            let known = self.bt_known.lock().unwrap().get(&peer.address).cloned();
            let skip = known.is_some_and(|id| !self.wants(&id, Transport::Bluetooth));
            if skip || !self.should_dial(&key) {
                continue;
            }
            let this = self.clone();
            let bluez = bluez.clone();
            tokio::spawn(async move {
                if let Err(e) = this.clone().dial_bluetooth(&bluez, &peer.address).await {
                    tracing::debug!(phone = %peer.name, "bluetooth dial failed: {e:#}");
                    this.dialing
                        .lock()
                        .unwrap()
                        .insert(key, Instant::now() + BT_RETRY);
                }
            });
        }
    }

    /// Phones that dial us over Bluetooth (when Konnekta opens on them):
    /// the link starts at once, through the same multiplexer as our dials.
    fn take_bt_incoming(self: Arc<Self>, bluez: Arc<bluetooth::Bluez>) {
        let weak = Arc::downgrade(&self);
        drop(self);
        tokio::spawn(async move {
            while let Some(session) = bluez.next_incoming().await {
                let Some(this) = weak.upgrade() else { break };
                let Some(address) = session.address().map(str::to_owned) else {
                    continue;
                };
                if !this.config.read().unwrap().bluetooth {
                    continue;
                }
                let known = this.bt_known.lock().unwrap().get(&address).cloned();
                if known.is_some_and(|id| !this.wants(&id, Transport::Bluetooth)) {
                    // Already linked a better way: keep the session for later.
                    this.bt_sessions.lock().unwrap().insert(address, session);
                    continue;
                }
                this.bt_sessions
                    .lock()
                    .unwrap()
                    .insert(address.clone(), session);
                let b = bluez.clone();
                tokio::spawn(async move {
                    if let Err(e) = this.clone().dial_bluetooth(&b, &address).await {
                        tracing::debug!(phone = %address, "bluetooth link from the phone failed: {e:#}");
                    }
                });
            }
        });
    }

    async fn dial_bluetooth(
        self: Arc<Self>,
        bluez: &bluetooth::Bluez,
        address: &str,
    ) -> Result<()> {
        let existing = self.bt_sessions.lock().unwrap().get(address).cloned();
        let session = match existing {
            Some(s) => s,
            None => {
                let s = bluez.connect(address).await?;
                self.bt_sessions
                    .lock()
                    .unwrap()
                    .insert(address.to_owned(), s.clone());
                s
            }
        };
        let mut last = None;
        for port in Protocol::Konnekta.tcp_ports().take(3) {
            let (addr, guard) = session.forward(port).await?;
            match link::dial_native(&self.identity, addr, PeerExpectation::Deferred).await {
                Ok(tls) => {
                    self.usb_forwards
                        .lock()
                        .unwrap()
                        .insert(format!("bt:{address}"), usb::Mapping::Bluetooth(guard));
                    let id = self
                        .clone()
                        .establish(
                            tls,
                            Protocol::Konnekta,
                            None,
                            addr,
                            true,
                            Some(Route::Bluetooth(session.clone())),
                        )
                        .await;
                    return id;
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.context("no port")?)
    }

    async fn dial_paired(self: Arc<Self>) {
        let cfg = self.config.read().unwrap().clone();
        let trusted: Vec<TrustedDevice> = self.trust.lock().unwrap().all().cloned().collect();
        for d in trusted {
            if !cfg.speaks(d.protocol) || !self.wants(&d.device_id, Transport::Wifi) {
                continue;
            }
            if d.protocol == Protocol::KdeConnect {
                if let Some(ip) = d.last_address {
                    self.ask_dial_back(Protocol::KdeConnect, ip).await;
                }
                continue;
            }
            // Konnekta and Flux devices listen: dial them.
            if !self.should_dial(&format!("paired:{}", d.device_id)) {
                continue;
            }
            let proto = d.protocol;
            let default_port = *proto.tcp_ports().start();
            let mut candidates: Vec<(String, u16)> = Vec::new();
            if let (Some(ip), Some(port)) = (d.last_address, d.last_port) {
                candidates.push((ip.to_string(), port));
            }
            for a in &d.extra_addresses {
                candidates.push(parse_host_port(a, d.last_port.unwrap_or(default_port)));
            }
            let this = self.clone();
            tokio::spawn(async move {
                for (host, port) in candidates {
                    let Ok(addrs) = tokio::net::lookup_host((host.as_str(), port)).await else {
                        continue;
                    };
                    for addr in addrs.take(2) {
                        if !this.wants(&d.device_id, Transport::Wifi) {
                            return;
                        }
                        match this
                            .clone()
                            .dial(proto, Some(d.device_id.clone()), addr)
                            .await
                        {
                            Ok(()) => return,
                            Err(e) => {
                                tracing::debug!(device = %d.name, %addr, "reconnect failed: {e:#}")
                            }
                        }
                    }
                }
            });
        }
    }

    async fn handle_incoming_native(
        self: Arc<Self>,
        tcp: TcpStream,
        from: SocketAddr,
    ) -> Result<()> {
        let tls = link::accept_native(&self.identity, tcp).await?;
        self.establish(tls, Protocol::Konnekta, None, from, false, None)
            .await
    }

    async fn handle_incoming_v8(
        self: Arc<Self>,
        mut tcp: TcpStream,
        from: SocketAddr,
        proto: Protocol,
    ) -> Result<()> {
        let id = link::read_plain_identity(&mut tcp, proto).await?;
        if id.device_id == self.identity.device_id {
            bail!("connection from ourselves");
        }
        let expect = self.expectation_for(&id.device_id);
        match link::accept_v8(&self.identity, tcp, expect).await {
            Ok(tls) => {
                self.establish(tls, proto, Some(id.device_id), from, false, None)
                    .await
            }
            Err(e) => {
                self.report_tls_failure(&id.device_id, &e);
                Err(e)
            }
        }
    }

    fn report_tls_failure(&self, device_id: &str, e: &anyhow::Error) {
        let msg = format!("{e:#}");
        if self.trust.lock().unwrap().is_trusted(device_id)
            && msg.to_lowercase().contains("certificate")
        {
            self.security_warning(Some(device_id), PIN_MISMATCH);
        }
        tracing::debug!(device_id, "tls failed: {msg}");
    }

    fn security_warning(&self, device_id: Option<&str>, message: &str) {
        tracing::warn!(device = ?device_id, "{message}");
        let _ = self.events.send(Event::SecurityWarning {
            device_id: device_id.map(str::to_owned),
            message: message.to_owned(),
        });
    }

    /// Finish a link after TLS: identity exchange and all trust checks, then
    /// start the reader and writer tasks. `adb` is the serial of the phone
    /// when we dialed it through a USB cable.
    async fn establish(
        self: Arc<Self>,
        tls: SecureStream,
        proto: Protocol,
        expected: Option<String>,
        addr: SocketAddr,
        outgoing: bool,
        route: Option<Route>,
    ) -> Result<()> {
        let peer_cert = link::peer_certificate(&tls).context("peer sent no certificate")?;
        let mut stream = BufReader::new(tls);
        let mut me = self.identity_packet(proto);
        match &route {
            Some(Route::Adb(_)) => {
                me.body.insert("usb".into(), Value::Bool(true));
            }
            Some(Route::Bluetooth(_)) => {
                me.body.insert("bluetooth".into(), Value::Bool(true));
            }
            None => {}
        }
        let identity = link::exchange_identity(&mut stream, &me, proto).await?;
        // The dialer says it came through adb; only believable from loopback.
        let from_loopback = !outgoing && addr.ip().is_loopback();
        let usb = matches!(route, Some(Route::Adb(_))) || (identity.usb && from_loopback);
        let bluetooth =
            matches!(route, Some(Route::Bluetooth(_))) || (identity.bluetooth && from_loopback);
        let device_id = identity.device_id.clone();
        if expected.as_ref().is_some_and(|e| *e != device_id) {
            bail!("device answered with another identity");
        }
        if device_id == self.identity.device_id {
            bail!("connection to ourselves");
        }
        // The certificate must belong to the identity (TLS-first links could
        // not check this during the handshake).
        if konnekta_crypto::cert_common_name(&peer_cert)
            .ok()
            .as_deref()
            != Some(device_id.as_str())
        {
            bail!("certificate does not match the device identity");
        }
        let pinned = self
            .trust
            .lock()
            .unwrap()
            .get(&device_id)
            .map(TrustedDevice::cert_der);
        if let Some(pin) = &pinned
            && *pin != peer_cert
        {
            self.security_warning(Some(&device_id), PIN_MISMATCH);
            bail!("certificate pin mismatch");
        }
        let trusted = pinned.is_some();

        let (rd, mut wr) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::channel::<Packet>(SEND_QUEUE);
        let link_id = self.next_link_id.fetch_add(1, Ordering::Relaxed);
        let handle = DeviceHandle {
            inner: Arc::downgrade(&self),
            link_id,
            tx,
            identity: Arc::new(identity.clone()),
            addr,
            protocol: proto,
            peer_cert: Arc::new(peer_cert.clone()),
            outgoing,
            usb,
            bluetooth,
            route,
        };

        // Decide whether this link may replace an existing one.
        let mut evicted = Vec::new();
        {
            let links = self.links.lock().unwrap();
            if let Some(old) = links.get(&device_id) {
                if old.pair.is_open() && *old.handle.peer_cert != peer_cert {
                    bail!("a pairing with another certificate is in progress for this device");
                }
                let dialer = |out: bool| {
                    if out {
                        self.identity.device_id.as_str()
                    } else {
                        device_id.as_str()
                    }
                };
                let (old_dialer, new_dialer) = (dialer(old.handle.outgoing), dialer(outgoing));
                // A cable beats the network, both ways. Otherwise, for
                // simultaneous connections both sides keep the one opened by
                // the smaller device id. Same dialer: the newer one wins.
                let preferred = self
                    .trust
                    .lock()
                    .unwrap()
                    .get(&device_id)
                    .and_then(|d| d.preferred_transport);
                let (old_rank, new_rank) = (
                    old.handle.transport().rank(preferred),
                    Transport::of_link(usb, bluetooth).rank(preferred),
                );
                if old_rank > new_rank {
                    tracing::debug!(device = %identity.device_name, "keeping the faster link");
                    return Ok(());
                }
                if old_rank == new_rank && old_dialer != new_dialer && old_dialer < new_dialer {
                    tracing::debug!(device = %identity.device_name, "keeping existing link");
                    return Ok(());
                }
            }
            if !trusted {
                let limits = self.config.read().unwrap().limits.clone();
                let unpaired: Vec<(String, IpAddr, Instant)> = links
                    .iter()
                    .filter(|(id, l)| **id != device_id && !l.pair.is_paired())
                    .map(|(id, l)| (id.clone(), l.handle.addr.ip(), l.created))
                    .collect();
                let same_addr: Vec<_> = unpaired
                    .iter()
                    .filter(|(_, ip, _)| *ip == addr.ip())
                    .collect();
                if same_addr.len() >= limits.unpaired_links_per_addr {
                    if let Some(o) = same_addr.iter().min_by_key(|(_, _, t)| *t) {
                        evicted.push(o.0.clone());
                    }
                } else if unpaired.len() >= limits.unpaired_links
                    && let Some(o) = unpaired.iter().min_by_key(|(_, _, t)| *t)
                {
                    evicted.push(o.0.clone());
                }
            }
        }
        for id in evicted {
            tracing::info!(device = %id, "closing the oldest unpaired link (limit reached)");
            self.close_link(&id).await;
        }

        let writer = tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                let Some(p) = link::to_wire(p, proto) else {
                    continue;
                };
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
                let max = if this.is_paired(&rid) {
                    link::MAX_PAIRED_LINE
                } else {
                    link::MAX_UNPAIRED_LINE
                };
                match link::read_packet(&mut rd, proto, max).await {
                    Ok(Some(Some(p))) => this.dispatch(&rid, link_id, p).await,
                    Ok(Some(None)) => {}
                    Ok(None) => break,
                    Err(e) => {
                        tracing::debug!(device = %rid, "link read error: {e:#}");
                        break;
                    }
                }
            }
            this.on_link_closed(&rid, link_id).await;
        });

        let entry = LinkEntry {
            handle: handle.clone(),
            pair: if trusted {
                PairState::Paired
            } else {
                PairState::NotPaired
            },
            tasks: vec![reader.abort_handle(), writer.abort_handle()],
            denied: HashSet::new(),
            created: Instant::now(),
            pair_requested: false,
        };
        let old = self.links.lock().unwrap().insert(device_id.clone(), entry);
        if let Some(old) = old {
            for t in &old.tasks {
                t.abort();
            }
            // Another way now (e.g. Wi-Fi to Bluetooth): what ran on the old
            // link, like a stream, ends with it.
            if old.handle.transport() != handle.transport() && old.pair.is_paired() {
                for p in &self.plugins {
                    p.on_disconnected(&device_id).await;
                }
            }
        }
        // Remember which device sits behind the cable or the Bluetooth address.
        match &handle.route {
            Some(Route::Adb(serial)) => {
                self.usb_known
                    .lock()
                    .unwrap()
                    .insert(serial.to_string(), device_id.clone());
            }
            Some(Route::Bluetooth(s)) => {
                if let Some(a) = s.address() {
                    self.bt_known
                        .lock()
                        .unwrap()
                        .insert(a.to_owned(), device_id.clone());
                }
            }
            None => {}
        }
        if trusted {
            self.trust.lock().unwrap().update(&device_id, |d| {
                d.name = identity.device_name.clone();
                d.device_type = identity.device_type;
                // Over USB the address is our own loopback.
                if !usb && !bluetooth {
                    d.last_address = Some(addr.ip());
                }
                if proto == Protocol::Konnekta {
                    d.last_port = identity.tcp_port.or(d.last_port);
                }
            });
        }
        tracing::info!(device = %identity.device_name, %addr, usb, bluetooth, protocol = proto.label(), paired = trusted, "device connected");
        if let Some(info) = self.device_info(&device_id) {
            let _ = self.events.send(Event::DeviceConnected { device: info });
        }
        if trusted {
            self.notify_plugins_connected(&handle).await;
        }
        Ok(())
    }

    async fn close_link(&self, device_id: &str) {
        let removed = self.links.lock().unwrap().remove(device_id);
        if let Some(entry) = removed {
            for t in &entry.tasks {
                t.abort();
            }
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
            if entry.pair.is_open() {
                let _ = self.events.send(Event::PairRejected {
                    device_id: device_id.to_owned(),
                    reason: "the connection closed during pairing".into(),
                });
                if matches!(entry.pair, PairState::RequestedByPeer { .. }) {
                    self.cool_down(device_id);
                }
            }
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
        self.send_permissions(handle.id());
        self.sent_transports.lock().unwrap().remove(handle.id());
        self.send_transports(handle.id());
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

    fn link_key(&self, l: &LinkEntry) -> Option<String> {
        let ts = l.pair.timestamp()?;
        verification_key(
            &self.identity.cert_der,
            &l.handle.peer_cert,
            ts,
            l.handle.protocol.key_len(),
        )
        .ok()
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
                usb: link.is_some_and(|l| l.handle.usb),
                bluetooth: link.is_some_and(|l| l.handle.bluetooth),
                transport: link.map(|l| l.handle.transport()),
                transports: self.transports_of(&t.device_id, link),
                preferred_transport: t.preferred_transport,
                fingerprint: Some(konnekta_crypto::cert_fingerprint(&t.cert_der())),
                permissions: t.permissions.clone(),
                disabled_plugins: t.disabled_plugins.clone(),
                incoming_capabilities: link
                    .map(|l| l.handle.identity.incoming_capabilities.clone())
                    .unwrap_or_default(),
                outgoing_capabilities: link
                    .map(|l| l.handle.identity.outgoing_capabilities.clone())
                    .unwrap_or_default(),
                protocol: link.map(|l| l.handle.protocol).unwrap_or(t.protocol),
                pair_key: None,
                extra_addresses: t.extra_addresses.clone(),
                app: link.and_then(|l| l.handle.identity.app.clone()),
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
                usb: l.handle.usb,
                bluetooth: l.handle.bluetooth,
                transport: Some(l.handle.transport()),
                transports: self.transports_of(id, Some(l)),
                preferred_transport: None,
                fingerprint: Some(konnekta_crypto::cert_fingerprint(&l.handle.peer_cert)),
                permissions: BTreeSet::new(),
                disabled_plugins: BTreeSet::new(),
                incoming_capabilities: l.handle.identity.incoming_capabilities.clone(),
                outgoing_capabilities: l.handle.identity.outgoing_capabilities.clone(),
                protocol: l.handle.protocol,
                pair_key: self.link_key(l),
                extra_addresses: Vec::new(),
                app: l.handle.identity.app.clone(),
            });
        }
        out
    }

    // --- transports ----------------------------------------------------------------
    //
    // The ways a device can be reached, the one the pair chose (synced like
    // permissions: the most recent choice wins) and the switch to it.

    /// The chosen transport of a paired device.
    fn preferred_transport(&self, id: &str) -> Option<Transport> {
        self.trust
            .lock()
            .unwrap()
            .get(id)
            .and_then(|d| d.preferred_transport)
    }

    /// Whether a link over `t` would be worth opening for `id`: none yet,
    /// or the current one ranks lower.
    fn wants(&self, id: &str, t: Transport) -> bool {
        let preferred = self.preferred_transport(id);
        match self.links.lock().unwrap().get(id) {
            None => true,
            Some(l) => l.handle.transport().rank(preferred) < t.rank(preferred),
        }
    }

    /// Ways we can reach `id` ourselves, now.
    fn local_transports(&self, id: &str, link: Option<&LinkEntry>) -> BTreeSet<Transport> {
        let mut out = BTreeSet::new();
        if let Some(l) = link {
            out.insert(l.handle.transport());
        }
        if self
            .discovered
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|(_, at)| at.elapsed() < SEEN_ON_NETWORK)
        {
            out.insert(Transport::Wifi);
        }
        {
            let attached = self.usb_attached.lock().unwrap();
            if self
                .usb_known
                .lock()
                .unwrap()
                .iter()
                .any(|(serial, d)| d == id && attached.contains(serial))
            {
                out.insert(Transport::Usb);
            }
        }
        {
            let sessions = self.bt_sessions.lock().unwrap();
            if self
                .bt_known
                .lock()
                .unwrap()
                .iter()
                .any(|(a, d)| d == id && sessions.get(a).is_some_and(|s| !s.is_closed()))
            {
                out.insert(Transport::Bluetooth);
            }
        }
        out
    }

    /// Ways `id` can be reached: ours and those it reported.
    fn transports_of(&self, id: &str, link: Option<&LinkEntry>) -> Vec<Transport> {
        let mut all = self.local_transports(id, link);
        if link.is_some()
            && let Some(r) = self.remote_transports.lock().unwrap().get(id)
        {
            all.extend(r.iter().copied());
        }
        all.into_iter().collect()
    }

    /// Tell `id` the ways we reach it and the pair's choice.
    fn send_transports(&self, id: &str) {
        let links = self.links.lock().unwrap();
        let Some(l) = links.get(id) else {
            return;
        };
        if !l.pair.is_paired() || l.handle.protocol != Protocol::Konnekta {
            return;
        }
        let available: Vec<Transport> = self.local_transports(id, Some(l)).into_iter().collect();
        let (preferred, at) = self
            .trust
            .lock()
            .unwrap()
            .get(id)
            .map(|d| (d.preferred_transport, d.transport_changed_at))
            .unwrap_or_default();
        let p = Packet::new(
            types::TRANSPORTS,
            serde_json::json!({ "available": available, "preferred": preferred, "changedAt": at }),
        );
        if l.handle.tx.try_send(p).is_ok() {
            self.sent_transports
                .lock()
                .unwrap()
                .insert(id.to_owned(), available);
        }
    }

    fn on_transports(self: &Arc<Self>, id: &str, packet: &Packet) {
        let available: Vec<Transport> = packet
            .body
            .get("available")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| serde_json::from_value(x.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        self.remote_transports
            .lock()
            .unwrap()
            .insert(id.to_owned(), available);
        let theirs_at = packet
            .body
            .get("changedAt")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let theirs: Option<Transport> = packet
            .body
            .get("preferred")
            .and_then(|v| v.as_str())
            .and_then(Transport::parse);
        let mine = self
            .trust
            .lock()
            .unwrap()
            .get(id)
            .map(|d| (d.preferred_transport, d.transport_changed_at));
        if let Some((mine, mine_at)) = mine {
            if theirs_at > mine_at && theirs != mine {
                tracing::info!(device = %id, ?theirs, "the pair chose another way to connect");
                self.trust.lock().unwrap().update(id, |d| {
                    d.preferred_transport = theirs;
                    d.transport_changed_at = theirs_at;
                });
                self.switch_soon();
            } else if mine_at > theirs_at {
                // Ours is newer: they learn it.
                self.sent_transports.lock().unwrap().remove(id);
                self.send_transports(id);
            }
        }
        self.emit_updated(id);
    }

    /// Choose how to reach a paired device (none = the fastest available)
    /// and switch to it when it is available.
    pub(crate) fn set_preferred_transport(
        self: &Arc<Self>,
        id: &str,
        t: Option<Transport>,
    ) -> Result<()> {
        if !self.trust.lock().unwrap().is_trusted(id) {
            bail!("device not paired");
        }
        let now = konnekta_proto::packet::now_millis();
        self.trust.lock().unwrap().update(id, |d| {
            d.preferred_transport = t;
            d.transport_changed_at = now;
        });
        self.sent_transports.lock().unwrap().remove(id);
        self.send_transports(id);
        self.emit_updated(id);
        self.switch_soon();
        Ok(())
    }

    /// Dial the better ways now instead of at the next tick.
    fn switch_soon(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            this.dialing.lock().unwrap().clear();
            this.clone().dial_paired().await;
            if cfg!(not(any(target_os = "android", target_os = "ios"))) {
                this.usb_tick().await;
            }
            if cfg!(target_os = "linux") {
                this.bt_tick().await;
            }
        });
    }

    /// Send the ways we reach each device again when they changed.
    fn refresh_transports(&self) {
        let ids: Vec<String> = self.links.lock().unwrap().keys().cloned().collect();
        for id in ids {
            let now: Vec<Transport> = {
                let links = self.links.lock().unwrap();
                let Some(l) = links.get(&id) else { continue };
                self.local_transports(&id, Some(l)).into_iter().collect()
            };
            let changed = self.sent_transports.lock().unwrap().get(&id) != Some(&now);
            if changed {
                self.send_transports(&id);
                self.emit_updated(&id);
            }
        }
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

    /// Change the pair state only if the link is still `link_id`.
    fn set_pair_state(&self, id: &str, link_id: u64, state: PairState) -> bool {
        match self.links.lock().unwrap().get_mut(id) {
            Some(l) if l.handle.link_id == link_id => {
                l.pair = state;
                true
            }
            _ => false,
        }
    }

    // --- dispatch --------------------------------------------------------------

    async fn dispatch(self: &Arc<Self>, device_id: &str, link_id: u64, packet: Packet) {
        let Some((handle, state)) = self.link_handle(device_id) else {
            return;
        };
        if handle.link_id != link_id {
            return;
        }
        tracing::trace!(device = %device_id, packet = %packet.packet_type, "received");
        match packet.packet_type.as_str() {
            types::PAIR => {
                self.clone().handle_pair(handle, state, packet).await;
                return;
            }
            types::IDENTITY => return,
            _ => {}
        }
        if !state.is_paired() || !self.trust.lock().unwrap().is_trusted(device_id) {
            tracing::debug!(device = %device_id, packet = %packet.packet_type, "dropping packet from unpaired device");
            if !state.is_open() {
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
            }
            return;
        }
        if packet.packet_type == types::PERMISSIONS {
            self.on_permissions(device_id, &packet);
            return;
        }
        if packet.packet_type == types::TRANSPORTS {
            self.on_transports(device_id, &packet);
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
        for p in &self.plugins {
            let info = p.info();
            if !info.incoming.contains(&packet.packet_type.as_str()) {
                continue;
            }
            if disabled_global.contains(info.id) || disabled_dev.contains(info.id) {
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
    }

    // --- permission sync ---------------------------------------------------------
    //
    // Both devices of a pair keep one permission set: a change on either side
    // is sent to the other, and on connect the most recent change wins. Only
    // paired, trusted devices on the native protocol take part.

    /// Send our permission set for `id` (no-op when sync is off or the
    /// device is not connected).
    fn send_permissions(&self, id: &str) {
        if !self.config.read().unwrap().sync_permissions {
            return;
        }
        let Some(d) = self.trust.lock().unwrap().get(id).cloned() else {
            return;
        };
        let links = self.links.lock().unwrap();
        let Some(l) = links.get(id) else {
            return;
        };
        if !l.pair.is_paired() || l.handle.protocol != Protocol::Konnekta {
            return;
        }
        let p = Packet::new(
            types::PERMISSIONS,
            serde_json::json!({
                "permissions": d.permissions,
                "changedAt": d.permissions_changed_at,
            }),
        );
        if l.handle.tx.try_send(p).is_err() {
            tracing::warn!(device = %id, "could not send the permission set");
        }
    }

    fn on_permissions(&self, id: &str, packet: &Packet) {
        if !self.config.read().unwrap().sync_permissions {
            return;
        }
        let theirs_at = packet
            .body
            .get("changedAt")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        // Names this version does not know are skipped.
        let theirs: BTreeSet<Permission> = packet
            .body
            .get("permissions")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| serde_json::from_value(x.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        let Some(mine) = self.trust.lock().unwrap().get(id).cloned() else {
            return;
        };
        if mine.permissions == theirs {
            if theirs_at > mine.permissions_changed_at {
                self.trust
                    .lock()
                    .unwrap()
                    .update(id, |d| d.permissions_changed_at = theirs_at);
            }
            return;
        }
        let now = konnekta_proto::packet::now_millis();
        let (new_set, new_at) = match (mine.permissions_changed_at, theirs_at) {
            // Never changed on either side since pairing (or since an update
            // that predates sync): what either device allowed is allowed.
            (0, 0) => (&mine.permissions | &theirs, now),
            (a, b) if b > a => (theirs, b),
            // Ours is newer (or the same moment): the device takes ours.
            _ => {
                self.send_permissions(id);
                return;
            }
        };
        let granted: Vec<&'static str> = new_set
            .difference(&mine.permissions)
            .filter(|p| p.is_sensitive())
            .map(|p| p.label())
            .collect();
        self.trust.lock().unwrap().update(id, |d| {
            d.permissions = new_set.clone();
            d.permissions_changed_at = new_at;
        });
        if let Some(e) = self.links.lock().unwrap().get_mut(id) {
            e.denied.clear();
        }
        tracing::info!(device = %id, "permissions synced from the device");
        self.emit_updated(id);
        if !granted.is_empty() {
            let _ = self.events.send(Event::SecurityWarning {
                device_id: Some(id.to_owned()),
                message: format!(
                    "{} granted from the device: {}.",
                    mine.name,
                    granted.join(", ")
                ),
            });
        }
        // A merged set is new to the device too.
        if theirs_at == 0 && mine.permissions_changed_at == 0 {
            self.send_permissions(id);
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

    // --- pairing ---------------------------------------------------------------

    fn key(&self, handle: &DeviceHandle, ts: u64) -> String {
        verification_key(
            &self.identity.cert_der,
            &handle.peer_cert,
            ts,
            handle.protocol.key_len(),
        )
        .unwrap_or_default()
    }

    async fn send_pair(&self, handle: &DeviceHandle, pair: bool, ts: Option<u64>) -> bool {
        handle
            .tx
            .send(Packet::new(
                types::PAIR,
                PairBody {
                    pair,
                    timestamp: ts,
                },
            ))
            .await
            .is_ok()
    }

    fn open_incoming_requests(&self) -> Vec<(String, IpAddr)> {
        self.links
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, l)| matches!(l.pair, PairState::RequestedByPeer { .. }))
            .map(|(id, l)| (id.clone(), l.handle.addr.ip()))
            .collect()
    }

    fn cool_down(&self, id: &str) {
        let cd = Duration::from_secs(self.config.read().unwrap().limits.pair_cooldown_secs);
        self.guard.lock().unwrap().cool_down(id, cd);
    }

    async fn handle_pair(self: Arc<Self>, handle: DeviceHandle, state: PairState, packet: Packet) {
        let Ok(body) = packet.body_as::<PairBody>() else {
            return;
        };
        let id = handle.id().to_owned();
        let lid = handle.link_id;
        if let Some(l) = self.links.lock().unwrap().get_mut(&id) {
            l.pair_requested = true;
        }
        if body.pair {
            match state {
                PairState::RequestedByUs { timestamp, .. } => {
                    // The device accepted: the local user must now confirm.
                    self.set_pair_state(
                        &id,
                        lid,
                        PairState::Confirm {
                            timestamp,
                            since: Instant::now(),
                        },
                    );
                    let _ = self.events.send(Event::PairConfirm {
                        device_id: id.clone(),
                        device_name: handle.name().to_owned(),
                        verification_key: self.key(&handle, timestamp),
                    });
                    self.emit_updated(&id);
                }
                PairState::Confirm { .. } => {}
                PairState::Paired => {
                    // Only a new request (with timestamp) gets an answer: the
                    // peer lost its pin. Answers are never answered (no loop).
                    if body.timestamp.is_some() {
                        self.send_pair(&handle, true, None).await;
                    }
                }
                PairState::NotPaired | PairState::RequestedByPeer { .. } => {
                    let Some(ts) = body.timestamp else {
                        self.send_pair(&handle, false, None).await;
                        return;
                    };
                    if state.timestamp() == Some(ts) {
                        return; // duplicate
                    }
                    if !pairing::timestamp_is_fresh(ts, pairing::now_secs()) {
                        self.send_pair(&handle, false, None).await;
                        let _ = self.events.send(Event::PairRejected {
                            device_id: id,
                            reason: "clocks differ too much between devices".into(),
                        });
                        return;
                    }
                    let limits = self.config.read().unwrap().limits.clone();
                    let open = self.open_incoming_requests();
                    let admission = self.guard.lock().unwrap().admit(
                        &id,
                        handle.addr.ip(),
                        &open,
                        limits.open_pair_requests,
                        limits.open_pair_requests_per_addr,
                    );
                    let reason = match admission {
                        Admission::Ok => None,
                        Admission::TooFrequent => return,
                        Admission::CoolingDown => {
                            Some("the device asked again too soon after a refused request")
                        }
                        Admission::TooMany => Some("too many pairing requests are open"),
                    };
                    if let Some(reason) = reason {
                        self.send_pair(&handle, false, None).await;
                        let _ = self.events.send(Event::PairRefused {
                            device_id: id,
                            device_name: handle.name().to_owned(),
                            address: handle.addr.ip().to_string(),
                            reason: reason.into(),
                        });
                        return;
                    }
                    self.set_pair_state(
                        &id,
                        lid,
                        PairState::RequestedByPeer {
                            timestamp: ts,
                            since: Instant::now(),
                        },
                    );
                    let _ = self.events.send(Event::PairRequested {
                        device_id: id.clone(),
                        device_name: handle.name().to_owned(),
                        verification_key: self.key(&handle, ts),
                    });
                    self.emit_updated(&id);
                }
            }
        } else {
            match state {
                PairState::RequestedByUs { .. } | PairState::Confirm { .. } => {
                    self.set_pair_state(&id, lid, PairState::NotPaired);
                    let _ = self.events.send(Event::PairRejected {
                        device_id: id.clone(),
                        reason: "rejected by the other device".into(),
                    });
                    self.emit_updated(&id);
                }
                PairState::RequestedByPeer { .. } => {
                    self.set_pair_state(&id, lid, PairState::NotPaired);
                    self.cool_down(&id);
                    let _ = self.events.send(Event::PairRejected {
                        device_id: id.clone(),
                        reason: "withdrawn by the other device".into(),
                    });
                    self.emit_updated(&id);
                }
                PairState::Paired | PairState::NotPaired => {
                    if self.trust.lock().unwrap().is_trusted(&id) {
                        let _ = self.unpair(&id, false).await;
                    }
                }
            }
        }
    }

    async fn request_pair(self: Arc<Self>, id: &str) -> Result<String> {
        let (handle, state) = self.link_handle(id).context("device not connected")?;
        match state {
            PairState::Paired => bail!("already paired"),
            PairState::RequestedByPeer { .. } => {
                bail!("the device is asking to pair: accept or reject its request")
            }
            PairState::Confirm { .. } => bail!("confirm or reject the open pairing first"),
            _ => {}
        }
        let ts = pairing::now_secs();
        self.set_pair_state(
            id,
            handle.link_id,
            PairState::RequestedByUs {
                timestamp: ts,
                since: Instant::now(),
            },
        );
        if let Some(l) = self.links.lock().unwrap().get_mut(id) {
            l.pair_requested = true;
        }
        self.send_pair(&handle, true, Some(ts)).await;
        let key = self.key(&handle, ts);
        let _ = self.events.send(Event::PairOutgoing {
            device_id: id.to_owned(),
            verification_key: key.clone(),
        });
        self.emit_updated(id);
        Ok(key)
    }

    /// A device paired again under a new identity (app reinstalled, data
    /// cleared) shows up with the same name and type as the old entry. The
    /// new pairing — confirmed with the key like any other — replaces the old
    /// entry when that one is offline, and keeps its permissions, plugin
    /// choices and extra addresses, so the list shows one device, not two.
    fn replace_predecessor(&self, handle: &DeviceHandle) {
        let norm = |s: &str| s.trim().to_lowercase();
        let new_id = handle.id();
        let old = {
            let trust = self.trust.lock().unwrap();
            let links = self.links.lock().unwrap();
            trust
                .all()
                .filter(|d| {
                    d.device_id != new_id
                        && norm(&d.name) == norm(handle.name())
                        && d.device_type == handle.identity.device_type
                        && d.protocol == handle.protocol
                        && !links.get(&d.device_id).is_some_and(|l| l.pair.is_paired())
                })
                .max_by_key(|d| d.paired_at)
                .cloned()
        };
        let Some(old) = old else { return };
        let now = konnekta_proto::packet::now_millis();
        {
            let mut trust = self.trust.lock().unwrap();
            trust.update(new_id, |d| {
                d.permissions = old.permissions.clone();
                // Newer than the fresh device's defaults: the sync sends it ours.
                d.permissions_changed_at = now;
                d.disabled_plugins = old.disabled_plugins.clone();
                d.extra_addresses = old.extra_addresses.clone();
            });
            trust.remove(&old.device_id);
        }
        if let Some(entry) = self.links.lock().unwrap().remove(&old.device_id) {
            for t in entry.tasks {
                t.abort();
            }
        }
        tracing::info!(old = %old.device_id, new = %new_id, name = %old.name, "paired device replaced its old entry");
        let _ = self.events.send(Event::Unpaired {
            device_id: old.device_id.clone(),
        });
        let _ = self.events.send(Event::SecurityWarning {
            device_id: Some(new_id.to_owned()),
            message: format!(
                "{} was paired again (new app identity): it replaced the old entry and kept its permissions.",
                old.name
            ),
        });
    }

    fn pin(&self, handle: &DeviceHandle) {
        self.trust.lock().unwrap().insert(TrustedDevice {
            device_id: handle.id().to_owned(),
            name: handle.name().to_owned(),
            device_type: handle.identity.device_type,
            certificate: TrustedDevice::encode_cert(&handle.peer_cert),
            permissions: Permission::defaults(),
            permissions_changed_at: 0,
            disabled_plugins: BTreeSet::new(),
            paired_at: konnekta_proto::packet::now_millis(),
            last_address: (!handle.usb && !handle.bluetooth).then(|| handle.addr.ip()),
            last_port: if handle.protocol == Protocol::Konnekta {
                handle.identity.tcp_port
            } else {
                None
            },
            protocol: handle.protocol,
            extra_addresses: Vec::new(),
            preferred_transport: None,
            transport_changed_at: 0,
        });
    }

    async fn accept_pair(self: Arc<Self>, id: &str, key: Option<&str>) -> Result<()> {
        let (handle, state) = self.link_handle(id).context("device not connected")?;
        let ts = match state {
            PairState::RequestedByPeer { timestamp, .. } | PairState::Confirm { timestamp, .. } => {
                timestamp
            }
            _ => bail!("no open pairing with this device"),
        };
        if state.expired() {
            bail!("the pairing request expired");
        }
        if let Some(k) = key
            && !pairing::key_matches(k, &self.key(&handle, ts))
        {
            bail!("the key does not match the open pairing");
        }
        // Pin before answering; undo if the answer cannot go out.
        self.pin(&handle);
        if matches!(state, PairState::RequestedByPeer { .. })
            && !self.send_pair(&handle, true, None).await
        {
            self.trust.lock().unwrap().remove(id);
            bail!("connection lost while answering");
        }
        if !self.set_pair_state(id, handle.link_id, PairState::Paired) {
            self.trust.lock().unwrap().remove(id);
            bail!("connection changed during pairing");
        }
        self.guard.lock().unwrap().clear(id);
        tracing::info!(device = %handle.name(), "paired");
        self.replace_predecessor(&handle);
        let _ = self.events.send(Event::Paired {
            device_id: id.to_owned(),
        });
        self.emit_updated(id);
        self.notify_plugins_connected(&handle).await;
        Ok(())
    }

    async fn reject_pair(self: Arc<Self>, id: &str, key: Option<&str>) -> Result<()> {
        let (handle, state) = self.link_handle(id).context("device not connected")?;
        if !state.is_open() {
            bail!("no open pairing with this device");
        }
        if let (Some(k), Some(ts)) = (key, state.timestamp())
            && !pairing::key_matches(k, &self.key(&handle, ts))
        {
            bail!("the key does not match the open pairing");
        }
        self.send_pair(&handle, false, None).await;
        self.set_pair_state(id, handle.link_id, PairState::NotPaired);
        if matches!(state, PairState::RequestedByPeer { .. }) {
            self.cool_down(id);
        }
        self.emit_updated(id);
        Ok(())
    }

    async fn unpair(self: &Arc<Self>, id: &str, notify_peer: bool) -> Result<()> {
        let removed = self.trust.lock().unwrap().remove(id).is_some();
        if let Some((handle, _)) = self.link_handle(id) {
            if notify_peer {
                self.send_pair(&handle, false, None).await;
            }
            self.set_pair_state(id, handle.link_id, PairState::NotPaired);
            // Close the link once `pair:false` is flushed.
            let this = self.clone();
            let id2 = id.to_owned();
            let lid = handle.link_id;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                let still = this
                    .links
                    .lock()
                    .unwrap()
                    .get(&id2)
                    .is_some_and(|l| l.handle.link_id == lid);
                if still {
                    this.close_link(&id2).await;
                }
            });
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

    /// Expire pairings and close idle incoming links of unpaired devices.
    async fn janitor(self: &Arc<Self>) {
        let idle = Duration::from_secs(self.config.read().unwrap().limits.unpaired_idle_secs);
        let mut expired = Vec::new();
        let mut idle_links = Vec::new();
        {
            let links = self.links.lock().unwrap();
            for (id, l) in links.iter() {
                if l.pair.is_open() && l.pair.expired() {
                    expired.push((l.handle.clone(), l.pair));
                } else if !l.pair.is_paired()
                    && !l.pair.is_open()
                    && !l.handle.outgoing
                    && !l.pair_requested
                    && l.created.elapsed() > idle
                {
                    idle_links.push(id.clone());
                }
            }
        }
        for (handle, state) in expired {
            let id = handle.id().to_owned();
            if self.set_pair_state(&id, handle.link_id, PairState::NotPaired) {
                self.send_pair(&handle, false, None).await;
                if matches!(state, PairState::RequestedByPeer { .. }) {
                    self.cool_down(&id);
                }
                let _ = self.events.send(Event::PairRejected {
                    device_id: id.clone(),
                    reason: "pairing timed out".into(),
                });
                self.emit_updated(&id);
            }
        }
        for id in idle_links {
            tracing::debug!(device = %id, "closing idle unpaired link");
            self.close_link(&id).await;
        }
        self.refresh_transports();
    }
}

const PIN_MISMATCH: &str = "A device claiming to be this paired device presented a different certificate. \
     The connection was refused. If the device was reinstalled, unpair and pair it again.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_parsing() {
        assert_eq!(
            parse_host_port("100.64.1.2", 17160),
            ("100.64.1.2".into(), 17160)
        );
        assert_eq!(
            parse_host_port("100.64.1.2:17161", 17160),
            ("100.64.1.2".into(), 17161)
        );
        assert_eq!(
            parse_host_port("pixel.tail1234.ts.net", 17160),
            ("pixel.tail1234.ts.net".into(), 17160)
        );
        assert_eq!(parse_host_port("pixel:9000", 17160), ("pixel".into(), 9000));
        assert_eq!(
            parse_host_port("[fd7a::1]:17162", 17160),
            ("fd7a::1".into(), 17162)
        );
        assert_eq!(parse_host_port("fd7a::1", 17160), ("fd7a::1".into(), 17160));
    }
}
