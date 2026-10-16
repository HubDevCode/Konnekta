//! Virtual input devices exposing phone streams to desktop applications
//! (Discord, Zoom, OBS...).
//!
//! | Feature        | Linux                                   | Windows                          | macOS                          |
//! |----------------|-----------------------------------------|----------------------------------|--------------------------------|
//! | Microphone     | PipeWire/Pulse `module-pipe-source`     | VB-Audio Cable (cpal)            | BlackHole 2ch (cpal)           |
//! | Webcam         | v4l2loopback + ffmpeg                   | ffmpeg -> OBS (Virtual Camera)   | ffmpeg -> OBS (Virtual Camera) |
//! | System audio   | `parec` on the default monitor          | WASAPI loopback (cpal)           | BlackHole input (cpal)         |

use anyhow::{Context, Result, bail};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::AsyncWriteExt;

pub const SAMPLE_RATE: u32 = 48_000;

/// Sink for phone microphone PCM (s16le, 48 kHz, mono).
pub struct VirtualMic {
    inner: MicImpl,
}

enum MicImpl {
    #[cfg(target_os = "linux")]
    Pipe {
        fifo: tokio::fs::File,
        module: String,
        path: std::path::PathBuf,
    },
    #[cfg(any(windows, target_os = "macos"))]
    Cpal(cpal_out::CpalOutput),
    #[allow(dead_code)]
    None,
}

