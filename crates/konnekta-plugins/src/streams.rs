//! Real-time streams over secure side channels.
//!
//! | Plugin        | Direction        | Permission  | Local endpoint                       |
//! |---------------|------------------|-------------|--------------------------------------|
//! | microphone    | phone → computer | Microphone  | virtual microphone                   |
//! | camera        | phone → computer | Camera      | virtual webcam                       |
//! | phonescreen   | phone → computer | Desktop     | viewer window                        |
//! | desktop       | computer → phone | Desktop     | screen capture (H.264, adaptive)     |
//! | extend        | computer → phone | Desktop     | virtual monitor of the phone's size  |
//! | systemaudio   | computer → phone | SystemAudio | system audio capture (PCM 48 kHz)    |
//!
//! Sessions have random 128-bit ids, are bound to one paired device, end on
//! `stop`, disconnect, unpair or permission revocation, and the local user is
//! notified whenever the computer screen or audio is being shared.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use konnekta_core::proto::bodies::{StreamAction, StreamMessage, StreamParams};
use konnekta_core::proto::{Packet, types};
use konnekta_core::{CoreHandle, DeviceHandle, Permission, Plugin, PluginInfo, SecureChannel};
use konnekta_media::FrameSender;
use konnekta_media::abr::DEFAULT_LADDER;
use konnekta_media::flv::{FlvReader, FlvVideo};
use konnekta_media::frame::{FLAG_CONFIG, Frame, FrameKind, read_frame};
use konnekta_media::pipeline::{ProcessPipeline, screen_capture_command};
use konnekta_media::virtual_devices::{
    Speakers, SystemAudioCapture, VirtualCamera, VirtualMic, screen_viewer,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::task::AbortHandle;

/// Why audio and video stop at a Bluetooth link.
pub const BLUETOOTH_REASON: &str = "audio e video non sono disponibili con il collegamento Bluetooth (troppo lento): usa Wi-Fi o il cavo";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Microphone,
    Camera,
    PhoneScreen,
    Desktop,
    SystemAudio,
    PhoneAudio,
    /// The phone as a second monitor.
    Extend,
}

