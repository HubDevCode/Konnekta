//! End-to-end plugin tests: two full Konnekta nodes (all plugins) with mock
//! platforms, talking over real TLS on loopback.

use konnekta_core::proto::{Packet, Protocol, types};
use konnekta_core::{Config, Event, Konnekta, Permission};
use konnekta_platform::mock::MockPlatform;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

const LO: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

struct Node {
    k: Konnekta,
    mock: Arc<MockPlatform>,
    ev: broadcast::Receiver<Event>,
    _dir: tempfile::TempDir,
}

async fn node(name: &str) -> Node {
    // Not ".tmpXXXX": folders inside dot folders are never browsable.
    let dir = tempfile::Builder::new()
        .prefix("konnekta-e2e")
        .tempdir()
        .unwrap();
    let mock = MockPlatform::new();
    let cfg = Config {
        device_name: name.into(),
        bind_addr: LO,
        tcp_port: 0,
        udp_port: 0,
        announce: false,
        mdns: false,
        download_dir: dir.path().join("downloads"),
        // Never reach real phones by cable or Bluetooth from a test.
        usb: false,
        bluetooth: false,
        virtual_camera: false,
        ..Config::default()
    };
    let k = Konnekta::builder(cfg)
        .data_dir(dir.path().join("data"))
        .plugins(konnekta_plugins::all(mock.platform()))
        .build()
        .unwrap();
    let ev = k.subscribe();
    k.start().await.unwrap();
    Node {
        k,
        mock,
        ev,
        _dir: dir,
    }
}

async fn wait<F: FnMut(&Event) -> bool>(rx: &mut broadcast::Receiver<Event>, mut f: F) -> Event {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match rx.recv().await {
                Ok(e) if f(&e) => return e,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .expect("timed out waiting for event")
}

async fn eventually<F: FnMut() -> bool>(mut f: F) {
    for _ in 0..300 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition never met");
}

async fn paired_pair() -> (Node, Node) {
    let mut a = node("Desktop").await;
    let mut b = node("Phone").await;
    // The phone announces itself; the computer dials it.
    b.k.announce_to(SocketAddr::new(LO, a.k.udp_port()), Protocol::Konnekta)
        .await
        .unwrap();
    wait(&mut a.ev, |e| matches!(e, Event::DeviceConnected { .. })).await;
    wait(&mut b.ev, |e| matches!(e, Event::DeviceConnected { .. })).await;
    let (aid, bid) = (a.k.device_id().to_string(), b.k.device_id().to_string());
    let key = a.k.request_pair(&bid).await.unwrap();
    wait(&mut b.ev, |e| matches!(e, Event::PairRequested { .. })).await;
    b.k.accept_pair(&aid, Some(&key)).await.unwrap();
    wait(&mut a.ev, |e| matches!(e, Event::PairConfirm { .. })).await;
    a.k.accept_pair(&bid, Some(&key)).await.unwrap();
    wait(&mut a.ev, |e| matches!(e, Event::Paired { .. })).await;
    (a, b)
}

#[tokio::test]
async fn clipboard_syncs_both_ways_without_loops() {
    let (a, b) = paired_pair().await;
    *a.mock.clipboard.lock().unwrap() = Some("from desktop".into());
    eventually(|| b.mock.clipboard.lock().unwrap().as_deref() == Some("from desktop")).await;
    *b.mock.clipboard.lock().unwrap() = Some("from phone".into());
    eventually(|| a.mock.clipboard.lock().unwrap().as_deref() == Some("from phone")).await;
    // Give watchers time to (wrongly) bounce content back.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let sets = |m: &MockPlatform| {
        m.calls()
            .iter()
            .filter(|c| c.starts_with("clipboard.set"))
            .count()
    };
    assert_eq!(
        sets(&a.mock),
        1,
        "desktop applied exactly one remote update"
    );
    assert_eq!(sets(&b.mock), 1, "phone applied exactly one remote update");
}

#[tokio::test]
async fn file_transfer_with_integrity_and_destination() {
    let (a, mut b) = paired_pair().await;
    let bid = b.k.device_id().to_string();
    let src = tempfile::tempdir().unwrap();
    let path = src.path().join("report.bin");
    let data: Vec<u8> = (0..1_500_000u32).map(|i| (i * 7 % 256) as u8).collect();
    std::fs::write(&path, &data).unwrap();
    let dest = tempfile::tempdir().unwrap();
    b.k.plugin_action(
        None,
        "share",
        "setConfig",
        json!({ "destination": dest.path() }),
    )
    .await
    .unwrap();
    a.k.plugin_action(Some(&bid), "share", "sendFiles", json!({ "paths": [path] }))
        .await
        .unwrap();
    wait(&mut b.ev, |e| matches!(e, Event::Plugin { plugin, data, .. } if plugin == "share" && data["state"] == "completed")).await;
    let got = std::fs::read(dest.path().join("report.bin")).unwrap();
    assert_eq!(Sha256::digest(&got), Sha256::digest(&data));
    // No leftover partial files.
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 1);
    let t =
        b.k.plugin_action(None, "share", "transfers", json!(null))
            .await
            .unwrap();
    assert_eq!(t[0]["direction"], "incoming");
}

