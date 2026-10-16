//! Real-time streams over secure side channels.
//!
//! | Plugin        | Direction        | Permission  | Local endpoint                       |
//! |---------------|------------------|-------------|--------------------------------------|
//! | microphone    | phone → computer | Microphone  | virtual microphone                   |
//! | camera        | phone → computer | Camera      | virtual webcam                       |
//! | phonescreen   | phone → computer | Desktop     | viewer window                        |
//! | desktop       | computer → phone | Desktop     | screen capture (H.264, adaptive)     |
//! | systemaudio   | computer → phone | SystemAudio | system audio capture (PCM 48 kHz)    |
//!
//! Sessions have random 128-bit ids, are bound to one paired device, end on
//! `stop`, disconnect, unpair or permission revocation, and the local user is
//! notified whenever the computer screen or audio is being shared.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::bodies::{StreamAction, StreamMessage, StreamParams};
use konnekta_core::proto::{Packet, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo, PluginOrigin, SecureChannel};
use konnekta_media::abr::{DEFAULT_LADDER, NetworkSample};
use konnekta_media::frame::{FLAG_CONFIG, Frame, FrameKind, read_frame};
use konnekta_media::pipeline::{AnnexBSplitter, ProcessPipeline, screen_capture_command};
use konnekta_media::virtual_devices::{SystemAudioCapture, VirtualCamera, VirtualMic, screen_viewer};
use konnekta_media::{AdaptiveBitrate, FrameSender};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::AbortHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Microphone,
    Camera,
    PhoneScreen,
    Desktop,
    SystemAudio,
}

impl Kind {
    fn produces_locally(self) -> bool {
        matches!(self, Self::Desktop | Self::SystemAudio)
    }
}

struct Session {
    id: String,
    task: Option<AbortHandle>,
    started: Instant,
}

pub struct StreamPlugin {
    kind: Kind,
    info: PluginInfo,
    me: std::sync::Weak<StreamPlugin>,
    core: OnceLock<CoreHandle>,
    sessions: Mutex<HashMap<String, Session>>,
}

fn session_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or_default()
}

impl StreamPlugin {
    fn make(kind: Kind, info: PluginInfo) -> Arc<Self> {
        Arc::new_cyclic(|me| Self { kind, info, me: me.clone(), core: OnceLock::new(), sessions: Mutex::default() })
    }

    pub fn microphone() -> Arc<Self> {
        Self::make(Kind::Microphone, PluginInfo {
            id: "microphone",
            name: "Phone microphone",
            description: "Use the phone as a wireless microphone (Discord, Zoom, OBS...)",
            origin: PluginOrigin::Konnekta,
            permission: Permission::Microphone,
            incoming: &[types::K_MICROPHONE],
            outgoing: &[types::K_MICROPHONE],
        })
    }

    pub fn camera() -> Arc<Self> {
        Self::make(Kind::Camera, PluginInfo {
            id: "camera",
            name: "Phone webcam",
            description: "Use the phone camera as a wireless webcam",
            origin: PluginOrigin::Konnekta,
            permission: Permission::Camera,
            incoming: &[types::K_CAMERA],
            outgoing: &[types::K_CAMERA],
        })
    }

    pub fn phone_screen() -> Arc<Self> {
        Self::make(Kind::PhoneScreen, PluginInfo {
            id: "phonescreen",
            name: "Phone screen",
            description: "Show the phone screen on the computer",
            origin: PluginOrigin::Konnekta,
            permission: Permission::Desktop,
            incoming: &[types::K_PHONE_SCREEN],
            outgoing: &[types::K_PHONE_SCREEN],
        })
    }

    pub fn desktop() -> Arc<Self> {
        Self::make(Kind::Desktop, PluginInfo {
            id: "desktop",
            name: "Remote desktop",
            description: "Share this screen with the phone and control it by touch",
            origin: PluginOrigin::Konnekta,
            permission: Permission::Desktop,
            incoming: &[types::K_DESKTOP],
            outgoing: &[types::K_DESKTOP],
        })
    }

    pub fn system_audio() -> Arc<Self> {
        Self::make(Kind::SystemAudio, PluginInfo {
            id: "systemaudio",
            name: "Computer audio on phone",
            description: "Listen to the computer audio on the phone",
            origin: PluginOrigin::Konnekta,
            permission: Permission::SystemAudio,
            incoming: &[types::K_SYSTEM_AUDIO],
            outgoing: &[types::K_SYSTEM_AUDIO],
        })
    }

