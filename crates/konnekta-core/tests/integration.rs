//! End-to-end tests: two real Konnekta instances over loopback TCP + TLS.

use anyhow::Result;
use async_trait::async_trait;
use konnekta_core::proto::{Packet, types};
use konnekta_core::{
    Config, DeviceHandle, Event, Konnekta, Permission, Plugin, PluginInfo, PluginOrigin,
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
            origin: PluginOrigin::KdeConnect,
            permission: Permission::Ping,
            incoming: &[types::PING],
            outgoing: &[types::PING],
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
            id: "runcommand",
            name: "Run command",
            description: "test",
            origin: PluginOrigin::KdeConnect,
            permission: Permission::Commands,
            incoming: &[types::RUNCOMMAND_REQUEST],
            outgoing: &[types::RUNCOMMAND_REQUEST],
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
            .send(Packet::new(types::RUNCOMMAND_REQUEST, json!({"key": "x"})))
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
        announce: false,
        listen_udp: true,
        ..Config::default()
    }
}

struct Node {
    k: Konnekta,
    rec: Arc<Recorder>,
    danger: Arc<Danger>,
    events: broadcast::Receiver<Event>,
}

async fn node(name: &str) -> Node {
    let rec = Arc::new(Recorder::default());
    let danger = Arc::new(Danger::default());
    let k = Konnekta::builder(config(name))
        .plugin(rec.clone())
        .plugin(danger.clone())
        .build()
        .unwrap();
    let events = k.subscribe();
    k.start().await.unwrap();
    Node {
        k,
        rec,
        danger,
        events,
    }
}

async fn wait_for<F: FnMut(&Event) -> bool>(
    rx: &mut broadcast::Receiver<Event>,
    mut f: F,
) -> Event {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let e = rx.recv().await.unwrap();
            if f(&e) {
                return e;
            }
        }
    })
    .await
    .expect("event not received in time")
}

async fn eventually<F: FnMut() -> bool>(mut f: F) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition not met");
}

async fn connect(a: &mut Node, b: &mut Node) {
    a.k.connect_to(SocketAddr::new(LO, b.k.udp_port()))
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

async fn pair(a: &mut Node, b: &mut Node) {
    let b_id = b.k.device_id().to_string();
    let a_id = a.k.device_id().to_string();
    let key_a = a.k.request_pair(&b_id).await.unwrap();
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
    b.k.accept_pair(&a_id).await.unwrap();
    wait_for(&mut a.events, |e| matches!(e, Event::Paired { .. })).await;
}

#[tokio::test]
async fn discover_pair_and_exchange() {
    let mut a = node("Alpha").await;
    let mut b = node("Beta").await;
    connect(&mut a, &mut b).await;

    let b_id = b.k.device_id().to_string();
    let a_id = a.k.device_id().to_string();
    let infos = a.k.devices();
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].name, "Beta");
    assert!(!infos[0].paired);
    // Unpaired: no handle available to plugins.
    assert!(a.k.device_handle(&b_id).is_none());

    pair(&mut a, &mut b).await;
    assert!(a.k.device(&b_id).unwrap().paired);
    assert!(b.k.device(&a_id).unwrap().paired);

    // Ping is a default permission.
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
async fn sensitive_permissions_are_enforced() {
    let mut a = node("Alpha").await;
    let mut b = node("Beta").await;
    connect(&mut a, &mut b).await;
    pair(&mut a, &mut b).await;
    let b_id = b.k.device_id().to_string();
    let a_id = a.k.device_id().to_string();

    // A refuses to *send* commands to B: not granted locally.
    assert!(
        a.k.plugin_action(Some(&b_id), "runcommand", "run", Value::Null)
            .await
            .is_err()
    );

    // A grants itself the right, but B still has not granted Commands to A.
    a.k.set_permission(&b_id, Permission::Commands, true)
        .unwrap();
    a.k.plugin_action(Some(&b_id), "runcommand", "run", Value::Null)
        .await
        .unwrap();
    wait_for(&mut b.events, |e| {
        matches!(e, Event::SecurityWarning { .. })
    })
    .await;
    assert_eq!(*b.danger.ran.lock().unwrap(), 0);

    // Once B grants it, the command is delivered.
    b.k.set_permission(&a_id, Permission::Commands, true)
        .unwrap();
    a.k.plugin_action(Some(&b_id), "runcommand", "run", Value::Null)
        .await
        .unwrap();
    eventually(|| *b.danger.ran.lock().unwrap() == 1).await;

    // Revoking stops it again.
    b.k.set_permission(&a_id, Permission::Commands, false)
        .unwrap();
    a.k.plugin_action(Some(&b_id), "runcommand", "run", Value::Null)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(*b.danger.ran.lock().unwrap(), 1);
}