#[tokio::test]
async fn commands_need_permission_and_run_by_key() {
    let (a, b) = paired_pair().await;
    let (aid, bid) = (a.k.device_id().to_string(), b.k.device_id().to_string());
    let r =
        a.k.plugin_action(
            None,
            "commands",
            "add",
            json!({ "name": "Lock", "actions": [{ "type": "shell", "command": "lock-now" }] }),
        )
        .await
        .unwrap();
    let key = r["id"].as_str().unwrap().to_owned();
    // With permission sync off the desktop decides alone.
    a.k.update_config(|c| c.sync_permissions = false).unwrap();
    // Phone grants itself the right to send, desktop has not granted it yet.
    b.k.set_permission(&aid, Permission::Commands, true)
        .unwrap();
    b.k.plugin_action(Some(&aid), "commands", "runRemote", json!({ "key": key }))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!a.mock.calls().iter().any(|c| c == "shell:lock-now"));
    a.k.set_permission(&bid, Permission::Commands, true)
        .unwrap();
    b.k.plugin_action(Some(&aid), "commands", "runRemote", json!({ "key": key }))
        .await
        .unwrap();
    eventually(|| a.mock.calls().iter().any(|c| c == "shell:lock-now")).await;
    // Unknown keys never execute anything.
    b.k.plugin_action(
        Some(&aid),
        "commands",
        "runRemote",
        json!({ "key": "rm -rf /" }),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        a.mock
            .calls()
            .iter()
            .filter(|c| c.starts_with("shell:"))
            .count(),
        1
    );
}

#[tokio::test]
async fn deck_press_from_phone_runs_macro() {
    let (a, b) = paired_pair().await;
    let (aid, bid) = (a.k.device_id().to_string(), b.k.device_id().to_string());
    a.k.set_permission(&bid, Permission::Deck, true).unwrap();
    b.k.set_permission(&aid, Permission::Deck, true).unwrap();
    let deck =
        a.k.plugin_action(None, "deck", "get", json!(null))
            .await
            .unwrap();
    let page = &deck["profiles"][0]["pages"][0];
    let next = page["buttons"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["label"] == "Successivo")
        .unwrap();
    // The phone side has no Deck UI in tests: send the raw request packet.
    let h = b.k.device_handle(&aid).unwrap();
    h.send(Packet::new(
        types::DECK_REQUEST,
        json!({ "request": "press", "profile": "work", "page": page["id"], "button": next["id"] }),
    ))
    .await
    .unwrap();
    eventually(|| a.mock.calls().iter().any(|c| c == "media:Next")).await;
}

#[tokio::test]
async fn permissions_stay_the_same_on_both_devices() {
    let (a, b) = paired_pair().await;
    let (aid, bid) = (a.k.device_id().to_string(), b.k.device_id().to_string());
    let has = |k: &Konnekta, id: &str, p: Permission| {
        k.device(id)
            .map(|d| d.permissions.contains(&p))
            .unwrap_or(false)
    };
    // Granted on the phone: the computer follows, and the command runs.
    b.k.set_permission(&aid, Permission::Commands, true)
        .unwrap();
    eventually(|| has(&a.k, &bid, Permission::Commands)).await;
    // Revoked on the computer: the phone follows.
    a.k.set_permission(&bid, Permission::Commands, false)
        .unwrap();
    eventually(|| !has(&b.k, &aid, Permission::Commands)).await;
    a.k.set_permission(&bid, Permission::RemoteInput, true)
        .unwrap();
    eventually(|| has(&b.k, &aid, Permission::RemoteInput)).await;

    // With sync off on the computer, its permissions are its own.
    a.k.update_config(|c| c.sync_permissions = false).unwrap();
    b.k.set_permission(&aid, Permission::Camera, true).unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!has(&a.k, &bid, Permission::Camera));
}

#[tokio::test]
async fn reinstalled_phone_replaces_its_old_entry() {
    let (mut a, b) = paired_pair().await;
    let (aid, old_id) = (a.k.device_id().to_string(), b.k.device_id().to_string());
    a.k.set_permission(&old_id, Permission::RemoteInput, true)
        .unwrap();
    // The app is reinstalled: the old identity goes offline for good.
    b.k.shutdown();
    drop(b);
    wait(
        &mut a.ev,
        |e| matches!(e, Event::DeviceDisconnected { device_id } if *device_id == old_id),
    )
    .await;

    // Same name, new identity, paired again with the key.
    let mut c = node("Phone").await;
    let new_id = c.k.device_id().to_string();
    c.k.announce_to(SocketAddr::new(LO, a.k.udp_port()), Protocol::Konnekta)
        .await
        .unwrap();
    wait(
        &mut a.ev,
        |e| matches!(e, Event::DeviceConnected { device } if device.id == new_id),
    )
    .await;
    let key = a.k.request_pair(&new_id).await.unwrap();
    wait(&mut c.ev, |e| matches!(e, Event::PairRequested { .. })).await;
    c.k.accept_pair(&aid, Some(&key)).await.unwrap();
    wait(&mut a.ev, |e| matches!(e, Event::PairConfirm { .. })).await;
    a.k.accept_pair(&new_id, Some(&key)).await.unwrap();
    wait(&mut a.ev, |e| matches!(e, Event::Paired { .. })).await;

    let paired: Vec<String> =
        a.k.devices()
            .into_iter()
            .filter(|d| d.paired)
            .map(|d| d.id)
            .collect();
    assert_eq!(paired, vec![new_id.clone()], "one entry for the phone");
    assert!(
        a.k.device(&new_id)
            .unwrap()
            .permissions
            .contains(&Permission::RemoteInput)
    );
    // The phone learns the kept permissions through the sync.
    eventually(|| {
        c.k.device(&aid)
            .is_some_and(|d| d.permissions.contains(&Permission::RemoteInput))
    })
    .await;
}

