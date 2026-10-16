//! Capture / encode pipelines built on external tools (ffmpeg, parec,
//! wf-recorder). They write H.264 Annex B or raw PCM to stdout, which we
//! split into frames and push to a [`FrameSender`](crate::FrameSender).

use crate::abr::Quality;
use anyhow::{Context, Result, bail};
use bytes::{Bytes, BytesMut};
use std::process::Stdio;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub struct ProcessPipeline {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
}

impl ProcessPipeline {
    pub fn spawn(program: &str, args: &[String], stdin: bool, stdout: bool) -> Result<Self> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(if stdin { Stdio::piped() } else { Stdio::null() })
            .stdout(if stdout {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("cannot start {program} (is it installed and in PATH?)"))?;
        Ok(Self {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            child,
        })
    }
}

/// Command line producing a low-latency H.264 elementary stream of the
/// primary screen on stdout.
pub fn screen_capture_command(q: Quality) -> Result<(String, Vec<String>)> {
    let fps = q.fps.to_string();
    let enc = |input: Vec<&str>| -> Vec<String> {
        let mut a: Vec<String> = vec![
            "-loglevel".into(),
            "error".into(),
            "-fflags".into(),
            "nobuffer".into(),
        ];
        a.extend(input.into_iter().map(String::from));
        a.extend(
            [
                "-vf",
                &format!("scale=-2:{}", q.height),
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-tune",
                "zerolatency",
                "-pix_fmt",
                "yuv420p",
                "-bf",
                "0",
                "-g",
                &(q.fps * 2).to_string(),
                "-b:v",
                &format!("{}k", q.bitrate_kbps),
                "-maxrate",
                &format!("{}k", q.bitrate_kbps),
                "-bufsize",
                &format!("{}k", q.bitrate_kbps / 2),
                "-bsf:v",
                "dump_extra",
                "-f",
                "h264",
                "-",
            ]
            .into_iter()
            .map(String::from),
        );
        a
    };
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            if which("wf-recorder") {
                return Ok((
                    "wf-recorder".into(),
                    [
                        "-y",
                        "-m",
                        "h264",
                        "-c",
                        "libx264",
                        "-r",
                        &fps,
                        "-p",
                        "preset=ultrafast",
                        "-p",
                        "tune=zerolatency",
                        "-p",
                        &format!("b={}k", q.bitrate_kbps),
                        "-f",
                        "/dev/stdout",
                    ]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                ));
            }
            if std::env::var_os("DISPLAY").is_none() {
                bail!(
                    "screen capture on this Wayland compositor needs wf-recorder (wlroots) \
                     or an X11 session; portal-based capture is not available yet"
                );
            }
        }
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        return Ok((
            "ffmpeg".into(),
            enc(vec!["-f", "x11grab", "-framerate", &fps, "-i", &display]),
        ));
    }
    #[cfg(windows)]
    {
        return Ok((
            "ffmpeg".into(),
            enc(vec!["-f", "gdigrab", "-framerate", &fps, "-i", "desktop"]),
        ));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok((
            "ffmpeg".into(),
            enc(vec![
                "-f",
                "avfoundation",
                "-framerate",
                &fps,
                "-capture_cursor",
                "1",
                "-i",
                "Capture screen 0:none",
            ]),
        ));
    }
    #[allow(unreachable_code)]
    {
        let _ = (enc, fps);
        bail!("screen capture not supported on this platform")
    }
}

/// System audio (what you hear) as s16le 48 kHz stereo on stdout.
pub fn system_audio_command() -> Result<(String, Vec<String>)> {
    #[cfg(target_os = "linux")]
    {
        return Ok((
            "parec".into(),
            [
                "--device=@DEFAULT_MONITOR@",
                "--format=s16le",
                "--rate=48000",
                "--channels=2",
                "--latency-msec=20",
                "--raw",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        ));
    }
    #[allow(unreachable_code)]
    {
        bail!("use virtual_devices::SystemAudioCapture (cpal loopback) on this platform")
    }
}

pub fn which(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|d| d.join(program).is_file() || d.join(format!("{program}.exe")).is_file())
}

/// Splits an H.264 Annex B byte stream into NAL units (start code included).
#[derive(Default)]
pub struct AnnexBSplitter {
    buf: BytesMut,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Nal {
    pub data: Bytes,
    pub nal_type: u8,
}

impl Nal {
    /// SPS / PPS / IDR: lets a decoder (re)start.
    pub fn is_key(&self) -> bool {
        matches!(self.nal_type, 5 | 7 | 8)
    }
    pub fn is_config(&self) -> bool {
        matches!(self.nal_type, 7 | 8)
    }
}

fn find_start(b: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 {
            if b[i + 2] == 1 {
                return Some((i, 3));
            }
            if i + 4 <= b.len() && b[i + 2] == 0 && b[i + 3] == 1 {
                return Some((i, 4));
            }
        }
        i += 1;
    }
    None
}

impl AnnexBSplitter {
    pub fn push(&mut self, data: &[u8]) -> Vec<Nal> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        while let Some((s, sl)) = find_start(&self.buf, 0) {
            if s > 0 {
                // Garbage before the first start code.
                let _ = self.buf.split_to(s);
                continue;
            }
            let Some((next, _)) = find_start(&self.buf, sl) else {
                break;
            };
            let nal = self.buf.split_to(next).freeze();
            let nal_type = nal.get(sl).map(|h| h & 0x1f).unwrap_or(0);
            out.push(Nal {
                data: nal,
                nal_type,
            });
        }
        out
    }

    /// Flush the last NAL at end of stream.
    pub fn finish(&mut self) -> Option<Nal> {
        let (_, sl) = find_start(&self.buf, 0)?;
        let nal = self.buf.split().freeze();
        let nal_type = nal.get(sl).map(|h| h & 0x1f).unwrap_or(0);
        Some(Nal {
            data: nal,
            nal_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_nals_across_chunks() {
        let stream = [
            &[0, 0, 0, 1, 0x67, 1, 2][..], // SPS
            &[0, 0, 0, 1, 0x68, 3][..],    // PPS
            &[0, 0, 1, 0x65, 9, 9, 9][..], // IDR (3-byte start code)
            &[0, 0, 0, 1, 0x41, 5][..],    // non-IDR slice
        ]
        .concat();
        let mut s = AnnexBSplitter::default();
        let mut nals = Vec::new();
        for chunk in stream.chunks(3) {
            nals.extend(s.push(chunk));
        }
        nals.extend(s.finish());
        let types: Vec<u8> = nals.iter().map(|n| n.nal_type).collect();
        assert_eq!(types, vec![7, 8, 5, 1]);
        assert!(nals[0].is_config() && nals[2].is_key() && !nals[3].is_key());
        assert_eq!(
            nals.iter().map(|n| n.data.len()).sum::<usize>(),
            stream.len()
        );
    }

    #[test]
    fn screen_command_has_low_latency_flags() {
        if let Ok((prog, args)) = screen_capture_command(crate::abr::DEFAULT_LADDER[2])
            && prog == "ffmpeg"
        {
            let joined = args.join(" ");
            assert!(joined.contains("zerolatency"));
            assert!(joined.contains("-bf 0"));
            assert!(joined.contains("2000k"));
        }
    }
}
