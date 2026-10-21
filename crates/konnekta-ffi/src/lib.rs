//! C ABI for mobile apps (Flutter `dart:ffi`, Kotlin JNI shims, Swift).
//!
//! All strings are UTF-8, NUL-terminated. Strings returned by the library must
//! be released with [`konnekta_free_string`]. Calls are thread-safe; blocking
//! calls (`konnekta_call`, `konnekta_next_event`, stream I/O) should be made
//! from a background isolate / thread.
//!
//! See `include/konnekta.h`.

use anyhow::{Context, Result};
use konnekta_core::proto::Packet;
use konnekta_core::{Config, Konnekta, SecureChannel};
use konnekta_media::FrameSender;
use konnekta_media::frame::{Frame, FrameKind, read_frame};
use konnekta_plugins::bridge::{PendingPayloads, mobile};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::runtime::Runtime;

enum Stream {
    Writer(FrameSender),
    Reader(Arc<Mutex<mpsc::Receiver<Frame>>>),
    Pending,
}

pub struct Handle {
    rt: Runtime,
    k: Konnekta,
    events: Mutex<mpsc::Receiver<String>>,
    pending: Arc<PendingPayloads>,
    streams: Arc<Mutex<HashMap<u64, Stream>>>,
    next_stream: AtomicU64,
}

fn cstr(p: *const c_char) -> Result<String> {
    if p.is_null() {
        return Ok(String::new());
    }
    Ok(unsafe { CStr::from_ptr(p) }
        .to_str()
        .context("invalid UTF-8")?
        .to_owned())
}

fn out(s: String) -> *mut c_char {
    CString::new(s.replace('\0', ""))
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

fn start(config_json: &str, data_dir: &str) -> Result<Handle> {
    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::layer::SubscriberExt;
        if let Ok(layer) = tracing_android::layer("Konnekta") {
            let _ =
                tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer));
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = tracing_subscriber::fmt().with_env_filter("info").try_init();

    let v: Value = if config_json.is_empty() {
        json!({})
    } else {
        serde_json::from_str(config_json)?
    };
    let mut cfg = Config::default();
    if let Some(n) = v.get("deviceName").and_then(Value::as_str) {
        cfg.device_name = n.into();
    }
    if let Some(t) = v.get("deviceType") {
        cfg.device_type = serde_json::from_value(t.clone()).unwrap_or_default();
    }
    if let Some(d) = v.get("downloadDir").and_then(Value::as_str) {
        cfg.download_dir = d.into();
    }
    if let Some(a) = v.get("announce").and_then(Value::as_bool) {
        cfg.announce = a;
    }
    if let Some(a) = v.get("mdns").and_then(Value::as_bool) {
        cfg.mdns = a;
    }
    // Interop adapters: {"kdeConnect": bool, "flux": bool}.
    for (key, p) in [
        ("kdeConnect", konnekta_core::proto::Protocol::KdeConnect),
        ("flux", konnekta_core::proto::Protocol::Flux),
    ] {
        if let (Some(on), Some(a)) = (v.get(key).and_then(Value::as_bool), cfg.adapter_mut(p)) {
            a.enabled = on;
        }
    }
    if let Some(p) = v.get("tcpPort").and_then(Value::as_u64) {
        cfg.tcp_port = p as u16;
    }
    if let Some(p) = v.get("udpPort").and_then(Value::as_u64) {
        cfg.udp_port = p as u16;
    }
    let call_control = v
        .get("callControl")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let pending = Arc::new(PendingPayloads::default());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let dir = PathBuf::from(data_dir);
    let k = rt.block_on(async {
        let k = Konnekta::builder(cfg)
            .data_dir(&dir)
            .plugins(mobile(pending.clone(), call_control))
            .build()?;
        k.start().await?;
        anyhow::Ok(k)
    })?;
    let (tx, rx) = mpsc::channel();
    let mut sub = k.subscribe();
    rt.spawn(async move {
        loop {
            match sub.recv().await {
                Ok(ev) => {
                    if tx
                        .send(serde_json::to_string(&ev).unwrap_or_default())
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    let _ = tx.send(json!({ "event": "lagged", "missed": n }).to_string());
                }
                Err(_) => break,
            }
        }
    });
    Ok(Handle {
        rt,
        k,
        events: Mutex::new(rx),
        pending,
        streams: Arc::default(),
        next_stream: AtomicU64::new(1),
    })
}