#[tokio::test]
async fn phone_drives_the_computer_mixer() {
    let (a, b) = paired_pair().await;
    let (aid, bid) = (a.k.device_id().to_string(), b.k.device_id().to_string());
    let h = b.k.device_handle(&aid).unwrap();
    let mute_player = || {
        h.send(Packet::new(
            types::MIXER_REQUEST,
            json!({ "kind": "app", "id": 3, "mute": true, "output": 1 }),
        ))
    };
    // Media control is granted by default; once revoked the computer ignores the phone.
    a.k.update_config(|c| c.sync_permissions = false).unwrap();
    a.k.set_permission(&bid, Permission::MediaControl, false)
        .unwrap();
    mute_player().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!a.mock.calls().iter().any(|c| c.starts_with("mixer.")));

    a.k.set_permission(&bid, Permission::MediaControl, true)
        .unwrap();
    mute_player().await.unwrap();
    eventually(|| {
        let c = a.mock.calls();
        c.iter().any(|c| c == "mixer.mute:App:3:true") && c.iter().any(|c| c == "mixer.move:3:1")
    })
    .await;
    // The phone gets the state back after the change.
    let mut got = None;
    for _ in 0..300 {
        got =
            b.k.plugin_action(Some(&aid), "mixer", "remoteState", json!(null))
                .await
                .ok()
                .filter(|v| !v.is_null());
        if got.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let state = got.expect("mixer state never arrived");
    assert_eq!(state["outputs"][0]["label"], "Speakers");
    assert_eq!(state["apps"][0]["output"], 1);

    // Volumes are capped; outputs cannot be "moved".
    h.send(Packet::new(
        types::MIXER_REQUEST,
        json!({ "kind": "output", "id": 1, "volume": 400, "makeDefault": true }),
    ))
    .await
    .unwrap();
    eventually(|| {
        let c = a.mock.calls();
        c.iter().any(|c| c == "mixer.volume:Output:1:150")
            && c.iter().any(|c| c == "mixer.default:Output:1")
    })
    .await;
}

#[tokio::test]
async fn screenshots_are_saved_deduplicated_and_copied() {
    let (mut a, b) = paired_pair().await;
    let aid = a.k.device_id().to_string();
    let folder = tempfile::tempdir().unwrap();
    a.k.plugin_action(None, "screenshot", "setConfig", json!({ "save": true, "folder": folder.path(), "copyToClipboard": true, "autoTransfer": true }))
        .await
        .unwrap();
    let png: Vec<u8> = [
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..],
        &[42u8; 5000][..],
    ]
    .concat();
    let hash = hex::encode(Sha256::digest(&png));
    let h = b.k.device_handle(&aid).unwrap();
    for _ in 0..2 {
        let p = Packet::new(
            types::SCREENSHOT,
            json!({ "filename": "Screenshot_1.png", "sha256": hash }),
        );
        let t = h
            .send_with_payload(
                p,
                konnekta_core::payload::PayloadSource::Bytes(png.clone()),
                None,
            )
            .await
            .unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), t).await;
    }
    wait(&mut a.ev, |e| matches!(e, Event::Plugin { plugin, kind, .. } if plugin == "screenshot" && kind == "received")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        std::fs::read_dir(folder.path()).unwrap().count(),
        1,
        "duplicate not saved twice"
    );
    assert_eq!(a.mock.png.lock().unwrap().as_deref(), Some(&png[..]));
}

#[tokio::test]
async fn notifications_are_shown_and_filtered() {
    let (a, b) = paired_pair().await;
    let aid = a.k.device_id().to_string();
    a.k.plugin_action(
        None,
        "notifications",
        "setFilter",
        json!({ "blockedApps": ["Spam"], "showPopups": true }),
    )
    .await
    .unwrap();
    let h = b.k.device_handle(&aid).unwrap();
    for (app, title) in [("Spam", "buy"), ("Signal", "hello")] {
        h.send(Packet::new(
            types::NOTIFICATION,
            json!({ "id": app, "appName": app, "title": title, "text": "x", "isClearable": true }),
        ))
        .await
        .unwrap();
    }
    eventually(|| a.mock.calls().iter().any(|c| c == "notify:hello:x")).await;
    assert!(!a.mock.calls().iter().any(|c| c.contains("buy")));
}