impl VirtualMic {
    pub async fn create() -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let dir = std::env::var_os("XDG_RUNTIME_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let path = dir.join(format!("konnekta-mic-{}", std::process::id()));
            let _ = std::fs::remove_file(&path);
            let st = tokio::process::Command::new("mkfifo")
                .arg("-m")
                .arg("600")
                .arg(&path)
                .status()
                .await?;
            if !st.success() {
                bail!("mkfifo failed");
            }
            let out = tokio::process::Command::new("pactl")
                .args([
                    "load-module",
                    "module-pipe-source",
                    "source_name=konnekta_mic",
                    "source_properties=device.description=Konnekta_Microphone",
                    &format!("file={}", path.display()),
                    "format=s16le",
                    &format!("rate={SAMPLE_RATE}"),
                    "channels=1",
                ])
                .output()
                .await
                .context("pactl not found (install pipewire-pulse or pulseaudio-utils)")?;
            if !out.status.success() {
                let _ = std::fs::remove_file(&path);
                bail!(
                    "cannot create virtual microphone: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            let module = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let p = path.clone();
            let fifo = tokio::task::spawn_blocking(move || {
                std::fs::OpenOptions::new().write(true).open(p)
            })
            .await??;
            tracing::info!("virtual microphone 'Konnekta Microphone' ready");
            return Ok(Self {
                inner: MicImpl::Pipe {
                    fifo: tokio::fs::File::from_std(fifo),
                    module,
                    path,
                },
            });
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            #[cfg(windows)]
            const NAMES: &[&str] = &["CABLE Input", "VB-Audio"];
            #[cfg(target_os = "macos")]
            const NAMES: &[&str] = &["BlackHole"];
            return Ok(Self {
                inner: MicImpl::Cpal(cpal_out::CpalOutput::open(NAMES, 1)?),
            });
        }
        #[allow(unreachable_code)]
        {
            bail!("virtual microphone not supported on this platform")
        }
    }

    /// Write PCM s16le mono samples.
    pub async fn write(&mut self, pcm: &[u8]) -> Result<()> {
        match &mut self.inner {
            #[cfg(target_os = "linux")]
            MicImpl::Pipe { fifo, .. } => Ok(fifo.write_all(pcm).await?),
            #[cfg(any(windows, target_os = "macos"))]
            MicImpl::Cpal(o) => {
                o.push(&crate::jitter::bytes_to_i16(pcm));
                Ok(())
            }
            MicImpl::None => {
                let _ = pcm;
                Ok(())
            }
        }
    }
}

impl Drop for VirtualMic {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let MicImpl::Pipe { module, path, .. } = &self.inner {
            let _ = std::process::Command::new("pactl")
                .args(["unload-module", module])
                .status();
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Sink for the phone camera (H.264 Annex B).
pub struct VirtualCamera {
    pipeline: crate::pipeline::ProcessPipeline,
    pub description: String,
}

impl VirtualCamera {
    pub async fn create(width: u32, height: u32, fps: u32) -> Result<Self> {
        let _ = (width, height);
        let mut args: Vec<String> = [
            "-loglevel",
            "error",
            "-fflags",
            "nobuffer",
            "-flags",
            "low_delay",
            "-f",
            "h264",
            "-i",
            "-",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        #[cfg(target_os = "linux")]
        let description = {
            let dev = find_v4l2loopback().context(
                "no v4l2loopback device found. Install v4l2loopback and run: \
                 sudo modprobe v4l2loopback card_label=\"Konnekta Camera\" exclusive_caps=1",
            )?;
            args.extend(
                [
                    "-r",
                    &fps.to_string(),
                    "-pix_fmt",
                    "yuv420p",
                    "-f",
                    "v4l2",
                    &dev,
                ]
                .into_iter()
                .map(String::from),
            );
            format!("v4l2 device {dev}")
        };
        #[cfg(not(target_os = "linux"))]
        let description = {
            // Native DirectShow / CoreMediaIO camera extensions require signed
            // drivers; until then we feed OBS (Media Source) whose Virtual
            // Camera is visible to every app.
            let url = format!("udp://127.0.0.1:{}?pkt_size=1316", OBS_UDP_PORT);
            args.extend(
                ["-c:v", "copy", "-r", &fps.to_string(), "-f", "mpegts", &url]
                    .into_iter()
                    .map(String::from),
            );
            format!(
                "OBS Media Source at udp://127.0.0.1:{OBS_UDP_PORT} (then start OBS Virtual Camera)"
            )
        };
        let pipeline = crate::pipeline::ProcessPipeline::spawn("ffmpeg", &args, true, false)?;
        Ok(Self {
            pipeline,
            description,
        })
    }

    pub async fn write(&mut self, h264: &[u8]) -> Result<()> {
        let stdin = self
            .pipeline
            .stdin
            .as_mut()
            .context("camera pipeline closed")?;
        stdin.write_all(h264).await?;
        Ok(())
    }
}

pub const OBS_UDP_PORT: u16 = 19_716;

#[cfg(target_os = "linux")]
pub fn find_v4l2loopback() -> Option<String> {
    let mut fallback = None;
    for entry in std::fs::read_dir("/sys/class/video4linux").ok()?.flatten() {
        let name = std::fs::read_to_string(entry.path().join("name")).unwrap_or_default();
        let dev = format!("/dev/{}", entry.file_name().to_string_lossy());
        if name.contains("Konnekta") {
            return Some(dev);
        }
        if name.contains("Dummy") || name.contains("loopback") {
            fallback = Some(dev);
        }
    }
    fallback
}

/// Window showing the phone screen on the desktop (ffplay).
pub fn screen_viewer(title: &str) -> Result<crate::pipeline::ProcessPipeline> {
    let args: Vec<String> = [
        "-loglevel",
        "error",
        "-fflags",
        "nobuffer",
        "-flags",
        "low_delay",
        "-framedrop",
        "-window_title",
        title,
        "-f",
        "h264",
        "-i",
        "-",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    crate::pipeline::ProcessPipeline::spawn("ffplay", &args, true, false)
}

/// Captures system audio as s16le 48 kHz stereo chunks.
pub struct SystemAudioCapture {
    pub rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    stop: Arc<AtomicBool>,
    _proc: Option<crate::pipeline::ProcessPipeline>,
}

impl SystemAudioCapture {
    pub async fn start() -> Result<Self> {
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "linux")]
        {
            use tokio::io::AsyncReadExt;
            let (prog, args) = crate::pipeline::system_audio_command()?;
            let mut p = crate::pipeline::ProcessPipeline::spawn(&prog, &args, false, true)?;
            let mut out = p.stdout.take().context("no stdout")?;
            let s = stop.clone();
            tokio::spawn(async move {
                // 10 ms of stereo s16 @ 48 kHz = 1920 bytes.
                let mut buf = vec![0u8; 1920];
                while !s.load(Ordering::Relaxed) {
                    if out.read_exact(&mut buf).await.is_err() {
                        break;
                    }
                    if tx.try_send(buf.clone()).is_err() && tx.is_closed() {
                        break;
                    }
                }
            });
            return Ok(Self {
                rx,
                stop,
                _proc: Some(p),
            });
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            cpal_out::start_loopback(tx, stop.clone())?;
            return Ok(Self {
                rx,
                stop,
                _proc: None,
            });
        }
        #[allow(unreachable_code)]
        {
            let _ = tx;
            bail!("system audio capture not supported on this platform")
        }
    }
}

impl Drop for SystemAudioCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(any(windows, target_os = "macos"))]
mod cpal_out {
    use super::*;
    use crate::jitter::{JitterBuffer, i16_to_bytes};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::Mutex;

    /// Plays mono 48 kHz PCM on a named output device (virtual cable).
    pub struct CpalOutput {
        jitter: Arc<Mutex<JitterBuffer>>,
        stop: Arc<AtomicBool>,
    }

