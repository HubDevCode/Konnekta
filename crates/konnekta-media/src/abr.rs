//! Adaptive bitrate: steps quality down fast when the sender queue grows or
//! frames are dropped, and back up slowly after a stable period.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quality {
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
}

pub const DEFAULT_LADDER: &[Quality] = &[
    Quality {
        height: 480,
        fps: 24,
        bitrate_kbps: 1500,
    },
    Quality {
        height: 720,
        fps: 30,
        bitrate_kbps: 3500,
    },
    Quality {
        height: 900,
        fps: 30,
        bitrate_kbps: 6000,
    },
    Quality {
        height: 1080,
        fps: 30,
        bitrate_kbps: 8000,
    },
    Quality {
        height: 1080,
        fps: 60,
        bitrate_kbps: 12000,
    },
];

#[derive(Debug, Clone, Copy, Default)]
pub struct NetworkSample {
    /// Time the oldest queued frame has been waiting.
    pub queue_delay: Duration,
    /// Frames dropped since the previous sample.
    pub dropped: u32,
}

pub struct AdaptiveBitrate {
    ladder: Vec<Quality>,
    level: usize,
    max_level: usize,
    stable_since: Instant,
    last_change: Instant,
    pub up_after: Duration,
    pub min_interval: Duration,
}

impl AdaptiveBitrate {
    pub fn new(ladder: &[Quality], start: usize, max_level: usize) -> Self {
        let max_level = max_level.min(ladder.len() - 1);
        let now = Instant::now();
        Self {
            ladder: ladder.to_vec(),
            level: start.min(max_level),
            max_level,
            stable_since: now,
            last_change: now - Duration::from_secs(60),
            up_after: Duration::from_secs(8),
            min_interval: Duration::from_secs(5),
        }
    }

    pub fn current(&self) -> Quality {
        self.ladder[self.level]
    }

    pub fn level(&self) -> usize {
        self.level
    }

    /// Feed a measurement; returns the new quality if it changed.
    pub fn update(&mut self, s: NetworkSample, now: Instant) -> Option<Quality> {
        // Each change restarts the capture (a visible pause), so only real,
        // sustained congestion counts: a keyframe burst is not one.
        let congested = s.dropped > 3 || s.queue_delay > Duration::from_millis(300);
        if congested {
            self.stable_since = now;
            if self.level > 0 && now.duration_since(self.last_change) >= self.min_interval {
                // Severe congestion: drop two steps.
                let step = if s.queue_delay > Duration::from_millis(800) || s.dropped > 15 {
                    2
                } else {
                    1
                };
                self.level = self.level.saturating_sub(step);
                self.last_change = now;
                return Some(self.current());
            }
            return None;
        }
        if s.queue_delay > Duration::from_millis(40) {
            // Not congested but not idle either: hold.
            self.stable_since = now;
            return None;
        }
        if self.level < self.max_level
            && now.duration_since(self.stable_since) >= self.up_after
            && now.duration_since(self.last_change) >= self.min_interval
        {
            self.level += 1;
            self.last_change = now;
            self.stable_since = now;
            return Some(self.current());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_down_fast_up_slow() {
        let t0 = Instant::now();
        let mut abr = AdaptiveBitrate::new(DEFAULT_LADDER, 3, 4);
        assert_eq!(abr.current().height, 1080);
        // A short burst is not congestion.
        assert!(
            abr.update(
                NetworkSample {
                    queue_delay: Duration::from_millis(200),
                    dropped: 1,
                },
                t0,
            )
            .is_none()
        );
        let q = abr.update(
            NetworkSample {
                queue_delay: Duration::from_millis(400),
                dropped: 0,
            },
            t0,
        );
        assert_eq!(q.unwrap().height, 900);
        // Within min_interval: no further change.
        assert!(
            abr.update(
                NetworkSample {
                    queue_delay: Duration::from_millis(400),
                    dropped: 5
                },
                t0 + Duration::from_millis(500)
            )
            .is_none()
        );
        // Severe -> two steps.
        let q = abr.update(
            NetworkSample {
                queue_delay: Duration::from_secs(1),
                dropped: 20,
            },
            t0 + Duration::from_secs(6),
        );
        assert_eq!(abr.level(), 0);
        assert_eq!(q.unwrap().height, 480);
        // Stable for a while -> up one.
        let ok = NetworkSample::default();
        assert!(abr.update(ok, t0 + Duration::from_secs(8)).is_none());
        assert!(abr.update(ok, t0 + Duration::from_secs(15)).is_some());
        assert_eq!(abr.level(), 1);
    }

    #[test]
    fn respects_max_level() {
        let t0 = Instant::now();
        let mut abr = AdaptiveBitrate::new(DEFAULT_LADDER, 2, 2);
        for i in 0..10 {
            abr.update(NetworkSample::default(), t0 + Duration::from_secs(20 * i));
        }
        assert_eq!(abr.level(), 2);
    }
}