/// Desktop (full plugins) <-> phone (bridge plugins used by the mobile apps).
#[tokio::test]
async fn desktop_interoperates_with_mobile_bridge() {
    use konnekta_plugins::bridge::{PendingPayloads, mobile};
    let mut desk = node("Desktop").await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        device_name: "Phone".into(),
        device_type: konnekta_core::proto::DeviceType::Phone,
        bind_addr: LO,
        tcp_port: 0,
        udp_port: 0,
        announce: false,
        mdns: false,
        // Never reach real phones by cable or Bluetooth from a test.
        usb: false,
        bluetooth: false,
        virtual_camera: false,
        ..Config::default()
    };
    let phone = Konnekta::builder(cfg)
        .data_dir(dir.path())
        .plugins(mobile(Arc::new(PendingPayloads::default()), true))
        .build()
        .unwrap();
    let mut pev = phone.subscribe();
    phone.start().await.unwrap();
    phone
        .announce_to(SocketAddr::new(LO, desk.k.udp_port()), Protocol::Konnekta)
        .await
        .unwrap();
    wait(&mut pev, |e| matches!(e, Event::DeviceConnected { .. })).await;
    let did = desk.k.device_id().to_string();
    let pid = phone.device_id().to_string();
    // The phone starts the pairing: the computer accepts (pins at once).
    let key = phone.request_pair(&did).await.unwrap();
    wait(&mut desk.ev, |e| matches!(e, Event::PairRequested { .. })).await;
    desk.k.accept_pair(&pid, Some(&key)).await.unwrap();
    wait(&mut pev, |e| matches!(e, Event::PairConfirm { .. })).await;
    phone.accept_pair(&did, Some(&key)).await.unwrap();
    wait(&mut pev, |e| matches!(e, Event::Paired { .. })).await;

    // Phone -> desktop notification through the bridge.
    phone
        .device_handle(&did)
        .unwrap()
        .send(Packet::new(
            types::NOTIFICATION,
            json!({ "id": "n1", "appName": "Telegram", "title": "Hi", "text": "there" }),
        ))
        .await
        .unwrap();
    eventually(|| desk.mock.calls().iter().any(|c| c == "notify:Hi:there")).await;

    // Desktop -> phone ping arrives as a bridge event for the app.
    desk.k
        .plugin_action(Some(&pid), "ping", "send", json!({ "message": "yo" }))
        .await
        .unwrap();
    let e = wait(
        &mut pev,
        |e| matches!(e, Event::Plugin { plugin, kind, .. } if plugin == "ping" && kind == "packet"),
    )
    .await;
    let Event::Plugin { data, .. } = e else {
        unreachable!()
    };
    assert_eq!(data["body"]["message"], "yo");

    // The desktop can't remote-control the phone's input: the phone bridge
    // declares mousepad as outgoing only.
    assert!(
        desk.k
            .device(&pid)
            .unwrap()
            .incoming_capabilities
            .iter()
            .all(|c| c != types::INPUT)
    );
}

/// The whole remote desktop path on this machine: real screen capture,
/// encoder, FLV, frames over the paired TLS channel to a phone; what arrives
/// is written as Annex B for ffmpeg to check.
/// `KONNEKTA_DUMP=/tmp/x.h264 cargo test -p konnekta-plugins --test e2e -- --ignored live_remote_desktop --nocapture`
#[tokio::test]
#[ignore]
async fn live_remote_desktop() {
    let (desk, phone, mut pev, _pdir) = desktop_and_bridge_phone().await;
    let (did, pid) = (
        desk.k.device_id().to_string(),
        phone.device_id().to_string(),
    );
    desk.k
        .set_permission(&pid, Permission::Desktop, true)
        .unwrap();
    phone
        .set_permission(&did, Permission::Desktop, true)
        .unwrap();
    let h = phone.device_handle(&did).unwrap();
    h.send(Packet::new(
        types::DESKTOP,
        json!({ "action": "request", "session": "live", "params": { "height": 1080 } }),
    ))
    .await
    .unwrap();
    let offer = bridge_packet(&mut pev, "desktop").await;
    let port = offer["body"]["port"].as_u64().expect("offer with port") as u16;
    let mut ch = h.connect_channel(port).await.unwrap();
    let t0 = std::time::Instant::now();
    let (mut n, mut bytes, mut dump) = (0usize, 0usize, Vec::new());
    let mut last = t0;
    let mut worst = Duration::ZERO;
    while t0.elapsed() < Duration::from_secs(8) {
        let f = tokio::time::timeout(
            Duration::from_secs(5),
            konnekta_media::frame::read_frame(&mut ch),
        )
        .await
        .expect("no frame for 3 s")
        .unwrap()
        .expect("stream ended");
        if f.kind != konnekta_media::FrameKind::Video {
            continue;
        }
        n += 1;
        bytes += f.data.len();
        dump.extend_from_slice(&f.data);
        if n == 1 {
            println!("first picture after {:?}", t0.elapsed());
        } else {
            worst = worst.max(last.elapsed());
        }
        last = std::time::Instant::now();
    }
    println!(
        "{n} frames, {} kbit/s, longest gap {worst:?}",
        bytes * 8 / 5000
    );
    if let Ok(path) = std::env::var("KONNEKTA_DUMP") {
        std::fs::write(path, &dump).unwrap();
    }
    assert!(n > 10);
}