    fn packet_type(&self) -> &'static str {
        self.info.incoming[0]
    }

    fn msg(&self, action: StreamAction, session: &str, port: Option<u16>, params: StreamParams, reason: Option<String>) -> Packet {
        Packet::new(self.packet_type(), StreamMessage { action, session: session.into(), port, params, reason })
    }

    fn stop_local(&self, device_id: &str) -> bool {
        match self.sessions.lock().unwrap().remove(device_id) {
            Some(s) => {
                if let Some(t) = s.task {
                    t.abort();
                }
                tracing::info!(plugin = self.info.id, device = device_id, secs = s.started.elapsed().as_secs(), "stream stopped");
                if let Some(c) = self.core.get() {
                    c.emit_plugin(Some(device_id), self.info.id, "stopped", json!({ "session": s.id }));
                }
                true
            }
            None => false,
        }
    }

    fn session_matches(&self, device_id: &str, session: &str) -> bool {
        self.sessions.lock().unwrap().get(device_id).is_some_and(|s| s.id == session)
    }

    /// Spawn `fut` as the session task; it removes itself when done.
    fn run_session<F>(&self, dev: &DeviceHandle, session: String, fut: F)
    where
        F: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let me = self.me.clone();
        let (dev_c, sid) = (dev.clone(), session.clone());
        let task = tokio::spawn(async move {
            let r = fut.await;
            let Some(me) = me.upgrade() else { return };
            if let Err(e) = &r {
                tracing::warn!(plugin = me.info.id, "stream ended: {e:#}");
                dev_c.emit(me.info.id, "error", json!({ "error": e.to_string() }));
            }
            if me.session_matches(dev_c.id(), &sid) {
                me.sessions.lock().unwrap().remove(dev_c.id());
                dev_c.emit(me.info.id, "stopped", json!({ "session": sid }));
                let reason = r.err().map(|e| e.to_string());
                let _ = dev_c.send_if_supported(me.msg(StreamAction::Stop, &sid, None, StreamParams::default(), reason)).await;
            }
        });
        self.sessions.lock().unwrap().insert(
            dev.id().to_owned(),
            Session { id: session.clone(), task: Some(task.abort_handle()), started: Instant::now() },
        );
        dev.emit(self.info.id, "started", json!({ "session": session }));
    }

    // --- consumer side (phone produces) -------------------------------------

    async fn consume(kind: Kind, mut ch: SecureChannel, params: StreamParams, title: String) -> Result<()> {
        match kind {
            Kind::Microphone => {
                if params.codec != "pcm_s16le" || params.sample_rate != 48_000 || params.channels != 1 {
                    bail!("unsupported microphone format {} {} Hz {} ch", params.codec, params.sample_rate, params.channels);
                }
                let mut mic = VirtualMic::create().await?;
                while let Some(f) = read_frame(&mut ch).await? {
                    if f.kind == FrameKind::Audio {
                        mic.write(&f.data).await?;
                    }
                }
            }
            Kind::Camera => {
                if params.codec != "h264" {
                    bail!("unsupported camera codec {}", params.codec);
                }
                let mut cam = VirtualCamera::create(params.width, params.height, params.fps.max(15)).await?;
                tracing::info!("phone camera available via {}", cam.description);
                while let Some(f) = read_frame(&mut ch).await? {
                    if f.kind == FrameKind::Video {
                        cam.write(&f.data).await?;
                    }
                }
            }
            Kind::PhoneScreen => {
                let mut viewer = screen_viewer(&title)?;
                let stdin = viewer.stdin.as_mut().context("viewer closed")?;
                while let Some(f) = read_frame(&mut ch).await? {
                    if f.kind == FrameKind::Video {
                        stdin.write_all(&f.data).await?;
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    // --- producer side (computer produces) ----------------------------------

    async fn produce_desktop(ch: SecureChannel, params: StreamParams) -> Result<()> {
        let sender = FrameSender::new(ch, 30);
        let max_level = DEFAULT_LADDER
            .iter()
            .rposition(|q| params.height == 0 || q.height <= params.height)
            .unwrap_or(0);
        let start = max_level.min(2);
        let mut abr = AdaptiveBitrate::new(DEFAULT_LADDER, start, max_level);
        loop {
            let q = abr.current();
            sender.push(Frame::control(&json!({ "quality": q })));
            let (prog, args) = screen_capture_command(q)?;
            let mut proc = ProcessPipeline::spawn(&prog, &args, false, true)?;
            let mut out = proc.stdout.take().context("capture has no output")?;
            let mut splitter = AnnexBSplitter::default();
            let mut buf = vec![0u8; 64 * 1024];
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    n = out.read(&mut buf) => {
                        let n = n?;
                        if n == 0 {
                            bail!("screen capture stopped");
                        }
                        for nal in splitter.push(&buf[..n]) {
                            let mut f = Frame::new(FrameKind::Video, now_us(), nal.data.clone());
                            if nal.is_key() { f = f.keyframe(); }
                            if nal.is_config() { f.flags |= FLAG_CONFIG; }
                            sender.push(f);
                        }
                        if sender.is_closed() {
                            return Ok(());
                        }
                    }
                    _ = tick.tick() => {
                        if sender.is_closed() {
                            return Ok(());
                        }
                        let s = NetworkSample { queue_delay: sender.stats.queue_delay(), dropped: sender.stats.take_dropped() };
                        if let Some(nq) = abr.update(s, Instant::now()) {
                            tracing::info!(?nq, "adapting desktop stream quality");
                            break;
                        }
                    }
                }
            }
            drop(proc);
        }
    }

    async fn produce_system_audio(ch: SecureChannel) -> Result<()> {
        let sender = FrameSender::new(ch, 50);
        let mut cap = SystemAudioCapture::start().await?;
        while let Some(chunk) = cap.rx.recv().await {
            sender.push(Frame::new(FrameKind::Audio, now_us(), chunk));
            if sender.is_closed() {
                break;
            }
        }
        Ok(())
    }

    async fn start_producer(&self, dev: &DeviceHandle, session: String, params: StreamParams) -> Result<()> {
        self.stop_local(dev.id());
        let (port, accept) = dev.listen_channel().await?;
        let out_params = match self.kind {
            Kind::Desktop => StreamParams { codec: "h264".into(), ..params.clone() },
            _ => StreamParams { codec: "pcm_s16le".into(), sample_rate: 48_000, channels: 2, ..Default::default() },
        };
        dev.send(self.msg(StreamAction::Offer, &session, Some(port), out_params, None)).await?;
        if let Some(c) = self.core.get() {
            let what = if self.kind == Kind::Desktop { "screen" } else { "audio" };
            c.emit_plugin(
                Some(dev.id()),
                self.info.id,
                "sharing",
                json!({ "message": format!("Your {what} is being shared with {}", dev.name()) }),
            );
        }
        let kind = self.kind;
        self.run_session(dev, session, async move {
            let ch = accept.await?;
            match kind {
                Kind::Desktop => Self::produce_desktop(ch, params).await,
                _ => Self::produce_system_audio(ch).await,
            }
        });
        Ok(())
    }
}

#[async_trait]
impl Plugin for StreamPlugin {
    fn info(&self) -> PluginInfo {
        self.info.clone()
    }

    async fn start(self: Arc<Self>, core: CoreHandle) {
        let _ = self.core.set(core.clone());
        // Stop sessions whose permission was revoked.
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            while core.is_alive() {
                tick.tick().await;
                let ids: Vec<String> = self.sessions.lock().unwrap().keys().cloned().collect();
                for id in ids {
                    let ok = core.device(&id).is_some_and(|d| d.has_permission(self.info.permission));
                    if !ok {
                        self.stop_local(&id);
                    }
                }
            }
        });
    }

    async fn on_disconnected(&self, device_id: &str) {
        self.stop_local(device_id);
    }

    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        let m: StreamMessage = p.body_as()?;
        if m.session.len() > 64 {
            bail!("invalid session id");
        }
        match (m.action, self.kind.produces_locally()) {
            (StreamAction::Request, true) => self.start_producer(dev, m.session, m.params).await?,
            (StreamAction::Offer, false) => {
                let port = m.port.context("offer without port")?;
                self.stop_local(dev.id());
                let ch = dev.connect_channel(port).await?;
                let title = format!("{} — Konnekta", dev.name());
                self.run_session(dev, m.session, Self::consume(self.kind, ch, m.params, title));
            }
            (StreamAction::Stop, _) => {
                if self.session_matches(dev.id(), &m.session) {
                    self.stop_local(dev.id());
                }
            }
            (StreamAction::Keyframe, true) => {
                // The encoder emits SPS/PPS with every IDR (short GOP); nothing to do.
            }
            _ => tracing::debug!(plugin = self.info.id, action = ?m.action, "ignored stream message"),
        }
        Ok(())
    }

    async fn handle_action(&self, dev: Option<&DeviceHandle>, action: &str, args: Value) -> Result<Value> {
        let d = crate::need_dev(dev)?;
        match action {
            "start" => {
                if self.kind.produces_locally() {
                    bail!("this stream is started from the phone");
                }
                let params: StreamParams = crate::args(args).unwrap_or_default();
                let session = session_id();
                d.send(self.msg(StreamAction::Request, &session, None, params, None)).await?;
                Ok(json!({ "session": session }))
            }
            "configure" => {
                let params: StreamParams = crate::args(args)?;
                let sid = self.sessions.lock().unwrap().get(d.id()).map(|s| s.id.clone()).context("no active session")?;
                d.send(self.msg(StreamAction::Configure, &sid, None, params, None)).await?;
                Ok(Value::Null)
            }
            "stop" => {
                let sid = self.sessions.lock().unwrap().get(d.id()).map(|s| s.id.clone());
                if let Some(sid) = sid {
                    self.stop_local(d.id());
                    d.send_if_supported(self.msg(StreamAction::Stop, &sid, None, StreamParams::default(), Some("stopped by user".into())))
                        .await?;
                }
                Ok(Value::Null)
            }
            "status" => {
                let s = self.sessions.lock().unwrap().get(d.id()).map(|s| json!({ "session": s.id, "seconds": s.started.elapsed().as_secs() }));
                Ok(s.unwrap_or(Value::Null))
            }
            _ => bail!("unknown action {action}"),
        }
    }
}
