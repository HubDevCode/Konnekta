//! End-to-end tests: real Konnekta instances over loopback TCP + TLS, for
//! the native protocol and the interop adapters (KDE Connect, Flux).

use anyhow::Result;
use async_trait::async_trait;
use konnekta_core::proto::{Packet, Protocol, types};
use konnekta_core::{
    Adapter, Config, DeviceHandle, Event, Konnekta, Permission, Plugin, PluginInfo, Transport,
};
use serde_json::{Value, json};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// Records packets it receives.
#[derive(Default)]
struct Recorder {
    got: Mutex<Vec<Packet>>,
}

#[async_trait]
impl Plugin for Recorder {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "ping",
            name: "Ping",
            description: "test",
            permission: Permission::Ping,
            incoming: &[types::PING, types::DECK],
            outgoing: &[types::PING, types::DECK],
        }
    }
    async fn handle_packet(&self, _dev: &DeviceHandle, p: Packet) -> Result<()> {
        self.got.lock().unwrap().push(p);
        Ok(())
    }
}

/// A sensitive plugin (commands) used to check permission enforcement.
#[derive(Default)]
struct Danger {
    ran: Mutex<u32>,
}

#[async_trait]
impl Plugin for Danger {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "commands",
            name: "Run command",
            description: "test",
            permission: Permission::Commands,
            incoming: &[types::COMMANDS_REQUEST],
            outgoing: &[types::COMMANDS_REQUEST],
        }
    }
    async fn handle_packet(&self, _dev: &DeviceHandle, _p: Packet) -> Result<()> {
        *self.ran.lock().unwrap() += 1;
        Ok(())
    }
    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        _a: &str,
        _args: Value,
    ) -> Result<Value> {
        dev.unwrap()
            .send(Packet::new(types::COMMANDS_REQUEST, json!({"key": "x"})))
            .await?;
        Ok(Value::Null)
    }
}

fn config(name: &str) -> Config {
    Config {
        device_name: name.into(),
        bind_addr: LO,
        tcp_port: 0,
        udp_port: 0,
        kde_connect: Adapter {
            enabled: false,
            udp_port: 0,
            tcp_port: 0,
        },
        flux: Adapter {
            enabled: false,
            udp_port: 0,
            tcp_port: 0,
        },
        announce: false,
        mdns: false,
        listen_udp: true,
        // Never reach real phones by cable or Bluetooth from a test.
        usb: false,
        bluetooth: false,
        virtual_camera: false,
        reconnect_interval_secs: 1,
        ..Config::default()
    }
}

struct Node {
    k: Konnekta,
    rec: Arc<Recorder>,
    danger: Arc<Danger>,
    events: broadcast::Receiver<Event>,
}

async fn node_with(cfg: Config, dir: Option<&std::path::Path>) -> Node {
    let rec = Arc::new(Recorder::default());
    let danger = Arc::new(Danger::default());
    let mut b = Konnekta::builder(cfg)
        .plugin(rec.clone())
        .plugin(danger.clone());
    if let Some(d) = dir {
        b = b.data_dir(d);
    }
    let k = b.build().unwrap();
    let events = k.subscribe();
    k.start().await.unwrap();
    Node {
        k,
        rec,
        danger,
        events,
    }
}

async fn node(name: &str) -> Node {
    node_with(config(name), None).await
}