/// A desktop node and a phone running the mobile bridge, paired.
async fn desktop_and_bridge_phone() -> (
    Node,
    Konnekta,
    broadcast::Receiver<Event>,
    tempfile::TempDir,
) {
    use konnekta_plugins::bridge::{PendingPayloads, mobile};
    let mut desk = node("Desktop").await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        device_name: "Phone".into(),
        device_type: konnekta_core::proto::DeviceType::Phone,
        bind_addr: LO,
        tcp_port: 0,
        udp_port: 0,
        announce: false,
        mdns: false,
        // Never reach real phones by cable or Bluetooth from a test.
        usb: false,
        bluetooth: false,
        virtual_camera: false,
        ..Config::default()
    };
    let phone = Konnekta::builder(cfg)
        .data_dir(dir.path())
        .plugins(mobile(Arc::new(PendingPayloads::default()), true))
        .build()
        .unwrap();
    let mut pev = phone.subscribe();
    phone.start().await.unwrap();
    phone
        .announce_to(SocketAddr::new(LO, desk.k.udp_port()), Protocol::Konnekta)
        .await
        .unwrap();
    wait(&mut pev, |e| matches!(e, Event::DeviceConnected { .. })).await;
    let (did, pid) = (
        desk.k.device_id().to_string(),
        phone.device_id().to_string(),
    );
    let key = phone.request_pair(&did).await.unwrap();
    wait(&mut desk.ev, |e| matches!(e, Event::PairRequested { .. })).await;
    desk.k.accept_pair(&pid, Some(&key)).await.unwrap();
    wait(&mut pev, |e| matches!(e, Event::PairConfirm { .. })).await;
    phone.accept_pair(&did, Some(&key)).await.unwrap();
    wait(&mut pev, |e| matches!(e, Event::Paired { .. })).await;
    (desk, phone, pev, dir)
}

/// Next packet of `plugin_id` delivered to the phone app.
async fn bridge_packet(pev: &mut broadcast::Receiver<Event>, plugin_id: &str) -> serde_json::Value {
    let e = wait(pev, |e| matches!(e, Event::Plugin { plugin, kind, .. } if plugin == plugin_id && kind == "packet")).await;
    let Event::Plugin { data, .. } = e else {
        unreachable!()
    };
    data
}