impl Kind {
    /// Feature id in the setup assistant ([`konnekta_media::setup`]).
    fn feature(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::Camera => "webcam",
            Self::PhoneScreen => "phoneScreen",
            Self::Desktop => "desktop",
            Self::SystemAudio => "systemAudio",
            Self::PhoneAudio => "phoneAudio",
            Self::Extend => "desktop",
        }
    }

    fn produces_locally(self) -> bool {
        matches!(self, Self::Desktop | Self::SystemAudio | Self::Extend)
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
        Arc::new_cyclic(|me| Self {
            kind,
            info,
            me: me.clone(),
            core: OnceLock::new(),
            sessions: Mutex::default(),
        })
    }

    pub fn microphone() -> Arc<Self> {
        Self::make(
            Kind::Microphone,
            PluginInfo {
                id: "microphone",
                name: "Phone microphone",
                description: "Use the phone as a wireless microphone (Discord, Zoom, OBS...)",
                permission: Permission::Microphone,
                incoming: &[types::MICROPHONE],
                outgoing: &[types::MICROPHONE],
            },
        )
    }

    pub fn camera() -> Arc<Self> {
        Self::make(
            Kind::Camera,
            PluginInfo {
                id: "camera",
                name: "Phone webcam",
                description: "Use the phone camera as a wireless webcam",
                permission: Permission::Camera,
                incoming: &[types::CAMERA],
                outgoing: &[types::CAMERA],
            },
        )
    }

    pub fn phone_screen() -> Arc<Self> {
        Self::make(
            Kind::PhoneScreen,
            PluginInfo {
                id: "phonescreen",
                name: "Phone screen",
                description: "Show the phone screen on the computer",
                permission: Permission::Desktop,
                incoming: &[types::PHONE_SCREEN],
                outgoing: &[types::PHONE_SCREEN],
            },
        )
    }

    pub fn phone_audio() -> Arc<Self> {
        Self::make(
            Kind::PhoneAudio,
            PluginInfo {
                id: "phoneaudio",
                name: "Phone audio",
                description: "Play what the phone is playing on this computer's speakers",
                permission: Permission::SystemAudio,
                incoming: &[types::PHONE_AUDIO],
                outgoing: &[types::PHONE_AUDIO],
            },
        )
    }

    pub fn desktop() -> Arc<Self> {
        Self::make(
            Kind::Desktop,
            PluginInfo {
                id: "desktop",
                name: "Remote desktop",
                description: "Share this screen with the phone and control it by touch",
                permission: Permission::Desktop,
                incoming: &[types::DESKTOP],
                outgoing: &[types::DESKTOP],
            },
        )
    }

    pub fn extend() -> Arc<Self> {
        Self::make(
            Kind::Extend,
            PluginInfo {
                id: "extend",
                name: "Phone as second screen",
                description: "Use the phone as an extra monitor of this computer",
                permission: Permission::Desktop,
                incoming: &[types::EXTEND],
                outgoing: &[types::EXTEND],
            },
        )
    }

    pub fn system_audio() -> Arc<Self> {
        Self::make(
            Kind::SystemAudio,
            PluginInfo {
                id: "systemaudio",
                name: "Computer audio on phone",
                description: "Listen to the computer audio on the phone",
                permission: Permission::SystemAudio,
                incoming: &[types::SYSTEM_AUDIO],
                outgoing: &[types::SYSTEM_AUDIO],
            },
        )
    }

    fn packet_type(&self) -> &'static str {
        self.info.incoming[0]
    }

    fn msg(
        &self,
        action: StreamAction,
        session: &str,
        port: Option<u16>,
        params: StreamParams,
        reason: Option<String>,
    ) -> Packet {
        Packet::new(
            self.packet_type(),
            StreamMessage {
                action,
                session: session.into(),
                port,
                params,
                reason,
                missing: vec![],
                fix: vec![],
            },
        )
    }

    /// What this computer still needs for the stream, checked off the
    /// async threads (it looks for programs and kernel modules).
    async fn needs(kind: Kind) -> (Vec<String>, Vec<String>) {
        tokio::task::spawn_blocking(move || konnekta_media::setup::needs(kind.feature()))
            .await
            .unwrap_or_default()
    }

    /// Refuse a stream because something is missing here: the phone shows
    /// what and how to install it.
    async fn refuse_missing(
        &self,
        dev: &DeviceHandle,
        session: &str,
        missing: Vec<String>,
        fix: Vec<String>,
    ) -> Result<()> {
        let reason = format!("missing on the computer: {}", missing.join(", "));
        tracing::warn!(plugin = self.info.id, "{reason}");
        dev.emit(
            self.info.id,
            "error",
            json!({ "error": reason, "missing": missing, "fix": fix }),
        );
        let mut p = self.msg(
            StreamAction::Stop,
            session,
            None,
            StreamParams::default(),
            Some(reason),
        );
        p.body.insert("missing".into(), json!(missing));
        p.body.insert("fix".into(), json!(fix));
        dev.send_if_supported(p).await
    }

    fn stop_local(&self, device_id: &str) -> bool {
        match self.sessions.lock().unwrap().remove(device_id) {
            Some(s) => {
                if let Some(t) = s.task {
                    t.abort();
                }
                tracing::info!(
                    plugin = self.info.id,
                    device = device_id,
                    secs = s.started.elapsed().as_secs(),
                    "stream stopped"
                );
                if let Some(c) = self.core.get() {
                    c.emit_plugin(
                        Some(device_id),
                        self.info.id,
                        "stopped",
                        json!({ "session": s.id }),
                    );
                }
                true
            }
            None => false,
        }
    }

    fn session_matches(&self, device_id: &str, session: &str) -> bool {
        self.sessions
            .lock()
            .unwrap()
            .get(device_id)
            .is_some_and(|s| s.id == session)
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
                let failed = r.is_err();
                let reason = r.err().map(|e| e.to_string());
                let mut p = me.msg(
                    StreamAction::Stop,
                    &sid,
                    None,
                    StreamParams::default(),
                    reason,
                );
                if failed {
                    let (missing, fix) = Self::needs(me.kind).await;
                    if !missing.is_empty() {
                        p.body.insert("missing".into(), json!(missing));
                        p.body.insert("fix".into(), json!(fix));
                    }
                }
                let _ = dev_c.send_if_supported(p).await;
            }
        });
        self.sessions.lock().unwrap().insert(
            dev.id().to_owned(),
            Session {
                id: session.clone(),
                task: Some(task.abort_handle()),
                started: Instant::now(),
            },
        );
        dev.emit(self.info.id, "started", json!({ "session": session }));
    }

    // --- consumer side (phone produces) -------------------------------------

    async fn consume(
        kind: Kind,
        mut ch: SecureChannel,
        params: StreamParams,
        title: String,
    ) -> Result<()> {
        match kind {
            Kind::Microphone => {
                if params.codec != "pcm_s16le"
                    || params.sample_rate != 48_000
                    || params.channels != 1
                {
                    bail!(
                        "unsupported microphone format {} {} Hz {} ch",
                        params.codec,
                        params.sample_rate,
                        params.channels
                    );
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
                // Linux: the camera that stays listed while Konnekta runs.
                if konnekta_media::webcam::available() {
                    let mut feed = konnekta_media::webcam::webcam().live().await?;
                    tracing::info!("phone camera on Konnekta Camera");
                    while let Some(f) = read_frame(&mut ch).await? {
                        if f.kind == FrameKind::Video {
                            feed.write(&f.data).await?;
                        }
                    }
                    return Ok(());
                }
                let mut cam =
                    VirtualCamera::create(params.width, params.height, params.fps.max(15)).await?;
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
            Kind::PhoneAudio => {
                if params.codec != "pcm_s16le"
                    || params.sample_rate != 48_000
                    || !(1..=2).contains(&params.channels)
                {
                    bail!(
                        "unsupported phone audio format {} {} Hz {} ch",
                        params.codec,
                        params.sample_rate,
                        params.channels
                    );
                }
                let mut out = Speakers::create(params.channels).await?;
                while let Some(f) = read_frame(&mut ch).await? {
                    if f.kind == FrameKind::Audio {
                        out.write(&f.data).await?;
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    // --- producer side (computer produces) ----------------------------------

    /// The computer's screen: one frame per encoded picture, sent the moment
    /// the encoder writes it, like Flux. Nothing is dropped: when the network
    /// is slower, sending waits, the capture's pipe fills and the capture
    /// tool skips pictures by itself, so what arrives is always current and
    /// clean. Quality is fixed for the session (constant-quality encoding
    /// keeps a still screen almost free).
    async fn produce_desktop(ch: SecureChannel, params: StreamParams, extend: bool) -> Result<()> {
        let sender = FrameSender::new(ch, 2);
        let level = DEFAULT_LADDER
            .iter()
            .rposition(|q| params.height == 0 || q.height <= params.height)
            .unwrap_or(0)
            .min(3);
        let mut q = DEFAULT_LADDER[level];
        // More pictures per second: each change reaches the phone sooner.
        if params.fps >= 50 {
            q.fps = 60;
        }
        // Second screen: a monitor of the phone's own size, in the shape the
        // phone asks for (landscape, or portrait when held upright), captured
        // whole so text stays sharp on the phone.
        let monitor = if extend {
            let (w, h) = if params.width == 0 || params.height == 0 {
                (1920, 1080)
            } else {
                (params.width, params.height)
            };
            let fps = if params.fps >= 50 { 60 } else { 30 };
            let m = tokio::task::spawn_blocking(move || {
                konnekta_platform::display::VirtualMonitor::create(w, h, fps)
            })
            .await??;
            q.height = h;
            q.fps = fps;
            Some(m)
        } else {
            None
        };
        // The monitor shown: touches land on it.
        let output = match &monitor {
            Some(m) => Some(m.name.clone()),
            None => {
                let name = tokio::task::spawn_blocking(konnekta_platform::display::focused_monitor)
                    .await
                    .ok()
                    .flatten();
                konnekta_platform::display::set_captured_monitor(name.clone());
                name
            }
        };
        sender.send(Frame::control(&json!({ "quality": q }))).await;
        let (prog, args) = screen_capture_command(q, output.as_deref())?;
        tracing::info!(program = %prog, height = q.height, fps = q.fps, "screen capture");
        let mut proc = ProcessPipeline::spawn(&prog, &args, false, true)?;
        let out = proc.stdout.take().context("capture has no output")?;
        let mut flv = FlvReader::new(out);
        let mut config: Option<bytes::Bytes> = None;
        let config_frame = |c: &bytes::Bytes| {
            let mut f = Frame::new(FrameKind::Video, now_us(), c.clone()).keyframe();
            f.flags |= FLAG_CONFIG;
            f
        };
        loop {
            let ok = match flv.next().await? {
                None => bail!("screen capture stopped ({prog})"),
                Some(FlvVideo::Config(c)) => {
                    let ok = sender.send(config_frame(&c)).await;
                    config = Some(c);
                    ok
                }
                Some(FlvVideo::Picture { data, key }) => {
                    // SPS/PPS again before each keyframe, for a decoder that
                    // (re)starts there.
                    let mut ok = true;
                    if key && let Some(c) = &config {
                        ok = sender.send(config_frame(c)).await;
                    }
                    let mut f = Frame::new(FrameKind::Video, now_us(), data);
                    if key {
                        f = f.keyframe();
                    }
                    ok && sender.send(f).await
                }
            };
            if !ok || sender.is_closed() {
                return Ok(());
            }
        }
    }

    /// What the computer plays, or (`screen`) a new output of its own for
    /// the second screen. Real time: a short queue, and audio that cannot
    /// leave at once is dropped rather than delayed.
    async fn produce_system_audio(ch: SecureChannel, screen: bool) -> Result<()> {
        let sender = FrameSender::new(ch, 6);
        let mut cap = SystemAudioCapture::start_from(screen).await?;
        while let Some(chunk) = cap.rx.recv().await {
            sender.push(Frame::new(FrameKind::Audio, now_us(), chunk));
            if sender.is_closed() {
                break;
            }
        }
        Ok(())
    }

    async fn start_producer(
        &self,
        dev: &DeviceHandle,
        session: String,
        params: StreamParams,
    ) -> Result<()> {
        self.stop_local(dev.id());
        let (port, accept) = dev.listen_channel().await?;
        let out_params = match self.kind {
            Kind::Desktop | Kind::Extend => StreamParams {
                codec: "h264".into(),
                ..params.clone()
            },
            _ => StreamParams {
                codec: "pcm_s16le".into(),
                sample_rate: 48_000,
                channels: 2,
                ..Default::default()
            },
        };
        dev.send(self.msg(StreamAction::Offer, &session, Some(port), out_params, None))
            .await?;
        if let Some(c) = self.core.get() {
            let what = match self.kind {
                Kind::Desktop => "screen",
                Kind::Extend => "second screen",
                _ => "audio",
            };
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
                Kind::Desktop => Self::produce_desktop(ch, params, false).await,
                Kind::Extend => Self::produce_desktop(ch, params, true).await,
                _ => {
                    let screen = params.source.as_deref() == Some("screen");
                    Self::produce_system_audio(ch, screen).await
                }
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
        // The webcam stays listed in video apps while Konnekta runs.
        if self.kind == Kind::Camera && core.keeps_virtual_camera() {
            konnekta_media::webcam::webcam().keep_open();
        }
        // Stop sessions whose permission was revoked.
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            while core.is_alive() {
                tick.tick().await;
                let ids: Vec<String> = self.sessions.lock().unwrap().keys().cloned().collect();
                for id in ids {
                    let ok = core
                        .device(&id)
                        .is_some_and(|d| d.has_permission(self.info.permission));
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
        // Bluetooth is far too slow for live audio and video: say no at once.
        if !dev.transport().carries_streams()
            && matches!(m.action, StreamAction::Request | StreamAction::Offer)
        {
            return dev
                .send_if_supported(self.msg(
                    StreamAction::Stop,
                    &m.session,
                    None,
                    StreamParams::default(),
                    Some(BLUETOOTH_REASON.into()),
                ))
                .await;
        }
        match (m.action, self.kind.produces_locally()) {
            (StreamAction::Request, true) => {
                let (missing, fix) = Self::needs(self.kind).await;
                if !missing.is_empty() {
                    return self.refuse_missing(dev, &m.session, missing, fix).await;
                }
                self.start_producer(dev, m.session, m.params).await?
            }
            (StreamAction::Offer, false) => {
                let port = m.port.context("offer without port")?;
                let (missing, fix) = Self::needs(self.kind).await;
                if !missing.is_empty() {
                    return self.refuse_missing(dev, &m.session, missing, fix).await;
                }
                self.stop_local(dev.id());
                let ch = dev.connect_channel(port).await?;
                let title = format!("{} — Konnekta", dev.name());
                self.run_session(
                    dev,
                    m.session,
                    Self::consume(self.kind, ch, m.params, title),
                );
            }
            (StreamAction::Stop, _) => {
                if self.session_matches(dev.id(), &m.session) {
                    self.stop_local(dev.id());
                }
            }
            (StreamAction::Keyframe, true) => {
                // The encoder emits SPS/PPS with every IDR (short GOP); nothing to do.
            }
            _ => {
                tracing::debug!(plugin = self.info.id, action = ?m.action, "ignored stream message")
            }
        }
        Ok(())
    }

    async fn handle_action(
        &self,
        dev: Option<&DeviceHandle>,
        action: &str,
        args: Value,
    ) -> Result<Value> {
        let d = crate::need_dev(dev)?;
        match action {
            "start" => {
                if self.kind.produces_locally() {
                    bail!("this stream is started from the phone");
                }
                if !d.transport().carries_streams() {
                    bail!(BLUETOOTH_REASON);
                }
                let params: StreamParams = crate::args(args).unwrap_or_default();
                let session = session_id();
                d.send(self.msg(StreamAction::Request, &session, None, params, None))
                    .await?;
                Ok(json!({ "session": session }))
            }
            "configure" => {
                let params: StreamParams = crate::args(args)?;
                let sid = self
                    .sessions
                    .lock()
                    .unwrap()
                    .get(d.id())
                    .map(|s| s.id.clone())
                    .context("no active session")?;
                d.send(self.msg(StreamAction::Configure, &sid, None, params, None))
                    .await?;
                Ok(Value::Null)
            }
            "stop" => {
                let sid = self
                    .sessions
                    .lock()
                    .unwrap()
                    .get(d.id())
                    .map(|s| s.id.clone());
                if let Some(sid) = sid {
                    self.stop_local(d.id());
                    d.send_if_supported(self.msg(
                        StreamAction::Stop,
                        &sid,
                        None,
                        StreamParams::default(),
                        Some("stopped by user".into()),
                    ))
                    .await?;
                }
                Ok(Value::Null)
            }
            "status" => {
                let s =
                    self.sessions.lock().unwrap().get(d.id()).map(
                        |s| json!({ "session": s.id, "seconds": s.started.elapsed().as_secs() }),
                    );
                Ok(s.unwrap_or(Value::Null))
            }
            _ => bail!("unknown action {action}"),
        }
    }
}
