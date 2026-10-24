//! The "Konnekta Camera" device on Linux (v4l2loopback), kept open for as
//! long as Konnekta runs.
//!
//! With `exclusive_caps=1` (needed by browsers) the device is listed as a
//! camera only while something writes to it. So Konnekta writes all the
//! time: a dark picture while no phone streams, the phone's video (decoded
//! and fitted to one fixed size) while one does. Video apps keep the camera
//! selected between calls, and a phone that changes lens, resolution or
//! frame rate never makes it disappear.

use crate::pipeline::ProcessPipeline;
use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::ChildStdin;
use tokio::sync::Mutex;

/// Size of the camera as apps see it; the phone's video is fitted inside.
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
const FRAME: usize = (WIDTH * HEIGHT * 3 / 2) as usize;
/// Pictures per second while no phone streams.
const IDLE_FPS: u64 = 10;
/// The idle picture comes back this long after the last phone picture.
const LIVE_HOLD_MS: u64 = 700;

pub struct Webcam {
    writer: Mutex<Option<ProcessPipeline>>,
    /// Wall clock ms of the last phone picture.
    live_at: AtomicU64,
    started: AtomicBool,
}

/// The process-wide camera.
pub fn webcam() -> Arc<Webcam> {
    static W: OnceLock<Arc<Webcam>> = OnceLock::new();
    W.get_or_init(|| {
        Arc::new(Webcam {
            writer: Mutex::new(None),
            live_at: AtomicU64::new(0),
            started: AtomicBool::new(false),
        })
    })
    .clone()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// A dark grey picture (YUV 4:2:0).
fn idle_picture() -> Vec<u8> {
    let mut f = vec![128u8; FRAME];
    f[..(WIDTH * HEIGHT) as usize].fill(24);
    f
}

/// Whether the camera can be kept on this computer (Linux with a
/// v4l2loopback device).
pub fn available() -> bool {
    cfg!(target_os = "linux") && device().is_some()
}

fn device() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        crate::virtual_devices::find_v4l2loopback()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

impl Webcam {
    /// Keep the device listed from now on (once per process): pictures go
    /// out even when no phone streams. Waits for the device if the module
    /// is not loaded yet.
    pub fn keep_open(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) || cfg!(not(target_os = "linux")) {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let idle = idle_picture();
            let mut tick = tokio::time::interval(Duration::from_millis(1000 / IDLE_FPS));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                if now_ms().saturating_sub(me.live_at.load(Ordering::Relaxed)) < LIVE_HOLD_MS {
                    continue;
                }
                if let Err(e) = me.write(&idle).await {
                    tracing::debug!("webcam idle: {e:#}");
                    // No device (module not loaded) or ffmpeg missing: retry later.
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        });
    }

    /// Write one picture of the fixed size, opening the device if needed.
    async fn write(&self, picture: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().await;
        if w.is_none() {
            let dev = device().context("no v4l2loopback device")?;
            let size = format!("{WIDTH}x{HEIGHT}");
            let args: Vec<String> = [
                "-loglevel",
                "error",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "yuv420p",
                "-s",
                &size,
                "-r",
                "30",
                "-i",
                "-",
                // Each picture out as it comes, never repeated or dropped.
                "-fps_mode",
                "passthrough",
                "-f",
                "v4l2",
                "-pix_fmt",
                "yuv420p",
                &dev,
            ]
            .into_iter()
            .map(String::from)
            .collect();
            *w = Some(ProcessPipeline::spawn("ffmpeg", &args, true, false)?);
            tracing::info!(device = %dev, "Konnekta Camera on");
        }
        let stdin = w
            .as_mut()
            .and_then(|p| p.stdin.as_mut())
            .context("camera closed")?;
        if let Err(e) = stdin.write_all(picture).await {
            *w = None;
            return Err(e.into());
        }
        Ok(())
    }

    /// Start showing a phone's H.264 video. Pictures flow until the returned
    /// feed is dropped; then the idle picture comes back.
    pub async fn live(self: &Arc<Self>) -> Result<LiveFeed> {
        self.keep_open();
        let filter = format!(
            "scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=decrease,\
             pad={WIDTH}:{HEIGHT}:(ow-iw)/2:(oh-ih)/2:color=black,format=yuv420p"
        );
        let args: Vec<String> = [
            "-loglevel",
            "error",
            // Start at once and never queue.
            "-probesize",
            "32",
            "-analyzeduration",
            "0",
            "-fflags",
            "nobuffer",
            "-flags",
            "low_delay",
            "-f",
            "h264",
            "-i",
            "-",
            "-vf",
            &filter,
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv420p",
            "-",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let mut decoder = ProcessPipeline::spawn("ffmpeg", &args, true, true)?;
        let stdin = decoder.stdin.take().context("decoder has no input")?;
        let mut out = decoder.stdout.take().context("decoder has no output")?;
        let me = self.clone();
        let task = tokio::spawn(async move {
            let _decoder = decoder;
            let mut pic = vec![0u8; FRAME];
            while out.read_exact(&mut pic).await.is_ok() {
                me.live_at.store(now_ms(), Ordering::Relaxed);
                if let Err(e) = me.write(&pic).await {
                    tracing::warn!("webcam: {e:#}");
                }
            }
        });
        Ok(LiveFeed {
            stdin,
            task: task.abort_handle(),
        })
    }
}

/// A phone's video going to the camera.
pub struct LiveFeed {
    stdin: ChildStdin,
    task: tokio::task::AbortHandle,
}

impl LiveFeed {
    /// H.264 Annex B as received.
    pub async fn write(&mut self, h264: &[u8]) -> Result<()> {
        self.stdin.write_all(h264).await?;
        Ok(())
    }
}

impl Drop for LiveFeed {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_picture_has_the_camera_size() {
        let p = idle_picture();
        assert_eq!(p.len(), 1280 * 720 * 3 / 2);
        assert_eq!(p[0], 24);
        assert_eq!(p[FRAME - 1], 128);
    }
}