#[tokio::test]
async fn phone_browses_computer_read_only() {
    let (mut desk, phone, mut pev, _pdir) = desktop_and_bridge_phone().await;
    let (did, pid) = (
        desk.k.device_id().to_string(),
        phone.device_id().to_string(),
    );
    // With permission sync off the computer decides alone.
    desk.k
        .update_config(|c| c.sync_permissions = false)
        .unwrap();
    phone
        .set_permission(&did, Permission::BrowseFiles, true)
        .unwrap();
    let dl = desk.k.config().download_dir.clone();
    std::fs::create_dir_all(dl.join("Viaggi")).unwrap();
    std::fs::write(dl.join("Viaggi/biglietto.pdf"), b"%PDF-1.7 ticket").unwrap();
    std::fs::write(dl.join(".segreto"), b"no").unwrap();
    let send = |body: serde_json::Value| {
        let h = phone.device_handle(&did).unwrap();
        async move {
            h.send(Packet::new(types::BROWSE_REQUEST, body))
                .await
                .unwrap()
        }
    };

    // High-risk permission: nothing is served until the computer grants it.
    send(json!({ "action": "roots" })).await;
    assert!(
        tokio::time::timeout(
            Duration::from_millis(500),
            bridge_packet(&mut pev, "browse")
        )
        .await
        .is_err(),
        "served without permission"
    );
    desk.k
        .set_permission(&pid, Permission::BrowseFiles, true)
        .unwrap();

    // Off by default.
    send(json!({ "action": "roots" })).await;
    let r = bridge_packet(&mut pev, "browse").await;
    assert_eq!(r["body"]["action"], "error");

    desk.k
        .plugin_action(None, "browse", "setConfig", json!({ "shareHome": true }))
        .await
        .unwrap();
    send(json!({ "action": "roots" })).await;
    let r = bridge_packet(&mut pev, "browse").await;
    assert!(
        r["body"]["roots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == "downloads")
    );
    wait(&mut desk.ev, |e| matches!(e, Event::Plugin { plugin, kind, .. } if plugin == "browse" && kind == "started")).await;

    send(json!({ "action": "list", "root": "downloads", "path": "Viaggi" })).await;
    let r = bridge_packet(&mut pev, "browse").await;
    let names: Vec<&str> = r["body"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["biglietto.pdf"]);

    send(json!({ "action": "get", "root": "downloads", "path": ".segreto" })).await;
    assert_eq!(
        bridge_packet(&mut pev, "browse").await["body"]["action"],
        "error"
    );
    send(json!({ "action": "list", "root": "downloads", "path": "../data" })).await;
    assert_eq!(
        bridge_packet(&mut pev, "browse").await["body"]["action"],
        "error"
    );

    send(json!({ "action": "get", "root": "downloads", "path": "Viaggi/biglietto.pdf" })).await;
    let r = bridge_packet(&mut pev, "browse").await;
    assert_eq!(r["body"]["action"], "file");
    assert_eq!(r["payloadSize"], 15);

    // Stopping from the computer ends the session.
    desk.k
        .plugin_action(None, "browse", "stop", json!({}))
        .await
        .unwrap();
    let s = desk
        .k
        .plugin_action(None, "browse", "sessions", json!({}))
        .await
        .unwrap();
    assert!(s.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn sms_conversations_and_send() {
    let (mut desk, phone, mut pev, _pdir) = desktop_and_bridge_phone().await;
    let (did, pid) = (
        desk.k.device_id().to_string(),
        phone.device_id().to_string(),
    );
    phone
        .set_permission(&did, Permission::Messages, true)
        .unwrap();
    desk.k
        .set_permission(&pid, Permission::Messages, true)
        .unwrap();

    let cached = desk
        .k
        .plugin_action(Some(&pid), "sms", "conversations", json!({}))
        .await
        .unwrap();
    assert_eq!(cached, json!([]));
    assert_eq!(
        bridge_packet(&mut pev, "sms").await["type"],
        types::SMS_CONVERSATIONS_REQUEST
    );
    phone
        .device_handle(&did)
        .unwrap()
        .send(Packet::new(
            types::SMS_MESSAGES,
            json!({ "version": 2, "messages": [{
                "_id": 41, "thread_id": 9, "date": 1_760_000_000_000i64, "type": 1, "read": 0, "event": 1,
                "body": "Arrivo alle 8", "addresses": [{ "address": "+393330000000" }]
            }] }),
        ))
        .await
        .unwrap();
    wait(
        &mut desk.ev,
        |e| matches!(e, Event::Plugin { plugin, kind, .. } if plugin == "sms" && kind == "updated"),
    )
    .await;
    let list = desk
        .k
        .plugin_action(Some(&pid), "sms", "conversations", json!({}))
        .await
        .unwrap();
    assert_eq!(list[0]["threadId"], 9);
    assert_eq!(list[0]["last"]["body"], "Arrivo alle 8");
    assert_eq!(list[0]["unread"], true);

    desk.k
        .plugin_action(
            Some(&pid),
            "sms",
            "send",
            json!({ "threadId": 9, "text": "Perfetto" }),
        )
        .await
        .unwrap();
    let r = loop {
        let r = bridge_packet(&mut pev, "sms").await;
        if r["type"] == types::SMS_SEND {
            break r;
        }
    };
    assert_eq!(r["body"]["messageBody"], "Perfetto");
    assert_eq!(r["body"]["addresses"][0]["address"], "+393330000000");
}

#[tokio::test]
async fn phone_camera_scan_and_photo_only_when_asked() {
    use konnekta_core::payload::PayloadSource;
    let (mut desk, phone, mut pev, _pdir) = desktop_and_bridge_phone().await;
    let (did, pid) = (
        desk.k.device_id().to_string(),
        phone.device_id().to_string(),
    );
    let h = phone.device_handle(&did).unwrap();
    let jpeg = b"\xFF\xD8\xFF\xE0 fake jpeg".to_vec();
    let photo = |name: &str| Packet::new(types::PHOTO, json!({ "filename": name }));

    // Unsolicited photo: refused, nothing written.
    h.send_with_payload(photo("spam.jpg"), PayloadSource::Bytes(jpeg.clone()), None)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let dl = desk.k.config().download_dir.clone();
    assert!(!dl.join("spam.jpg").exists());

    // Document scan: the recognised text lands on the clipboard.
    desk.k
        .plugin_action(Some(&pid), "photo", "take", json!({ "mode": "scan" }))
        .await
        .unwrap();
    let r = bridge_packet(&mut pev, "photo").await;
    assert_eq!(r["body"]["mode"], "scan");
    h.send(Packet::new(
        types::PHOTO,
        json!({ "text": "Fattura n. 12" }),
    ))
    .await
    .unwrap();
    wait(
        &mut desk.ev,
        |e| matches!(e, Event::Plugin { plugin, kind, .. } if plugin == "photo" && kind == "text"),
    )
    .await;
    assert_eq!(
        desk.mock.clipboard.lock().unwrap().as_deref(),
        Some("Fattura n. 12")
    );
    // One answer per request.
    h.send(Packet::new(types::PHOTO, json!({ "text": "again" })))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        desk.mock.clipboard.lock().unwrap().as_deref(),
        Some("Fattura n. 12")
    );

    // Photo: saved in the download folder.
    desk.k
        .plugin_action(Some(&pid), "photo", "take", json!({}))
        .await
        .unwrap();
    bridge_packet(&mut pev, "photo").await;
    h.send_with_payload(
        photo("../IMG_1.jpg"),
        PayloadSource::Bytes(jpeg.clone()),
        None,
    )
    .await
    .unwrap();
    wait(&mut desk.ev, |e| matches!(e, Event::Plugin { plugin, kind, .. } if plugin == "photo" && kind == "received")).await;
    assert_eq!(std::fs::read(dl.join("IMG_1.jpg")).unwrap(), jpeg);
}

/// The full fingerprint approval, as root would run it but without root:
/// the real PAM helper logic talks to the real daemon over its IPC socket,
/// and a simulated phone signs with a P-256 key.
#[cfg(unix)]
mod approve {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use konnekta_approve::helper::{Outcome, PamEnv, Settings, run};
    use konnekta_approve::key::{KeyFile, PublicKey};
    use konnekta_approve::message::Approval;
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};

    const P256_SPKI_PREFIX: [u8; 26] = [
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];

    fn phone_key() -> EcdsaKeyPair {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap()
    }

    #[derive(Clone, Copy)]
    enum Phone {
        Approve,
        Deny,
        /// Signs the request with another nonce (a forged or replayed answer).
        SignOther,
        Silent,
    }

    struct Setup {
        _desk: Node,
        _phone: Konnekta,
        _dirs: tempfile::TempDir,
        settings: Settings,
        env: PamEnv,
        pev: broadcast::Receiver<Event>,
        phone_handle: konnekta_core::DeviceHandle,
        key: EcdsaKeyPair,
    }

    async fn setup(enroll: bool) -> Setup {
        let (desk, phone, pev, pdir) = desktop_and_bridge_phone().await;
        let (did, pid) = (
            desk.k.device_id().to_string(),
            phone.device_id().to_string(),
        );
        phone
            .set_permission(&did, Permission::Approve, true)
            .unwrap();
        desk.k
            .set_permission(&pid, Permission::Approve, true)
            .unwrap();
        let dirs = tempfile::tempdir().unwrap();
        // The key folder check refuses symlinked ancestors (/var on macOS).
        let root = dirs.path().canonicalize().unwrap();
        let sock = root.join("run/konnekta.sock");
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        std::fs::set_permissions(
            sock.parent().unwrap(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        tokio::spawn(konnekta_core::ipc::listen(sock.clone(), desk.k.clone()));
        eventually(|| sock.exists()).await;
        let key = phone_key();
        let key_dir = root.join("etc/approve");
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::set_permissions(
            root.join("etc"),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        std::fs::set_permissions(root, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let uid = unsafe { libc_getuid() };
        if enroll {
            let spki = [&P256_SPKI_PREFIX[..], key.public_key().as_ref()].concat();
            let kf = KeyFile {
                device_id: pid.clone(),
                device_name: "Phone".into(),
                enrolled: 1,
                key: PublicKey::from_spki_der(&spki).unwrap(),
            };
            konnekta_approve::keyfile::write(&key_dir, "tester", &kf).unwrap();
        }
        let settings = Settings {
            key_dir,
            key_owner: uid,
            socket: Box::new(move |_| sock.clone()),
            host: "testhost".into(),
            lookup_uid: Box::new(move |u| (u == "tester").then_some(uid)),
        };
        let env = PamEnv {
            user: "tester".into(),
            service: "sudo".into(),
            tty: "/dev/pts/7".into(),
            pam_type: "auth".into(),
            real_uid: uid,
            ..Default::default()
        };
        let phone_handle = phone.device_handle(&did).unwrap();
        drop(pdir);
        Setup {
            _desk: desk,
            _phone: phone,
            _dirs: dirs,
            settings,
            env,
            pev,
            phone_handle,
            key,
        }
    }

    unsafe extern "C" {
        #[link_name = "getuid"]
        fn libc_getuid() -> u32;
    }

    /// Run the helper (blocking, like pam_exec) while the phone answers.
    async fn approve_with(
        phone: Phone,
        s: Setup,
    ) -> (Outcome, Vec<String>, Option<serde_json::Value>) {
        let Setup {
            settings,
            env,
            mut pev,
            phone_handle,
            key,
            _desk,
            _phone,
            _dirs,
        } = s;
        let helper = tokio::task::spawn_blocking(move || {
            let mut said = Vec::new();
            let o = run(&env, &settings, &mut |l| said.push(l.to_owned()));
            (o, said)
        });
        let request =
            tokio::time::timeout(Duration::from_secs(5), bridge_packet(&mut pev, "approve"))
                .await
                .ok();
        if let Some(r) = &request {
            let b = &r["body"];
            let mut a = Approval {
                host: b["host"].as_str().unwrap().into(),
                user: b["user"].as_str().unwrap().into(),
                service: b["service"].as_str().unwrap().into(),
                tty: b["tty"].as_str().unwrap().into(),
                rhost: b["rhost"].as_str().unwrap().into(),
                time: b["time"].as_u64().unwrap(),
                nonce: b["nonce"].as_str().unwrap().into(),
            };
            let id = b["id"].as_str().unwrap();
            let answer = match phone {
                Phone::Approve | Phone::SignOther => {
                    if matches!(phone, Phone::SignOther) {
                        a.nonce = "0".repeat(64);
                    }
                    let sig = key
                        .sign(&SystemRandom::new(), &a.message().unwrap())
                        .unwrap();
                    Some(
                        json!({ "id": id, "result": "approved", "signature": B64.encode(sig.as_ref()) }),
                    )
                }
                Phone::Deny => Some(json!({ "id": id, "result": "denied" })),
                Phone::Silent => None,
            };
            if let Some(body) = answer {
                phone_handle
                    .send(Packet::new(types::APPROVE, body))
                    .await
                    .unwrap();
            }
        }
        let (o, said) = helper.await.unwrap();
        (o, said, request.map(|r| r["body"].clone()))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fingerprint_approves_sudo() {
        let (o, said, req) = approve_with(Phone::Approve, setup(true).await).await;
        assert_eq!(o, Outcome::Approved);
        let req = req.unwrap();
        assert_eq!(
            (
                req["service"].as_str(),
                req["user"].as_str(),
                req["host"].as_str()
            ),
            (Some("sudo"), Some("tester"), Some("testhost"))
        );
        assert_eq!(req["nonce"].as_str().unwrap().len(), 64);
        assert_eq!(
            said,
            [
                "Approva su Phone per il terminale /dev/pts/7, oppure attendi la richiesta della password."
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn denial_and_forged_signatures_fall_back_to_password() {
        let (o, _, _) = approve_with(Phone::Deny, setup(true).await).await;
        assert!(
            matches!(o, Outcome::Fallback(ref m) if m.contains("denied")),
            "{o:?}"
        );
        let (o, _, _) = approve_with(Phone::SignOther, setup(true).await).await;
        assert_eq!(o, Outcome::Fallback("bad signature".into()));
    }

    /// The phone side of an enrollment: new key, signature over the
    /// enrollment message. Returns the key and the code the phone shows.
    async fn phone_enrolls(
        pev: &mut broadcast::Receiver<Event>,
        h: &konnekta_core::DeviceHandle,
    ) -> (EcdsaKeyPair, String) {
        use sha2::{Digest, Sha256};
        let r = bridge_packet(pev, "approve").await;
        let b = &r["body"];
        assert_eq!(b["kind"], "enroll");
        let key = phone_key();
        let spki = [&P256_SPKI_PREFIX[..], key.public_key().as_ref()].concat();
        let m = konnekta_approve::message::Enrollment {
            host: b["host"].as_str().unwrap().into(),
            user: b["user"].as_str().unwrap().into(),
            key: hex::encode(Sha256::digest(&spki)),
            time: b["time"].as_u64().unwrap(),
            nonce: b["nonce"].as_str().unwrap().into(),
        };
        let sig = key
            .sign(&SystemRandom::new(), &m.message().unwrap())
            .unwrap();
        h.send(Packet::new(
            types::APPROVE,
            json!({ "id": b["id"], "result": "approved", "publicKey": B64.encode(&spki), "signature": B64.encode(sig.as_ref()) }),
        ))
        .await
        .unwrap();
        let code = hex::encode_upper(&Sha256::digest(&spki)[..8]);
        (key, code)
    }

    fn enroll_settings(s: &Setup, uid: u32) -> konnekta_approve::enroll::Settings {
        konnekta_approve::enroll::Settings {
            key_dir: s.settings.key_dir.clone(),
            socket: (s.settings.socket)(uid),
            user: "tester".into(),
            uid,
            host: "testhost".into(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enroll_then_approve_with_the_new_key() {
        let mut s = setup(false).await;
        let uid = unsafe { libc_getuid() };
        let es = enroll_settings(&s, uid);
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let job = tokio::task::spawn_blocking(move || {
            let mut said = Vec::new();
            // A user who types nothing more ends the enrollment (no hang).
            let mut ask = |_: &str| {
                rx.recv_timeout(Duration::from_secs(5))
                    .map_err(std::io::Error::other)
            };
            let r = konnekta_approve::enroll::run(&es, "Phone", &mut ask, &mut |l| {
                said.push(l.to_owned())
            });
            (r, said)
        });
        let (key, code) = phone_enrolls(&mut s.pev, &s.phone_handle).await;
        // The user types the code from the phone screen, with spaces.
        tx.send("0000 0000 0000 0000\n".into()).unwrap();
        tx.send(format!("{} {}\n", code[..8].to_lowercase(), &code[8..]))
            .unwrap();
        let (r, said) = job.await.unwrap();
        let path = r.unwrap();
        assert!(said.iter().any(|l| l.contains("non corrisponde")));
        assert!(
            !said.iter().any(|l| l.contains(&code)),
            "the code is never printed"
        );
        let stored = konnekta_approve::keyfile::read(&path, uid).unwrap();
        assert_eq!(stored.device_name, "Phone");

        // The enrolled key now approves sudo; the old test key is not used.
        s.key = key;
        let (o, _, _) = approve_with(Phone::Approve, s).await;
        assert_eq!(o, Outcome::Approved);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wrong_code_saves_nothing() {
        let mut s = setup(false).await;
        let uid = unsafe { libc_getuid() };
        let es = enroll_settings(&s, uid);
        let key_dir = es.key_dir.clone();
        let job = tokio::task::spawn_blocking(move || {
            konnekta_approve::enroll::run(
                &es,
                "Phone",
                &mut |_| Ok("DEAD BEEF DEAD BEEF\n".into()),
                &mut |_| {},
            )
        });
        phone_enrolls(&mut s.pev, &s.phone_handle).await;
        assert!(
            job.await
                .unwrap()
                .unwrap_err()
                .contains("non corrispondono")
        );
        assert!(!key_dir.join("tester.pub").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_enrolled_key_asks_nothing() {
        let (o, said, req) = approve_with(Phone::Silent, setup(false).await).await;
        assert_eq!(o, Outcome::Fallback("no key enrolled".into()));
        assert!(said.is_empty() && req.is_none(), "the phone is never asked");
    }
}