    impl CpalOutput {
        pub fn open(names: &'static [&'static str], channels: u8) -> Result<Self> {
            let jitter = Arc::new(Mutex::new(JitterBuffer::new(
                SAMPLE_RATE,
                channels,
                60,
                250,
            )));
            let stop = Arc::new(AtomicBool::new(false));
            let (j, s) = (jitter.clone(), stop.clone());
            let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
            std::thread::spawn(move || {
                let r = (|| -> Result<cpal::Stream> {
                    let host = cpal::default_host();
                    let dev = host
                        .output_devices()?
                        .find(|d| d.name().map(|n| names.iter().any(|x| n.contains(x))).unwrap_or(false))
                        .with_context(|| {
                            format!("virtual audio device not found ({}). Install it to expose the phone microphone.", names.join(" / "))
                        })?;
                    let cfg = dev.default_output_config()?;
                    let out_ch = cfg.channels() as usize;
                    let out_rate = cfg.sample_rate().0;
                    let stream_cfg: cpal::StreamConfig = cfg.into();
                    let ratio = SAMPLE_RATE as f64 / out_rate as f64;
                    let mut scratch: Vec<i16> = Vec::new();
                    let stream = dev.build_output_stream(
                        &stream_cfg,
                        move |data: &mut [f32], _| {
                            let frames = data.len() / out_ch;
                            let need = ((frames as f64) * ratio).ceil() as usize + 1;
                            scratch.resize(need, 0);
                            j.lock().unwrap().pop(&mut scratch);
                            for f in 0..frames {
                                let pos = f as f64 * ratio;
                                let i = pos as usize;
                                let frac = pos - i as f64;
                                let a = scratch[i.min(need - 1)] as f64;
                                let b = scratch[(i + 1).min(need - 1)] as f64;
                                let v = ((a + (b - a) * frac) / 32768.0) as f32;
                                for c in 0..out_ch {
                                    data[f * out_ch + c] = v;
                                }
                            }
                        },
                        |e| tracing::warn!("audio output error: {e}"),
                        None,
                    )?;
                    stream.play()?;
                    Ok(stream)
                })();
                match r {
                    Ok(stream) => {
                        let _ = ready_tx.send(Ok(()));
                        while !s.load(Ordering::Relaxed) {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            });
            ready_rx.recv().context("audio thread died")??;
            Ok(Self { jitter, stop })
        }

        pub fn push(&self, samples: &[i16]) {
            self.jitter.lock().unwrap().push(samples);
        }
    }

    impl Drop for CpalOutput {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Loopback capture of the default output (Windows WASAPI) or of the
    /// BlackHole input (macOS), converted to s16le 48 kHz stereo.
    pub fn start_loopback(
        tx: tokio::sync::mpsc::Sender<Vec<u8>>,
        stop: Arc<AtomicBool>,
    ) -> Result<()> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        std::thread::spawn(move || {
            let r = (|| -> Result<cpal::Stream> {
                let host = cpal::default_host();
                #[cfg(windows)]
                let dev = host.default_output_device().context("no output device")?;
                #[cfg(target_os = "macos")]
                let dev = host
                    .input_devices()?
                    .find(|d| d.name().map(|n| n.contains("BlackHole")).unwrap_or(false))
                    .context(
                        "install BlackHole and route system output to it (Multi-Output Device)",
                    )?;
                #[cfg(windows)]
                let cfg = dev.default_output_config()?;
                #[cfg(target_os = "macos")]
                let cfg = dev.default_input_config()?;
                let ch = cfg.channels() as usize;
                let rate = cfg.sample_rate().0;
                let stream_cfg: cpal::StreamConfig = cfg.into();
                let ratio = rate as f64 / SAMPLE_RATE as f64;
                let mut acc: Vec<i16> = Vec::with_capacity(1920);
                let mut phase = 0.0f64;
                let stream = dev.build_input_stream(
                    &stream_cfg,
                    move |data: &[f32], _| {
                        let frames = data.len() / ch;
                        while phase < frames as f64 {
                            let i = phase as usize;
                            let l = data[i * ch];
                            let r = if ch > 1 { data[i * ch + 1] } else { l };
                            acc.push((l.clamp(-1.0, 1.0) * 32767.0) as i16);
                            acc.push((r.clamp(-1.0, 1.0) * 32767.0) as i16);
                            if acc.len() >= 960 {
                                let _ = tx.try_send(i16_to_bytes(&acc));
                                acc.clear();
                            }
                            phase += ratio;
                        }
                        phase -= frames as f64;
                    },
                    |e| tracing::warn!("audio capture error: {e}"),
                    None,
                )?;
                stream.play()?;
                Ok(stream)
            })();
            match r {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        ready_rx.recv().context("audio thread died")?
    }
}
