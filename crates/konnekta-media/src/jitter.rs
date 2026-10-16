//! Audio jitter buffer for interleaved i16 PCM.
//!
//! Absorbs network jitter: playback starts once `target` samples are queued;
//! on underrun it outputs silence and re-buffers; if latency grows beyond
//! `max`, the oldest samples are discarded to resynchronise.

use std::collections::VecDeque;

pub struct JitterBuffer {
    buf: VecDeque<i16>,
    target: usize,
    max: usize,
    playing: bool,
    pub underruns: u64,
    pub overflows: u64,
}

impl JitterBuffer {
    /// `sample_rate * channels` samples per second.
    pub fn new(sample_rate: u32, channels: u8, target_ms: u32, max_ms: u32) -> Self {
        let per_ms = (sample_rate as usize * channels as usize) / 1000;
        Self {
            buf: VecDeque::with_capacity(per_ms * max_ms as usize * 2),
            target: per_ms * target_ms as usize,
            max: per_ms * max_ms as usize,
            playing: false,
            underruns: 0,
            overflows: 0,
        }
    }

    pub fn push(&mut self, samples: &[i16]) {
        self.buf.extend(samples.iter().copied());
        if self.buf.len() > self.max {
            let excess = self.buf.len() - self.target;
            self.buf.drain(..excess);
            self.overflows += 1;
        }
        if !self.playing && self.buf.len() >= self.target {
            self.playing = true;
        }
    }

    /// Fill `out` completely (silence when not enough data).
    pub fn pop(&mut self, out: &mut [i16]) {
        if !self.playing {
            out.fill(0);
            return;
        }
        let n = out.len().min(self.buf.len());
        for (o, s) in out.iter_mut().zip(self.buf.drain(..n)) {
            *o = s;
        }
        if n < out.len() {
            out[n..].fill(0);
            self.playing = false;
            self.underruns += 1;
        }
    }

    pub fn buffered(&self) -> usize {
        self.buf.len()
    }
}

/// Little-endian bytes -> i16 samples.
pub fn bytes_to_i16(b: &[u8]) -> Vec<i16> {
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

pub fn i16_to_bytes(s: &[i16]) -> Vec<u8> {
    s.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_then_plays_and_recovers() {
        // 1 kHz mono -> 1 sample per ms for easy math.
        let mut j = JitterBuffer::new(1000, 1, 10, 50);
        let mut out = [1i16; 5];
        j.push(&[7; 5]);
        j.pop(&mut out);
        assert_eq!(out, [0; 5], "still buffering");
        j.push(&[7; 5]);
        j.pop(&mut out);
        assert_eq!(out, [7; 5]);
        j.pop(&mut out);
        assert_eq!(out, [7; 5]);
        j.pop(&mut out);
        assert_eq!(out, [0; 5]);
        assert_eq!(j.underruns, 1);
        j.push(&[3; 100]);
        assert_eq!(j.overflows, 1);
        assert_eq!(j.buffered(), 10);
    }

    #[test]
    fn pcm_conversion() {
        let s = [1i16, -2, 300];
        assert_eq!(bytes_to_i16(&i16_to_bytes(&s)), s);
    }
}
