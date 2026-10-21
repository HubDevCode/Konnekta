//! Minimal FLV reader for the screen capture.
//!
//! The capture tools (ffmpeg, wf-recorder, gpu-screen-recorder) write FLV
//! to a pipe: one tag per encoded picture. Reading tags instead of an Annex B
//! byte stream means every picture is complete the moment its tag arrives,
//! so nothing waits for the next picture to find where this one ends.
//!
//! Video tags carry AVC in "length-prefixed" form; they are returned as
//! Annex B (start codes), which is what the phone's decoder takes.

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncReadExt};

/// Largest tag accepted (a 4K keyframe at high quality fits easily).
const MAX_TAG: usize = 16 * 1024 * 1024;
const START: [u8; 4] = [0, 0, 0, 1];

#[derive(Debug, Clone, PartialEq)]
pub enum FlvVideo {
    /// SPS and PPS (Annex B), from the AVC sequence header.
    Config(Bytes),
    /// One picture (Annex B); `key` for an IDR.
    Picture { data: Bytes, key: bool },
}

pub struct FlvReader<R> {
    r: R,
    header_read: bool,
    /// Bytes of each NAL length in the pictures (from the sequence header).
    length_size: usize,
}

impl<R: AsyncRead + Unpin> FlvReader<R> {
    pub fn new(r: R) -> Self {
        Self {
            r,
            header_read: false,
            length_size: 4,
        }
    }

    /// Next video item; `None` at the end of the stream.
    pub async fn next(&mut self) -> Result<Option<FlvVideo>> {
        if !self.header_read {
            let mut h = [0u8; 9];
            if !read_or_eof(&mut self.r, &mut h).await? {
                return Ok(None);
            }
            if &h[..3] != b"FLV" {
                bail!("the capture does not write FLV");
            }
            let skip = u32::from_be_bytes([h[5], h[6], h[7], h[8]]) as usize;
            // Rest of the header, then PreviousTagSize0.
            let mut rest = vec![0u8; skip.saturating_sub(9) + 4];
            self.r.read_exact(&mut rest).await?;
            self.header_read = true;
        }
        loop {
            let mut th = [0u8; 11];
            if !read_or_eof(&mut self.r, &mut th).await? {
                return Ok(None);
            }
            let kind = th[0] & 0x1f;
            let size = u32::from_be_bytes([0, th[1], th[2], th[3]]) as usize;
            if size > MAX_TAG {
                bail!("FLV tag of {size} bytes");
            }
            let mut body = vec![0u8; size + 4]; // + PreviousTagSize
            self.r.read_exact(&mut body).await?;
            body.truncate(size);
            if kind != 9 {
                continue; // audio or script data
            }
            if let Some(v) = self.video(&body)? {
                return Ok(Some(v));
            }
        }
    }

    fn video(&mut self, b: &[u8]) -> Result<Option<FlvVideo>> {
        if b.len() < 5 || b[0] & 0x0f != 7 {
            return Ok(None); // not AVC
        }
        let key = b[0] >> 4 == 1;
        let p = &b[5..];
        match b[1] {
            0 => {
                let (config, length_size) = avc_config(p)?;
                self.length_size = length_size;
                Ok(Some(FlvVideo::Config(config)))
            }
            1 => {
                let data = annex_b(p, self.length_size)?;
                if data.is_empty() {
                    return Ok(None);
                }
                Ok(Some(FlvVideo::Picture { data, key }))
            }
            _ => Ok(None),
        }
    }
}

