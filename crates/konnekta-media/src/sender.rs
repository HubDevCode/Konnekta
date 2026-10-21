//! Latency-first frame sender.
//!
//! Frames go through a small bounded queue to a writer task. If the queue is
//! full the frame is dropped instead of waiting (stale real-time data is
//! useless); for video, after a drop we also skip until the next keyframe so
//! the decoder never sees a broken reference chain.

use crate::frame::{Frame, FrameKind, write_frame};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};

#[derive(Debug, Default)]
pub struct SenderStats {
    pub sent: AtomicU64,
    pub dropped: AtomicU32,
    pub bytes: AtomicU64,
    /// Queue delay of the last written frame in microseconds.
    pub last_delay_us: AtomicU64,
}

impl SenderStats {
    pub fn take_dropped(&self) -> u32 {
        self.dropped.swap(0, Ordering::Relaxed)
    }

    pub fn queue_delay(&self) -> Duration {
        Duration::from_micros(self.last_delay_us.load(Ordering::Relaxed))
    }
}

type KeyframeCallback = Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>;

pub struct FrameSender {
    tx: mpsc::Sender<(Instant, Frame)>,
    seq: AtomicU32,
    need_keyframe: AtomicBool,
    pub stats: Arc<SenderStats>,
    closed: Arc<AtomicBool>,
    request_keyframe: KeyframeCallback,
}

impl FrameSender {
    pub fn new<W: AsyncWrite + Send + Unpin + 'static>(mut w: W, queue: usize) -> Self {
        let (tx, mut rx) = mpsc::channel::<(Instant, Frame)>(queue.max(1));
        let stats = Arc::new(SenderStats::default());
        let closed = Arc::new(AtomicBool::new(false));
        let (s, c) = (stats.clone(), closed.clone());
        tokio::spawn(async move {
            while let Some((t, f)) = rx.recv().await {
                s.last_delay_us
                    .store(t.elapsed().as_micros() as u64, Ordering::Relaxed);
                if write_frame(&mut w, &f).await.is_err() || w.flush().await.is_err() {
                    break;
                }
                s.sent.fetch_add(1, Ordering::Relaxed);
                s.bytes.fetch_add(f.data.len() as u64, Ordering::Relaxed);
            }
            c.store(true, Ordering::Relaxed);
            let _ = w.shutdown().await;
        });
        Self {
            tx,
            seq: AtomicU32::new(0),
            need_keyframe: AtomicBool::new(false),
            stats,
            closed,
            request_keyframe: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed) || self.tx.is_closed()
    }

    /// Callback asking the encoder for an IDR frame after drops.
    pub async fn on_keyframe_needed(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.request_keyframe.lock().await = Some(Box::new(f));
    }

    /// Queue a frame, waiting for room instead of dropping it: the
    /// producer slows down to the network (back-pressure), so every picture
    /// that goes out decodes cleanly and nothing waits for a keyframe.
    /// Returns false when the channel is closed.
    pub async fn send(&self, mut f: Frame) -> bool {
        f.seq = self.seq.fetch_add(1, Ordering::Relaxed);
        self.tx.send((Instant::now(), f)).await.is_ok()
    }

    /// Queue a frame. Returns false if it was dropped.
    pub fn push(&self, mut f: Frame) -> bool {
        if f.kind == FrameKind::Video && self.need_keyframe.load(Ordering::Relaxed) {
            if !f.is_keyframe() {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            self.need_keyframe.store(false, Ordering::Relaxed);
        }
        f.seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let kind = f.kind;
        match self.tx.try_send((Instant::now(), f)) {
            Ok(()) => true,
            Err(_) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                if kind == FrameKind::Video {
                    self.need_keyframe.store(true, Ordering::Relaxed);
                    if let Ok(cb) = self.request_keyframe.try_lock()
                        && let Some(cb) = cb.as_ref()
                    {
                        cb();
                    }
                }
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::read_frame;

    #[tokio::test]
    async fn drops_until_keyframe_when_full() {
        // Tiny pipe the reader doesn't drain: writer blocks, queue fills.
        let (w, mut r) = tokio::io::duplex(32);
        let s = FrameSender::new(w, 2);
        let mut accepted = 0;
        for i in 0..20 {
            if s.push(Frame::new(FrameKind::Video, i, vec![0u8; 64])) {
                accepted += 1;
            }
        }
        assert!(accepted < 20);
        assert!(s.stats.dropped.load(Ordering::Relaxed) > 0);
        // Non-keyframes are refused until a keyframe arrives.
        assert!(!s.push(Frame::new(FrameKind::Video, 100, vec![1])));
        // Drain so there is room again.
        let mut got = 0;
        while got < accepted {
            read_frame(&mut r).await.unwrap().unwrap();
            got += 1;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(s.push(Frame::new(FrameKind::Video, 101, vec![1]).keyframe()));
        assert!(s.push(Frame::new(FrameKind::Video, 102, vec![1])));
    }
}
