//! Real-time media transport.
//!
//! Streams travel over a dedicated mutually-authenticated TLS channel
//! (see `konnekta_core::channel`), so audio and video are encrypted and bound
//! to the paired device exactly like control packets.
//!
//! Latency strategy:
//! * TCP_NODELAY, small frames, no buffering in the sender beyond a short
//!   bounded queue: when the network cannot keep up, *old* frames are dropped
//!   (never queued) and the adaptive bitrate controller steps quality down;
//! * receivers use a small jitter buffer for audio (default 60 ms);
//! * video is encoded with zero-latency settings (no B-frames, short GOP).

pub mod abr;
pub mod frame;
pub mod jitter;
pub mod pipeline;
pub mod sender;
pub mod setup;
pub mod virtual_devices;

pub use abr::{AdaptiveBitrate, Quality};
pub use frame::{Frame, FrameKind, read_frame, write_frame};
pub use jitter::JitterBuffer;
pub use sender::{FrameSender, SenderStats};