impl Handle {
    fn dev(&self, p: &Value) -> Result<konnekta_core::DeviceHandle> {
        let id = p
            .get("deviceId")
            .and_then(Value::as_str)
            .context("missing deviceId")?;
        self.k
            .device_handle(id)
            .context("device not connected or not paired")
    }

    fn new_stream(&self, s: Stream) -> u64 {
        let id = self.next_stream.fetch_add(1, Ordering::Relaxed);
        self.streams.lock().unwrap().insert(id, s);
        id
    }

    fn attach_channel(
        streams: &Arc<Mutex<HashMap<u64, Stream>>>,
        id: u64,
        ch: SecureChannel,
        write: bool,
    ) {
        let s = if write {
            Stream::Writer(FrameSender::new(ch, 30))
        } else {
            let (tx, rx) = mpsc::sync_channel::<Frame>(60);
            let mut ch = ch;
            tokio::spawn(async move {
                while let Ok(Some(f)) = read_frame(&mut ch).await {
                    // Drop when the app is not reading fast enough.
                    if let Err(mpsc::TrySendError::Disconnected(_)) = tx.try_send(f) {
                        break;
                    }
                }
            });
            Stream::Reader(Arc::new(Mutex::new(rx)))
        };
        if let Some(slot) = streams.lock().unwrap().get_mut(&id) {
            *slot = s;
        }
    }

    async fn call(&self, method: &str, p: Value) -> Result<Value> {
        Ok(match method {
            // Send a packet on behalf of a bridge plugin (permission-checked).
            "sendPacket" => {
                let ty = p
                    .get("type")
                    .and_then(Value::as_str)
                    .context("missing type")?;
                self.dev(&p)?
                    .send(Packet::new(ty, p.get("body").cloned().unwrap_or(json!({}))))
                    .await?;
                Value::Null
            }
            "sendFile" => {
                let d = self.dev(&p)?;
                let ty = p
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or(konnekta_core::proto::types::SHARE);
                let path = p
                    .get("path")
                    .and_then(Value::as_str)
                    .context("missing path")?;
                let packet = Packet::new(ty, p.get("body").cloned().unwrap_or(json!({})));
                let task = d
                    .send_with_payload(
                        packet,
                        konnekta_core::payload::PayloadSource::File(path.into()),
                        None,
                    )
                    .await?;
                task.await??;
                Value::Null
            }
            "fetchPayload" => {
                let key = p
                    .get("payloadKey")
                    .and_then(Value::as_str)
                    .context("missing payloadKey")?;
                let path = p
                    .get("path")
                    .and_then(Value::as_str)
                    .context("missing path")?;
                let (dev_id, packet) = self
                    .pending
                    .take(key)
                    .context("payload expired or unknown")?;
                let d = self.k.device_handle(&dev_id).context("device gone")?;
                let mut f = tokio::fs::File::create(path).await?;
                let n = d.fetch_payload(&packet, &mut f, None, None).await?;
                json!({ "bytes": n })
            }
            // Producer side: listen, announce the port in an offer, then write frames.
            "streamListen" => {
                let d = self.dev(&p)?;
                let (port, accept) = d.listen_channel().await?;
                let id = self.new_stream(Stream::Pending);
                let streams = self.streams.clone();
                tokio::spawn(async move {
                    match accept.await {
                        Ok(ch) => Self::attach_channel(&streams, id, ch, true),
                        Err(e) => {
                            tracing::warn!("stream accept failed: {e}");
                            streams.lock().unwrap().remove(&id);
                        }
                    }
                });
                json!({ "port": port, "channelId": id })
            }
            // Consumer side: connect to an offered port, then read frames.
            "streamConnect" => {
                let d = self.dev(&p)?;
                let port = p
                    .get("port")
                    .and_then(Value::as_u64)
                    .context("missing port")? as u16;
                let ch = d.connect_channel(port).await?;
                let id = self.new_stream(Stream::Pending);
                Self::attach_channel(&self.streams, id, ch, false);
                json!({ "channelId": id })
            }
            "streamClose" => {
                let id = p.get("channelId").and_then(Value::as_u64).unwrap_or(0);
                self.streams.lock().unwrap().remove(&id);
                Value::Null
            }
            _ => konnekta_core::api::call(&self.k, method, p).await?,
        })
    }
}

