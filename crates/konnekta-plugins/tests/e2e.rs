//! End-to-end plugin tests: two full Konnekta nodes (all plugins) with mock
//! platforms, talking over real TLS on loopback.

use konnekta_core::proto::{Packet, types};
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
    let dir = tempfile::tempdir().unwrap();
    let mock = MockPlatform::new();
    let cfg = Config {
        device_name: name.into(),
        bind_addr: LO,
        tcp_port: 0,
        udp_port: 0,
        announce: false,
        download_dir: dir.path().join("downloads"),
        ..Config::default()
    };
    let k = Konnekta::builder(cfg)
        .data_dir(dir.path().join("data"))
        .plugins(konnekta_plugins::all(mock.platform()))
        .build()
        .unwrap();
    let ev = k.subscribe();
    k.start().await.unwrap();
    Node { k, mock, ev, _dir: dir }
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
    a.k.connect_to(SocketAddr::new(LO, b.k.udp_port())).await.unwrap();
    wait(&mut a.ev, |e| matches!(e, Event::DeviceConnected { .. })).await;
    wait(&mut b.ev, |e| matches!(e, Event::DeviceConnected { .. })).await;
    let (aid, bid) = (a.k.device_id().to_string(), b.k.device_id().to_string());
    a.k.request_pair(&bid).await.unwrap();
    wait(&mut b.ev, |e| matches!(e, Event::PairRequested { .. })).await;
    b.k.accept_pair(&aid).await.unwrap();
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
    let sets = |m: &MockPlatform| m.calls().iter().filter(|c| c.starts_with("clipboard.set")).count();
    assert_eq!(sets(&a.mock), 1, "desktop applied exactly one remote update");
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
    b.k.plugin_action(None, "share", "setConfig", json!({ "destination": dest.path() })).await.unwrap();
    a.k.plugin_action(Some(&bid), "share", "sendFiles", json!({ "paths": [path] })).await.unwrap();
    wait(&mut b.ev, |e| matches!(e, Event::Plugin { plugin, data, .. } if plugin == "share" && data["state"] == "completed")).await;
    let got = std::fs::read(dest.path().join("report.bin")).unwrap();
    assert_eq!(Sha256::digest(&got), Sha256::digest(&data));
    // No leftover partial files.
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 1);
    let t = b.k.plugin_action(None, "share", "transfers", json!(null)).await.unwrap();
    assert_eq!(t[0]["direction"], "incoming");
}

#[tokio::test]
async fn commands_need_permission_and_run_by_key() {
    let (a, b) = paired_pair().await;
    let (aid, bid) = (a.k.device_id().to_string(), b.k.device_id().to_string());
    let r = a
        .k
        .plugin_action(None, "runcommand", "add", json!({ "name": "Lock", "actions": [{ "type": "shell", "command": "lock-now" }] }))
        .await
        .unwrap();
    let key = r["id"].as_str().unwrap().to_owned();
    // Phone grants itself the right to send, desktop has not granted it yet.
    b.k.set_permission(&aid, Permission::Commands, true).unwrap();
    b.k.plugin_action(Some(&aid), "runcommand", "runRemote", json!({ "key": key })).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!a.mock.calls().iter().any(|c| c == "shell:lock-now"));
    a.k.set_permission(&bid, Permission::Commands, true).unwrap();
    b.k.plugin_action(Some(&aid), "runcommand", "runRemote", json!({ "key": key })).await.unwrap();
    eventually(|| a.mock.calls().iter().any(|c| c == "shell:lock-now")).await;
    // Unknown keys never execute anything.
    b.k.plugin_action(Some(&aid), "runcommand", "runRemote", json!({ "key": "rm -rf /" })).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(a.mock.calls().iter().filter(|c| c.starts_with("shell:")).count(), 1);
}

#[tokio::test]
async fn deck_press_from_phone_runs_macro() {
    let (a, b) = paired_pair().await;
    let (aid, bid) = (a.k.device_id().to_string(), b.k.device_id().to_string());
    a.k.set_permission(&bid, Permission::Deck, true).unwrap();
    b.k.set_permission(&aid, Permission::Deck, true).unwrap();
    let deck = a.k.plugin_action(None, "deck", "get", json!(null)).await.unwrap();
    let page = &deck["profiles"][0]["pages"][0];
    let next = page["buttons"].as_array().unwrap().iter().find(|x| x["label"] == "Successivo").unwrap();
    // The phone side has no Deck UI in tests: send the raw request packet.
    let h = b.k.device_handle(&aid).unwrap();
    h.send(Packet::new(
        types::K_DECK_REQUEST,
        json!({ "request": "press", "profile": "work", "page": page["id"], "button": next["id"] }),
    ))
    .await
    .unwrap();
    eventually(|| a.mock.calls().iter().any(|c| c == "media:Next")).await;
}

#[tokio::test]
async fn screenshots_are_saved_deduplicated_and_copied() {
    let (mut a, b) = paired_pair().await;
    let aid = a.k.device_id().to_string();
    let folder = tempfile::tempdir().unwrap();
    a.k.plugin_action(None, "screenshot", "setConfig", json!({ "save": true, "folder": folder.path(), "copyToClipboard": true, "autoTransfer": true }))
        .await
        .unwrap();
    let png: Vec<u8> = [&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..], &[42u8; 5000][..]].concat();
    let hash = hex::encode(Sha256::digest(&png));
    let h = b.k.device_handle(&aid).unwrap();
    for _ in 0..2 {
        let p = Packet::new(types::K_SCREENSHOT, json!({ "filename": "Screenshot_1.png", "sha256": hash }));
        let t = h.send_with_payload(p, konnekta_core::payload::PayloadSource::Bytes(png.clone()), None).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), t).await;
    }
    wait(&mut a.ev, |e| matches!(e, Event::Plugin { plugin, kind, .. } if plugin == "screenshot" && kind == "received")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 1, "duplicate not saved twice");
    assert_eq!(a.mock.png.lock().unwrap().as_deref(), Some(&png[..]));
}

#[tokio::test]
async fn notifications_are_shown_and_filtered() {
    let (a, b) = paired_pair().await;
    let aid = a.k.device_id().to_string();
    a.k.plugin_action(None, "notifications", "setFilter", json!({ "blockedApps": ["Spam"], "showPopups": true }))
        .await
        .unwrap();
    let h = b.k.device_handle(&aid).unwrap();
    for (app, title) in [("Spam", "buy"), ("Signal", "hello")] {
        h.send(Packet::new(types::NOTIFICATION, json!({ "id": app, "appName": app, "title": title, "text": "x", "isClearable": true })))
            .await
            .unwrap();
    }
    eventually(|| a.mock.calls().iter().any(|c| c == "notify:hello:x")).await;
    assert!(!a.mock.calls().iter().any(|c| c.contains("buy")));
}