async fn read_or_eof<R: AsyncRead + Unpin>(r: &mut R, buf: &mut [u8]) -> Result<bool> {
    match r.read_exact(buf).await {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// SPS + PPS from an AVCDecoderConfigurationRecord, and the NAL length size.
fn avc_config(p: &[u8]) -> Result<(Bytes, usize)> {
    let short = || anyhow::anyhow!("short AVC sequence header");
    let length_size = (*p.get(4).ok_or_else(short)? & 3) as usize + 1;
    let mut out = Vec::new();
    let mut i = 5;
    for sets in 0..2 {
        let n = *p.get(i).ok_or_else(short)? as usize & if sets == 0 { 0x1f } else { 0xff };
        i += 1;
        for _ in 0..n {
            let len = u16::from_be_bytes([
                *p.get(i).ok_or_else(short)?,
                *p.get(i + 1).ok_or_else(short)?,
            ]) as usize;
            i += 2;
            let nal = p.get(i..i + len).context("short parameter set")?;
            out.extend_from_slice(&START);
            out.extend_from_slice(nal);
            i += len;
        }
    }
    Ok((Bytes::from(out), length_size))
}

/// Length-prefixed NAL units to Annex B.
fn annex_b(mut p: &[u8], length_size: usize) -> Result<Bytes> {
    let mut out = Vec::with_capacity(p.len() + 16);
    while !p.is_empty() {
        if p.len() < length_size {
            bail!("truncated NAL length");
        }
        let len = p[..length_size]
            .iter()
            .fold(0usize, |a, &b| (a << 8) | b as usize);
        p = &p[length_size..];
        let nal = p.get(..len).context("truncated NAL")?;
        // Access unit delimiters carry nothing the decoder needs.
        if nal.first().map(|h| h & 0x1f) != Some(9) {
            out.extend_from_slice(&START);
            out.extend_from_slice(nal);
        }
        p = &p[len..];
    }
    Ok(Bytes::from(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut t = vec![kind];
        t.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        t.extend_from_slice(&[0; 7]); // timestamp, stream id
        t.extend_from_slice(body);
        t.extend_from_slice(&((body.len() + 11) as u32).to_be_bytes());
        t
    }

    #[tokio::test]
    async fn reads_config_and_pictures() {
        let mut s = b"FLV\x01\x01\x00\x00\x00\x09".to_vec();
        s.extend_from_slice(&[0, 0, 0, 0]);
        s.extend(tag(18, b"onMetaData")); // script tag, skipped
        // Sequence header: 1 SPS (67 aa), 1 PPS (68 bb), 4-byte lengths.
        s.extend(tag(
            9,
            &[
                0x17, 0, 0, 0, 0, 1, 0x64, 0, 0x1f, 0xff, 0xe1, 0, 2, 0x67, 0xaa, 1, 0, 2, 0x68,
                0xbb,
            ],
        ));
        // Keyframe: AUD (skipped) + IDR slice.
        s.extend(tag(
            9,
            &[
                0x17, 1, 0, 0, 0, 0, 0, 0, 2, 0x09, 0xf0, 0, 0, 0, 3, 0x65, 1, 2,
            ],
        ));
        // Inter frame.
        s.extend(tag(9, &[0x27, 1, 0, 0, 0, 0, 0, 0, 2, 0x41, 7]));
        let mut r = FlvReader::new(&s[..]);
        assert_eq!(
            r.next().await.unwrap(),
            Some(FlvVideo::Config(Bytes::from_static(&[
                0, 0, 0, 1, 0x67, 0xaa, 0, 0, 0, 1, 0x68, 0xbb
            ])))
        );
        assert_eq!(
            r.next().await.unwrap(),
            Some(FlvVideo::Picture {
                data: Bytes::from_static(&[0, 0, 0, 1, 0x65, 1, 2]),
                key: true
            })
        );
        assert_eq!(
            r.next().await.unwrap(),
            Some(FlvVideo::Picture {
                data: Bytes::from_static(&[0, 0, 0, 1, 0x41, 7]),
                key: false
            })
        );
        assert_eq!(r.next().await.unwrap(), None);
    }

    /// Captures the real screen for a few seconds and prints when each
    /// picture arrives: `cargo test -p konnekta-media -- --ignored live_capture --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_capture() {
        let q = crate::abr::DEFAULT_LADDER[3];
        let (prog, args) = crate::pipeline::screen_capture_command(q).unwrap();
        println!("{prog} {}", args.join(" "));
        let mut p = crate::pipeline::ProcessPipeline::spawn(&prog, &args, false, true).unwrap();
        let mut r = FlvReader::new(p.stdout.take().unwrap());
        let t0 = std::time::Instant::now();
        let (mut n, mut bytes) = (0, 0);
        let mut dump = Vec::new();
        while t0.elapsed().as_secs() < 4 {
            match tokio::time::timeout(std::time::Duration::from_secs(5), r.next()).await {
                Ok(Ok(Some(FlvVideo::Config(c)))) => {
                    dump.extend_from_slice(&c);
                    println!("{:>5} ms config {} B", t0.elapsed().as_millis(), c.len())
                }
                Ok(Ok(Some(FlvVideo::Picture { data, key }))) => {
                    dump.extend_from_slice(&data);
                    n += 1;
                    bytes += data.len();
                    if key || n <= 3 {
                        println!(
                            "{:>5} ms picture {} B key={key}",
                            t0.elapsed().as_millis(),
                            data.len()
                        );
                    }
                }
                other => {
                    println!("end: {other:?}");
                    break;
                }
            }
        }
        println!("{n} pictures, {} kbit/s", bytes * 8 / 4000);
        assert!(n > 0);
        if let Ok(path) = std::env::var("KONNEKTA_DUMP") {
            std::fs::write(path, &dump).unwrap();
        }
    }

    #[tokio::test]
    async fn rejects_other_formats() {
        let mut r = FlvReader::new(&[0u8, 0, 0, 1, 0x67, 0, 0, 0, 0, 0][..]);
        assert!(r.next().await.is_err());
    }
}
