//! Binary frame format on stream channels:
//!
//! ```text
//! u8  kind   (1 audio, 2 video, 3 control/json)
//! u8  flags  (bit0 keyframe, bit1 codec config)
//! u32 seq    (big endian, wraps)
//! u64 pts_us (capture timestamp, microseconds)
//! u32 len    (payload length)
//! [len] payload
//! ```

use anyhow::{Result, bail};
use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_FRAME: usize = 8 * 1024 * 1024;
pub const FLAG_KEYFRAME: u8 = 1;
pub const FLAG_CONFIG: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    Audio = 1,
    Video = 2,
    Control = 3,
}

impl FrameKind {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Audio,
            2 => Self::Video,
            3 => Self::Control,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub kind: FrameKind,
    pub flags: u8,
    pub seq: u32,
    pub pts_us: u64,
    pub data: Bytes,
}

impl Frame {
    pub fn new(kind: FrameKind, pts_us: u64, data: impl Into<Bytes>) -> Self {
        Self { kind, flags: 0, seq: 0, pts_us, data: data.into() }
    }

    pub fn keyframe(mut self) -> Self {
        self.flags |= FLAG_KEYFRAME;
        self
    }

    pub fn is_keyframe(&self) -> bool {
        self.flags & (FLAG_KEYFRAME | FLAG_CONFIG) != 0
    }

    pub fn control(json: &serde_json::Value) -> Self {
        Self::new(FrameKind::Control, 0, serde_json::to_vec(json).unwrap_or_default())
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, f: &Frame) -> Result<()> {
    if f.data.len() > MAX_FRAME {
        bail!("frame too large");
    }
    let mut hdr = [0u8; 18];
    hdr[0] = f.kind as u8;
    hdr[1] = f.flags;
    hdr[2..6].copy_from_slice(&f.seq.to_be_bytes());
    hdr[6..14].copy_from_slice(&f.pts_us.to_be_bytes());
    hdr[14..18].copy_from_slice(&(f.data.len() as u32).to_be_bytes());
    w.write_all(&hdr).await?;
    w.write_all(&f.data).await?;
    Ok(())
}

/// Returns `None` on clean EOF.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Frame>> {
    let mut hdr = [0u8; 18];
    match r.read_exact(&mut hdr).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let Some(kind) = FrameKind::from_u8(hdr[0]) else { bail!("unknown frame kind {}", hdr[0]) };
    let len = u32::from_be_bytes(hdr[14..18].try_into().unwrap()) as usize;
    if len > MAX_FRAME {
        bail!("frame too large ({len} bytes)");
    }
    let mut data = vec![0u8; len];
    r.read_exact(&mut data).await?;
    Ok(Some(Frame {
        kind,
        flags: hdr[1],
        seq: u32::from_be_bytes(hdr[2..6].try_into().unwrap()),
        pts_us: u64::from_be_bytes(hdr[6..14].try_into().unwrap()),
        data: data.into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(1 << 16);
        let f = Frame { kind: FrameKind::Video, flags: FLAG_KEYFRAME, seq: 7, pts_us: 123456, data: Bytes::from_static(b"nal") };
        write_frame(&mut a, &f).await.unwrap();
        drop(a);
        assert_eq!(read_frame(&mut b).await.unwrap().unwrap(), f);
        assert!(read_frame(&mut b).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rejects_huge_and_unknown() {
        let (mut a, mut b) = tokio::io::duplex(64);
        let mut hdr = [0u8; 18];
        hdr[0] = 2;
        hdr[14..18].copy_from_slice(&u32::MAX.to_be_bytes());
        a.write_all(&hdr).await.unwrap();
        assert!(read_frame(&mut b).await.is_err());
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&[9u8; 18]).await.unwrap();
        assert!(read_frame(&mut b).await.is_err());
    }
}