async fn wait_for<F: FnMut(&Event) -> bool>(
    rx: &mut broadcast::Receiver<Event>,
    mut f: F,
) -> Event {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match rx.recv().await {
                Ok(e) if f(&e) => return e,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .expect("event not received in time")
}

async fn eventually<F: FnMut() -> bool>(mut f: F) {
    for _ in 0..300 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition not met");
}

/// B announces itself to A; A dials B (computer-initiated connection).
async fn connect(a: &mut Node, b: &mut Node) {
    b.k.announce_to(SocketAddr::new(LO, a.k.udp_port()), Protocol::Konnekta)
        .await
        .unwrap();
    wait_for(&mut a.events, |e| {
        matches!(e, Event::DeviceConnected { .. })
    })
    .await;
    wait_for(&mut b.events, |e| {
        matches!(e, Event::DeviceConnected { .. })
    })
    .await;
}

/// A asks, B accepts, A confirms the key.
async fn pair(a: &mut Node, b: &mut Node) {
    let b_id = b.k.device_id().to_string();
    let a_id = a.k.device_id().to_string();
    let key_a = a.k.request_pair(&b_id).await.unwrap();
    assert_eq!(key_a.len(), 16, "native keys have 64 bits");
    let Event::PairRequested {
        verification_key,
        device_id,
        ..
    } = wait_for(&mut b.events, |e| matches!(e, Event::PairRequested { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(device_id, a_id);
    assert_eq!(
        verification_key, key_a,
        "both sides must show the same code"
    );
    b.k.accept_pair(&a_id, Some(&verification_key))
        .await
        .unwrap();
    let Event::PairConfirm {
        verification_key: k2,
        ..
    } = wait_for(&mut a.events, |e| matches!(e, Event::PairConfirm { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(k2, key_a);
    // Not trusted until the computer confirms.
    assert!(a.k.device_handle(&b_id).is_none());
    a.k.accept_pair(&b_id, Some(&k2)).await.unwrap();
    wait_for(&mut a.events, |e| matches!(e, Event::Paired { .. })).await;
}

#[tokio::test]
async fn native_discover_pair_and_exchange() {
    let mut a = node("Alpha").await;
    let mut b = node("Beta").await;
    connect(&mut a, &mut b).await;

    let b_id = b.k.device_id().to_string();
    let a_id = a.k.device_id().to_string();
    let infos = a.k.devices();
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].name, "Beta");
    assert_eq!(infos[0].protocol, Protocol::Konnekta);
    assert_eq!(infos[0].app.as_deref(), Some("Konnekta"));
    assert!(!infos[0].paired);
    assert!(a.k.device_handle(&b_id).is_none());

    pair(&mut a, &mut b).await;
    assert!(a.k.device(&b_id).unwrap().paired);
    assert!(b.k.device(&a_id).unwrap().paired);

    let h = a.k.device_handle(&b_id).unwrap();
    h.send(Packet::new(types::PING, json!({"message": "ciao"})))
        .await
        .unwrap();
    eventually(|| b.rec.got.lock().unwrap().len() == 1).await;
    assert_eq!(
        b.rec.got.lock().unwrap()[0].get_str("message"),
        Some("ciao")
    );
}

#[tokio::test]
async fn confirm_rejects_wrong_key_and_reject_cools_down() {
    let mut a = node("Alpha").await;
    let mut b = node("Beta").await;
    connect(&mut a, &mut b).await;
    let (a_id, b_id) = (a.k.device_id().to_string(), b.k.device_id().to_string());

    a.k.request_pair(&b_id).await.unwrap();
    wait_for(&mut b.events, |e| matches!(e, Event::PairRequested { .. })).await;
    assert!(
        b.k.accept_pair(&a_id, Some("0000 0000 0000 0000"))
            .await
            .is_err(),
        "wrong key refused"
    );
    b.k.reject_pair(&a_id, None).await.unwrap();
    wait_for(&mut a.events, |e| matches!(e, Event::PairRejected { .. })).await;
    assert!(!a.k.device(&b_id).unwrap().paired);

    // A asks again at once: B is cooling down and refuses automatically.
    tokio::time::sleep(Duration::from_millis(100)).await;
    a.k.request_pair(&b_id).await.unwrap();
    wait_for(&mut b.events, |e| matches!(e, Event::PairRefused { .. })).await;
    wait_for(&mut a.events, |e| matches!(e, Event::PairRejected { .. })).await;
}

#[tokio::test]
async fn sensitive_permissions_are_enforced() {
    let mut a = node("Alpha").await;
    let mut b = node("Beta").await;
    connect(&mut a, &mut b).await;
    pair(&mut a, &mut b).await;
    let b_id = b.k.device_id().to_string();
    let a_id = a.k.device_id().to_string();
    // Each side decides alone when permission sync is off.
    b.k.update_config(|c| c.sync_permissions = false).unwrap();

    assert!(
        a.k.plugin_action(Some(&b_id), "commands", "run", Value::Null)
            .await
            .is_err()
    );
    a.k.set_permission(&b_id, Permission::Commands, true)
        .unwrap();
    a.k.plugin_action(Some(&b_id), "commands", "run", Value::Null)
        .await
        .unwrap();
    wait_for(&mut b.events, |e| {
        matches!(e, Event::SecurityWarning { .. })
    })
    .await;
    assert_eq!(*b.danger.ran.lock().unwrap(), 0);

    b.k.set_permission(&a_id, Permission::Commands, true)
        .unwrap();
    a.k.plugin_action(Some(&b_id), "commands", "run", Value::Null)
        .await
        .unwrap();
    eventually(|| *b.danger.ran.lock().unwrap() == 1).await;

    b.k.set_permission(&a_id, Permission::Commands, false)
        .unwrap();
    a.k.plugin_action(Some(&b_id), "commands", "run", Value::Null)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(*b.danger.ran.lock().unwrap(), 1);
}

#[tokio::test]
async fn transport_choice_is_synced_and_falls_back() {
    let mut a = node("A").await;
    let mut b = node("B").await;
    connect(&mut a, &mut b).await;
    pair(&mut a, &mut b).await;
    let (a_id, b_id) = (a.k.device_id().to_owned(), b.k.device_id().to_owned());
    eventually(|| b.k.device(&a_id).is_some_and(|d| d.paired && d.connected)).await;
    let d = a.k.device(&b_id).unwrap();
    assert_eq!(d.transport, Some(Transport::Wifi));
    assert!(d.transports.contains(&Transport::Wifi));

    // Chosen on one side: the other learns it.
    a.k.set_preferred_transport(&b_id, Some(Transport::Bluetooth))
        .unwrap();
    eventually(|| {
        b.k.device(&a_id)
            .is_some_and(|d| d.preferred_transport == Some(Transport::Bluetooth))
    })
    .await;
    // No Bluetooth here: the network link stays meanwhile.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let d = a.k.device(&b_id).unwrap();
    assert!(d.connected);
    assert_eq!(d.transport, Some(Transport::Wifi));

    // Back to automatic from the other side.
    b.k.set_preferred_transport(&a_id, None).unwrap();
    eventually(|| {
        a.k.device(&b_id)
            .is_some_and(|d| d.preferred_transport.is_none())
    })
    .await;
    assert!(a.k.set_preferred_transport("nobody", None).is_err());
}

#[tokio::test]
async fn unpair_closes_the_link() {
    let mut a = node("Alpha").await;
    let mut b = node("Beta").await;
    connect(&mut a, &mut b).await;
    pair(&mut a, &mut b).await;
    let (a_id, b_id) = (a.k.device_id().to_string(), b.k.device_id().to_string());
    b.k.unpair(&a_id).await.unwrap();
    wait_for(&mut a.events, |e| matches!(e, Event::Unpaired { .. })).await;
    wait_for(&mut a.events, |e| {
        matches!(e, Event::DeviceDisconnected { .. })
    })
    .await;
    assert!(a.k.device_handle(&b_id).is_none());
    assert!(a.k.device(&b_id).is_none());
}

#[tokio::test]
async fn paired_device_reconnects_with_pin_and_extra_address() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let mut a = node_with(config("Alpha"), Some(dir_a.path())).await;
    let mut b = node_with(config("Beta"), Some(dir_b.path())).await;
    connect(&mut a, &mut b).await;
    pair(&mut a, &mut b).await;
    let b_id = b.k.device_id().to_string();
    b.k.shutdown();
    wait_for(&mut a.events, |e| {
        matches!(e, Event::DeviceDisconnected { .. })
    })
    .await;

    // B comes back on another port; A only knows it through an extra
    // address (like a Tailscale name) and dials it by itself.
    let mut b2 = node_with(config("Beta"), Some(dir_b.path())).await;
    assert_eq!(b2.k.device_id(), b_id);
    a.k.set_extra_addresses(&b_id, vec![format!("127.0.0.1:{}", b2.k.tcp_port())])
        .unwrap();
    wait_for(&mut b2.events, |e| {
        matches!(e, Event::DeviceConnected { .. })
    })
    .await;
    eventually(|| a.k.device_handle(&b_id).is_some()).await;

    // An impostor with B's id but another key is refused by the pin.
    let evil_id = konnekta_crypto::LocalIdentity::generate(&b_id).unwrap();
    let evil = Konnekta::builder(config("Beta"))
        .identity(evil_id)
        .plugin(Arc::new(Recorder::default()))
        .build()
        .unwrap();
    evil.start().await.unwrap();
    b2.k.shutdown();
    wait_for(&mut a.events, |e| {
        matches!(e, Event::DeviceDisconnected { .. })
    })
    .await;
    a.k.set_extra_addresses(&b_id, vec![format!("127.0.0.1:{}", evil.tcp_port())])
        .unwrap();
    wait_for(&mut a.events, |e| {
        matches!(e, Event::SecurityWarning { .. })
    })
    .await;
    assert!(a.k.device_handle(&b_id).is_none());
}

#[tokio::test]
async fn simultaneous_connections_converge_to_one_link() {
    let mut a = node("Alpha").await;
    let mut b = node("Beta").await;
    // Both learn about each other at the same moment and dial.
    let (ra, rb) = tokio::join!(
        a.k.announce_to(SocketAddr::new(LO, b.k.udp_port()), Protocol::Konnekta),
        b.k.announce_to(SocketAddr::new(LO, a.k.udp_port()), Protocol::Konnekta),
    );
    ra.unwrap();
    rb.unwrap();
    wait_for(&mut a.events, |e| {
        matches!(e, Event::DeviceConnected { .. })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(a.k.devices().len(), 1);
    assert_eq!(b.k.devices().len(), 1);
    pair(&mut a, &mut b).await;
    let h = a.k.device_handle(b.k.device_id()).unwrap();
    h.send(Packet::new(types::PING, json!({}))).await.unwrap();
    eventually(|| b.rec.got.lock().unwrap().len() == 1).await;
}

#[tokio::test]
async fn manual_connect_by_address() {
    let mut a = node("Alpha").await;
    let mut b = node("Beta").await;
    a.k.connect_to(&format!("127.0.0.1:{}", b.k.tcp_port()))
        .await
        .unwrap();
    wait_for(&mut a.events, |e| {
        matches!(e, Event::DeviceConnected { .. })
    })
    .await;
    wait_for(&mut b.events, |e| {
        matches!(e, Event::DeviceConnected { .. })
    })
    .await;
    assert_eq!(a.k.devices()[0].id, b.k.device_id());
}

/// Config with the adapter of `p` turned on (ephemeral ports).
fn with_adapter(p: Protocol, name: &str) -> Config {
    let mut c = config(name);
    c.adapter_mut(p).unwrap().enabled = true;
    c
}

/// Two Konnekta nodes talking through an interop adapter.
async fn adapter_roundtrip(p: Protocol, key_len: usize) {
    let mut a = node_with(with_adapter(p, "Alpha"), None).await;
    let mut b = node_with(with_adapter(p, "Beta"), None).await;
    let (_, b_udp) = b.k.adapter_ports(p);
    a.k.announce_to(SocketAddr::new(LO, b_udp), p)
        .await
        .unwrap();
    wait_for(&mut a.events, |e| {
        matches!(e, Event::DeviceConnected { .. })
    })
    .await;
    wait_for(&mut b.events, |e| {
        matches!(e, Event::DeviceConnected { .. })
    })
    .await;
    let b_id = b.k.device_id().to_string();
    let a_id = a.k.device_id().to_string();
    assert_eq!(a.k.device(&b_id).unwrap().protocol, p);
    // Konnekta names after translation; Konnekta-only types filtered.
    let caps = a.k.device(&b_id).unwrap().incoming_capabilities;
    assert!(caps.contains(&types::PING.to_string()));
    assert!(!caps.contains(&types::DECK.to_string()));

    let key = a.k.request_pair(&b_id).await.unwrap();
    assert_eq!(key.len(), key_len);
    wait_for(&mut b.events, |e| matches!(e, Event::PairRequested { .. })).await;
    b.k.accept_pair(&a_id, Some(&key)).await.unwrap();
    wait_for(&mut a.events, |e| matches!(e, Event::PairConfirm { .. })).await;
    a.k.accept_pair(&b_id, Some(&key)).await.unwrap();

    let h = a.k.device_handle(&b_id).unwrap();
    h.send(Packet::new(types::PING, json!({"message": "interop"})))
        .await
        .unwrap();
    eventually(|| b.rec.got.lock().unwrap().len() == 1).await;
    // Konnekta-only packets cannot leave through an adapter.
    assert!(h.send(Packet::empty(types::DECK)).await.is_err());
}

#[tokio::test]
async fn kde_connect_adapter() {
    adapter_roundtrip(Protocol::KdeConnect, 8).await;
}

#[tokio::test]
async fn flux_adapter() {
    adapter_roundtrip(Protocol::Flux, 16).await;
}

#[tokio::test]
async fn adapters_are_off_by_default() {
    assert!(!Config::default().kde_connect.enabled);
    assert!(!Config::default().flux.enabled);
    let a = node_with(with_adapter(Protocol::KdeConnect, "Alpha"), None).await;
    let b = node("Beta").await;
    for p in Protocol::FOREIGN {
        assert_eq!(
            b.k.adapter_ports(p),
            (0, 0),
            "no {} sockets when disabled",
            p.label()
        );
    }
    // A foreign identity sent to the native port is not a valid announcement.
    a.k.announce_to(SocketAddr::new(LO, b.k.udp_port()), Protocol::KdeConnect)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(b.k.devices().is_empty());
    assert!(a.k.devices().is_empty());
}

/// A phone that speaks only the Flux wire format: it listens, the computer
/// dials it, sends `flux.identity` in clear, then the phone is the TLS
/// client. Checks names, capabilities, the 16-character key and packets.
#[tokio::test]
async fn raw_flux_phone_pairs_and_pings() {
    use konnekta_crypto::{LocalIdentity, PeerExpectation, client_config, generate_device_id};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let mut desk = node_with(with_adapter(Protocol::Flux, "Desk"), None).await;
    let desk_id = desk.k.device_id().to_string();
    let phone_id = generate_device_id();
    let phone = LocalIdentity::generate(&phone_id).unwrap();
    let listener = tokio::net::TcpListener::bind((LO, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let identity = |caps_in: &[&str], caps_out: &[&str]| {
        Packet::new(
            "flux.identity",
            json!({
                "deviceId": phone_id, "deviceName": "Pixel", "deviceType": "phone",
                "protocolVersion": 8, "tcpPort": port, "app": "android", "appVersion": "0.9.0",
                "incomingCapabilities": caps_in, "outgoingCapabilities": caps_out,
            }),
        )
    };
    // The phone scans: UDP identity to the computer's Flux port.
    let udp = tokio::net::UdpSocket::bind((LO, 0)).await.unwrap();
    let (_, desk_udp) = desk.k.adapter_ports(Protocol::Flux);
    udp.send_to(&identity(&[], &[]).to_line(), (LO, desk_udp))
        .await
        .unwrap();

    let (mut tcp, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
        .await
        .expect("computer never dialed")
        .unwrap();
    let mut plain = Vec::new();
    loop {
        let mut b = [0u8; 1];
        tokio::io::AsyncReadExt::read_exact(&mut tcp, &mut b)
            .await
            .unwrap();
        if b[0] == b'\n' {
            break;
        }
        plain.push(b[0]);
    }
    let first = Packet::from_line(&plain).unwrap();
    assert_eq!(first.packet_type, "flux.identity");
    assert_eq!(first.body["deviceId"], desk_id);

    let cfg = client_config(
        &phone,
        PeerExpectation::Unpaired {
            device_id: desk_id.clone(),
        },
    )
    .unwrap();
    let tls = tokio_rustls::TlsConnector::from(cfg)
        .connect(rustls_pki_types::ServerName::IpAddress(LO.into()), tcp)
        .await
        .unwrap();
    let mut s = BufReader::new(tls);
    let mut line = String::new();
    s.get_mut()
        .write_all(&identity(&["flux.ping"], &["flux.ping", "flux.herdr"]).to_line())
        .await
        .unwrap();
    s.read_line(&mut line).await.unwrap();
    let inner = Packet::from_line(line.as_bytes()).unwrap();
    assert_eq!(inner.packet_type, "flux.identity");
    let caps: Vec<String> =
        serde_json::from_value(inner.body["incomingCapabilities"].clone()).unwrap();
    assert!(caps.contains(&"flux.ping".to_string()));
    assert!(caps.iter().all(|c| c.starts_with("flux.")), "{caps:?}");
    wait_for(&mut desk.events, |e| {
        matches!(e, Event::DeviceConnected { .. })
    })
    .await;
    let d = desk.k.device(&phone_id).unwrap();
    assert_eq!(d.protocol, Protocol::Flux);
    assert_eq!(
        d.outgoing_capabilities,
        [types::PING.to_string()],
        "unknown Flux types dropped"
    );

    // Pairing started by the computer; the phone accepts.
    let key = desk.k.request_pair(&phone_id).await.unwrap();
    assert_eq!(key.len(), 16);
    let req = next_packet(&mut s).await;
    assert_eq!(req.packet_type, "flux.pair");
    assert_eq!(req.body["pair"], true);
    let ts = req.body["timestamp"].as_u64().unwrap();
    let desk_cert = desk.k.certificate_der();
    let expected = konnekta_crypto::verification_key(&phone.cert_der, &desk_cert, ts, 16).unwrap();
    assert_eq!(key, expected, "same key as the Flux app computes");
    s.get_mut()
        .write_all(&Packet::new("flux.pair", json!({ "pair": true })).to_line())
        .await
        .unwrap();
    wait_for(&mut desk.events, |e| matches!(e, Event::PairConfirm { .. })).await;
    desk.k.accept_pair(&phone_id, Some(&key)).await.unwrap();
    wait_for(&mut desk.events, |e| matches!(e, Event::Paired { .. })).await;

    // Packets both ways, translated at the boundary.
    desk.k
        .device_handle(&phone_id)
        .unwrap()
        .send(Packet::new(types::PING, json!({ "message": "ciao" })))
        .await
        .unwrap();
    let ping = next_packet(&mut s).await;
    assert_eq!(
        (ping.packet_type.as_str(), ping.body["message"].as_str()),
        ("flux.ping", Some("ciao"))
    );
    s.get_mut()
        .write_all(&Packet::new("flux.ping", json!({ "message": "dal telefono" })).to_line())
        .await
        .unwrap();
    eventually(|| b_got(&desk.rec, "dal telefono")).await;
}

async fn next_packet<R: tokio::io::AsyncBufRead + Unpin>(r: &mut R) -> Packet {
    use tokio::io::AsyncBufReadExt;
    let mut l = String::new();
    r.read_line(&mut l).await.unwrap();
    Packet::from_line(l.as_bytes()).unwrap()
}

fn b_got(rec: &Recorder, msg: &str) -> bool {
    rec.got
        .lock()
        .unwrap()
        .iter()
        .any(|p| p.packet_type == types::PING && p.body["message"] == msg)
}

#[tokio::test]
async fn secure_stream_channel() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut a = node("Alpha").await;
    let mut b = node("Beta").await;
    connect(&mut a, &mut b).await;
    pair(&mut a, &mut b).await;
    let ha = a.k.device_handle(b.k.device_id()).unwrap();
    let hb = b.k.device_handle(a.k.device_id()).unwrap();
    let (port, accept) = ha.listen_channel().await.unwrap();
    let server = tokio::spawn(async move {
        let mut ch = accept.await.unwrap();
        ch.write_all(b"frame").await.unwrap();
        ch.flush().await.unwrap();
        let mut ack = [0u8; 3];
        ch.read_exact(&mut ack).await.unwrap();
        ack
    });
    let mut ch = hb.connect_channel(port).await.unwrap();
    let mut buf = [0u8; 5];
    ch.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"frame");
    ch.write_all(b"ack").await.unwrap();
    ch.flush().await.unwrap();
    assert_eq!(&server.await.unwrap(), b"ack");
}

#[tokio::test]
async fn renamed_plugin_data_is_migrated() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("plugins/runcommand")).unwrap();
    std::fs::write(dir.path().join("plugins/runcommand/commands.json"), b"{}").unwrap();
    let mut cfg = config("Alpha");
    cfg.disabled_plugins.insert("mpris".into());
    let n = node_with(cfg, Some(dir.path())).await;
    assert!(dir.path().join("plugins/commands/commands.json").is_file());
    assert!(!dir.path().join("plugins/runcommand").exists());
    assert!(n.k.config().disabled_plugins.contains("media"));
}
