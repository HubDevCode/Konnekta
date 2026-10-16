//! Pairing state machine and anti-abuse bookkeeping.
//!
//! ```text
//!  NotPaired ──request_pair──▶ RequestedByUs ──peer pair:true──▶ Confirm ──user confirms key──▶ Paired
//!  NotPaired ──peer pair:true─▶ RequestedByPeer ──user accepts key──────────────────────────────▶ Paired
//! ```
//! Every open state expires after [`PAIR_TIMEOUT`]; expiry, rejection or a
//! dropped link send `pair:false` so the peer drops any pin it made. A
//! pairing belongs to the link (and certificate) on which it started.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const PAIR_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum clock skew accepted for the pairing timestamp.
pub const MAX_PAIR_CLOCK_SKEW_SECS: u64 = 1800;
/// Minimum interval between two pair requests of the same device.
pub const MIN_REQUEST_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairState {
    NotPaired,
    /// We asked; waiting for the peer.
    RequestedByUs {
        timestamp: u64,
        since: Instant,
    },
    /// The peer accepted our request; the local user must confirm the key
    /// before we pin the certificate.
    Confirm {
        timestamp: u64,
        since: Instant,
    },
    /// The peer asked; waiting for the local user.
    RequestedByPeer {
        timestamp: u64,
        since: Instant,
    },
    Paired,
}

impl PairState {
    pub fn is_paired(&self) -> bool {
        matches!(self, Self::Paired)
    }

    pub fn is_open(&self) -> bool {
        matches!(
            self,
            Self::RequestedByUs { .. } | Self::Confirm { .. } | Self::RequestedByPeer { .. }
        )
    }

    pub fn timestamp(&self) -> Option<u64> {
        match self {
            Self::RequestedByUs { timestamp, .. }
            | Self::Confirm { timestamp, .. }
            | Self::RequestedByPeer { timestamp, .. } => Some(*timestamp),
            _ => None,
        }
    }

    pub fn expired(&self) -> bool {
        match self {
            Self::RequestedByUs { since, .. }
            | Self::Confirm { since, .. }
            | Self::RequestedByPeer { since, .. } => since.elapsed() > PAIR_TIMEOUT,
            _ => false,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotPaired => "notPaired",
            Self::RequestedByUs { .. } => "requested",
            Self::Confirm { .. } => "confirm",
            Self::RequestedByPeer { .. } => "requestedByPeer",
            Self::Paired => "paired",
        }
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Reject stale or future pairing requests.
pub fn timestamp_is_fresh(ts: u64, now: u64) -> bool {
    ts.abs_diff(now) <= MAX_PAIR_CLOCK_SKEW_SECS
}

/// Compare a key typed or confirmed by the user with the expected one
/// (spaces and case ignored).
pub fn key_matches(given: &str, expected: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_uppercase()
    };
    !expected.is_empty() && norm(given) == norm(expected)
}

/// Rate limiting of incoming pair requests.
#[derive(Default)]
pub struct RequestGuard {
    last_request: HashMap<String, Instant>,
    cooldown_until: HashMap<String, Instant>,
}

pub enum Admission {
    Ok,
    TooFrequent,
    CoolingDown,
    TooMany,
}

impl RequestGuard {
    /// `open` = currently open incoming requests as (device, address).
    pub fn admit(
        &mut self,
        device: &str,
        addr: IpAddr,
        open: &[(String, IpAddr)],
        max_open: usize,
        max_per_addr: usize,
    ) -> Admission {
        let now = Instant::now();
        if self.cooldown_until.get(device).is_some_and(|t| *t > now) {
            return Admission::CoolingDown;
        }
        if self
            .last_request
            .get(device)
            .is_some_and(|t| now.duration_since(*t) < MIN_REQUEST_INTERVAL)
        {
            return Admission::TooFrequent;
        }
        self.last_request.insert(device.to_owned(), now);
        let others: Vec<_> = open.iter().filter(|(d, _)| d != device).collect();
        if others.len() >= max_open
            || others.iter().filter(|(_, a)| *a == addr).count() >= max_per_addr
        {
            return Admission::TooMany;
        }
        Admission::Ok
    }

    pub fn cool_down(&mut self, device: &str, d: Duration) {
        self.cooldown_until
            .insert(device.to_owned(), Instant::now() + d);
    }

    pub fn clear(&mut self, device: &str) {
        self.cooldown_until.remove(device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn freshness() {
        assert!(timestamp_is_fresh(1000, 1000));
        assert!(timestamp_is_fresh(1000, 1000 + MAX_PAIR_CLOCK_SKEW_SECS));
        assert!(!timestamp_is_fresh(1000, 1001 + MAX_PAIR_CLOCK_SKEW_SECS));
        assert!(!timestamp_is_fresh(5000, 1000));
    }

    #[test]
    fn keys() {
        assert!(key_matches("5ee6 825f 974e d59a", "5EE6825F974ED59A"));
        assert!(!key_matches("5EE6825F974ED59B", "5EE6825F974ED59A"));
        assert!(!key_matches("", ""));
    }

    #[test]
    fn request_guard() {
        let mut g = RequestGuard::default();
        let a = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let b = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        assert!(matches!(g.admit("d1", a, &[], 4, 2), Admission::Ok));
        assert!(matches!(
            g.admit("d1", a, &[], 4, 2),
            Admission::TooFrequent
        ));
        let open = vec![("x".to_string(), a), ("y".to_string(), a)];
        assert!(matches!(g.admit("d2", a, &open, 4, 2), Admission::TooMany));
        assert!(matches!(g.admit("d3", b, &open, 4, 2), Admission::Ok));
        g.cool_down("d4", Duration::from_secs(30));
        assert!(matches!(
            g.admit("d4", b, &[], 4, 2),
            Admission::CoolingDown
        ));
    }
}