/// Start the core. `config_json` may contain `deviceName`, `deviceType`
/// ("phone"/"tablet"), `downloadDir`, `announce`, `callControl`.
/// Returns NULL on failure (details in the log).
/// # Safety
/// Pointer arguments must be NULL or valid for the documented use: handles
/// returned by `konnekta_start` and not yet stopped, NUL-terminated UTF-8
/// strings, and buffers of at least the given length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn konnekta_start(
    config_json: *const c_char,
    data_dir: *const c_char,
) -> *mut Handle {
    let r = (|| start(&cstr(config_json)?, &cstr(data_dir)?))();
    match r {
        Ok(h) => Box::into_raw(Box::new(h)),
        Err(e) => {
            tracing::error!("konnekta_start failed: {e:#}");
            std::ptr::null_mut()
        }
    }
}

/// Call an API method (see `konnekta_core::api` plus `sendPacket`,
/// `sendFile`, `fetchPayload`, `streamListen`, `streamConnect`,
/// `streamClose`). Returns `{"result": ...}` or `{"error": "..."}`.
/// # Safety
/// Pointer arguments must be NULL or valid for the documented use: handles
/// returned by `konnekta_start` and not yet stopped, NUL-terminated UTF-8
/// strings, and buffers of at least the given length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn konnekta_call(
    h: *mut Handle,
    method: *const c_char,
    params_json: *const c_char,
) -> *mut c_char {
    let Some(h) = (unsafe { h.as_ref() }) else {
        return out(json!({ "error": "null handle" }).to_string());
    };
    let r = (|| -> Result<Value> {
        let method = cstr(method)?;
        let raw = cstr(params_json)?;
        let params: Value = if raw.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&raw)?
        };
        h.rt.block_on(h.call(&method, params))
    })();
    out(match r {
        Ok(v) => json!({ "result": v }).to_string(),
        Err(e) => json!({ "error": format!("{e:#}") }).to_string(),
    })
}

/// Wait up to `timeout_ms` for the next event (JSON). Returns NULL on timeout.
/// # Safety
/// Pointer arguments must be NULL or valid for the documented use: handles
/// returned by `konnekta_start` and not yet stopped, NUL-terminated UTF-8
/// strings, and buffers of at least the given length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn konnekta_next_event(h: *mut Handle, timeout_ms: i32) -> *mut c_char {
    let Some(h) = (unsafe { h.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let rx = h.events.lock().unwrap();
    match rx.recv_timeout(Duration::from_millis(timeout_ms.max(0) as u64)) {
        Ok(s) => out(s),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Queue a media frame on a producer channel.
/// Returns 0 = queued, 1 = dropped (congestion), -1 = closed / unknown.
/// # Safety
/// Pointer arguments must be NULL or valid for the documented use: handles
/// returned by `konnekta_start` and not yet stopped, NUL-terminated UTF-8
/// strings, and buffers of at least the given length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn konnekta_stream_write(
    h: *mut Handle,
    channel: u64,
    kind: u8,
    flags: u8,
    pts_us: u64,
    data: *const u8,
    len: usize,
) -> i32 {
    let Some(h) = (unsafe { h.as_ref() }) else {
        return -1;
    };
    if data.is_null() && len > 0 {
        return -1;
    }
    let kind = match kind {
        1 => FrameKind::Audio,
        2 => FrameKind::Video,
        _ => FrameKind::Control,
    };
    let bytes = if len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(data, len) }.to_vec()
    };
    let streams = h.streams.lock().unwrap();
    match streams.get(&channel) {
        Some(Stream::Writer(w)) if !w.is_closed() => {
            let _guard = h.rt.enter();
            let mut f = Frame::new(kind, pts_us, bytes);
            f.flags = flags;
            if w.push(f) { 0 } else { 1 }
        }
        Some(Stream::Pending) => 1,
        _ => -1,
    }
}