#[tokio::test]
async fn rejected_pairing_and_unpair() {
    let mut a = node("Alpha").await;
    let mut b = node("Beta").await;
    connect(&mut a, &mut b).await;
    let b_id = b.k.device_id().to_string();
    let a_id = a.k.device_id().to_string();

    a.k.request_pair(&b_id).await.unwrap();
    wait_for(&mut b.events, |e| matches!(e, Event::PairRequested { .. })).await;
    b.k.reject_pair(&a_id).await.unwrap();
    wait_for(&mut a.events, |e| matches!(e, Event::PairRejected { .. })).await;
    assert!(!a.k.device(&b_id).unwrap().paired);

    pair(&mut a, &mut b).await;
    b.k.unpair(&a_id).await.unwrap();
    wait_for(&mut a.events, |e| matches!(e, Event::Unpaired { .. })).await;
    assert!(a.k.device_handle(&b_id).is_none());
    assert!(!a.k.device(&b_id).unwrap().paired);
}

#[tokio::test]
async fn paired_device_reconnects_with_pinned_certificate() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let mk = |name: &str, dir: &std::path::Path| {
        Konnekta::builder(config(name))
            .data_dir(dir)
            .plugin(Arc::new(Recorder::default()))
            .build()
            .unwrap()
    };
    let (ka, kb) = (mk("Alpha", dir_a.path()), mk("Beta", dir_b.path()));
    let mut a = Node {
        events: ka.subscribe(),
        k: ka,
        rec: Default::default(),
        danger: Default::default(),
    };
    let mut b = Node {
        events: kb.subscribe(),
        k: kb,
        rec: Default::default(),
        danger: Default::default(),
    };
    a.k.start().await.unwrap();
    b.k.start().await.unwrap();
    connect(&mut a, &mut b).await;
    pair(&mut a, &mut b).await;
    let b_id = b.k.device_id().to_string();
    a.k.shutdown();
    b.k.shutdown();
    drop(a);

    // Restart A from disk: same identity, B still trusted.
    let ka = mk("Alpha", dir_a.path());
    let mut a2 = Node {
        events: ka.subscribe(),
        k: ka,
        rec: Default::default(),
        danger: Default::default(),
    };
    a2.k.start().await.unwrap();
    let kb = mk("Beta", dir_b.path());
    let mut b2 = Node {
        events: kb.subscribe(),
        k: kb,
        rec: Default::default(),
        danger: Default::default(),
    };
    b2.k.start().await.unwrap();
    assert_eq!(b2.k.device_id(), b_id);
    connect(&mut a2, &mut b2).await;
    eventually(|| a2.k.device_handle(&b_id).is_some()).await;

    // An impostor reusing B's device id but with a different key is refused.
    let evil_id = konnekta_crypto::LocalIdentity::generate(&b_id).unwrap();
    let evil = Konnekta::builder(config("Beta"))
        .identity(evil_id)
        .plugin(Arc::new(Recorder::default()))
        .build()
        .unwrap();
    evil.start().await.unwrap();
    b2.k.shutdown();
    wait_for(&mut a2.events, |e| {
        matches!(e, Event::DeviceDisconnected { .. })
    })
    .await;
    evil.connect_to(SocketAddr::new(LO, a2.k.udp_port()))
        .await
        .unwrap();
    wait_for(&mut a2.events, |e| {
        matches!(e, Event::SecurityWarning { .. })
    })
    .await;
    assert!(a2.k.device_handle(&b_id).is_none());
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
