use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long a pairing request stays valid.
pub const PAIR_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum clock skew accepted for the pairing timestamp (KDE uses 30 min).
pub const MAX_PAIR_CLOCK_SKEW_SECS: u64 = 1800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairState {
    NotPaired,
    RequestedByUs { timestamp: u64, since: Instant },
    RequestedByPeer { timestamp: u64, since: Instant },
    Paired,
}

impl PairState {
    pub fn is_paired(&self) -> bool {
        matches!(self, Self::Paired)
    }

    pub fn expired(&self) -> bool {
        match self {
            Self::RequestedByUs { since, .. } | Self::RequestedByPeer { since, .. } => {
                since.elapsed() > PAIR_TIMEOUT
            }
            _ => false,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotPaired => "notPaired",
            Self::RequestedByUs { .. } => "requested",
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

/// Reject stale or future pairing requests (limits replay of a captured
/// pairing exchange and keeps the verification key fresh).
pub fn timestamp_is_fresh(ts: u64, now: u64) -> bool {
    ts.abs_diff(now) <= MAX_PAIR_CLOCK_SKEW_SECS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness() {
        assert!(timestamp_is_fresh(1000, 1000));
        assert!(timestamp_is_fresh(1000, 1000 + MAX_PAIR_CLOCK_SKEW_SECS));
        assert!(!timestamp_is_fresh(1000, 1001 + MAX_PAIR_CLOCK_SKEW_SECS));
        assert!(!timestamp_is_fresh(5000, 1000));
    }
}