/// Read the next frame from a consumer channel into `buf`.
/// Returns the frame length, 0 on timeout, -1 when closed, -2 if `cap` is too
/// small (the frame is discarded). `meta` (may be NULL) receives
/// `[kind, flags]` and `pts_us`.
/// # Safety
/// Pointer arguments must be NULL or valid for the documented use: handles
/// returned by `konnekta_start` and not yet stopped, NUL-terminated UTF-8
/// strings, and buffers of at least the given length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn konnekta_stream_read(
    h: *mut Handle,
    channel: u64,
    buf: *mut u8,
    cap: usize,
    meta: *mut u8,
    pts_us: *mut u64,
    timeout_ms: i32,
) -> i64 {
    let Some(h) = (unsafe { h.as_ref() }) else {
        return -1;
    };
    let rx = match h.streams.lock().unwrap().get(&channel) {
        Some(Stream::Reader(rx)) => rx.clone(),
        _ => return -1,
    };
    let frame = {
        // The map lock is released here: waiting only blocks this channel.
        let rx = rx.lock().unwrap();
        match rx.recv_timeout(Duration::from_millis(timeout_ms.max(0) as u64)) {
            Ok(f) => f,
            Err(mpsc::RecvTimeoutError::Timeout) => return 0,
            Err(mpsc::RecvTimeoutError::Disconnected) => return -1,
        }
    };
    if frame.data.len() > cap || buf.is_null() {
        return -2;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(frame.data.as_ptr(), buf, frame.data.len());
        if !meta.is_null() {
            *meta = frame.kind as u8;
            *meta.add(1) = frame.flags;
        }
        if !pts_us.is_null() {
            *pts_us = frame.pts_us;
        }
    }
    frame.data.len() as i64
}

/// # Safety
/// Pointer arguments must be NULL or valid for the documented use: handles
/// returned by `konnekta_start` and not yet stopped, NUL-terminated UTF-8
/// strings, and buffers of at least the given length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn konnekta_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Stop the core and free the handle.
/// # Safety
/// Pointer arguments must be NULL or valid for the documented use: handles
/// returned by `konnekta_start` and not yet stopped, NUL-terminated UTF-8
/// strings, and buffers of at least the given length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn konnekta_stop(h: *mut Handle) {
    if h.is_null() {
        return;
    }
    let h = unsafe { Box::from_raw(h) };
    h.k.shutdown();
    h.streams.lock().unwrap().clear();
    h.rt.shutdown_timeout(Duration::from_secs(2));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_call_stop() {
        unsafe {
            let dir =
                std::env::temp_dir().join(format!("konnekta-ffi-test-{}", std::process::id()));
            let cfg = CString::new(
                r#"{"deviceName":"Test Phone","deviceType":"phone","announce":false,"mdns":false,"tcpPort":0,"udpPort":0}"#,
            )
            .unwrap();
            let d = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
            let h = konnekta_start(cfg.as_ptr(), d.as_ptr());
            assert!(!h.is_null());
            let m = CString::new("info").unwrap();
            let r = konnekta_call(h, m.as_ptr(), std::ptr::null());
            let s = CStr::from_ptr(r).to_str().unwrap().to_owned();
            konnekta_free_string(r);
            let v: Value = serde_json::from_str(&s).unwrap();
            assert_eq!(v["result"]["deviceName"], "Test Phone");
            let m = CString::new("bogus").unwrap();
            let r = konnekta_call(h, m.as_ptr(), std::ptr::null());
            assert!(CStr::from_ptr(r).to_str().unwrap().contains("error"));
            konnekta_free_string(r);
            assert!(konnekta_next_event(h, 10).is_null());
            assert_eq!(
                konnekta_stream_write(h, 99, 1, 0, 0, std::ptr::null(), 0),
                -1
            );
            konnekta_stop(h);
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
