//! Konnekta core runtime.
//!
//! Responsibilities:
//! * device discovery (UDP broadcast, mDNS, extra addresses);
//! * optional bridges to KDE Connect and Flux apps, translated in the link;
//! * secure links: TCP + mutual TLS with certificate pinning;
//! * pairing with user-confirmed verification keys;
//! * centralised, per-device, per-feature permissions;
//! * plugin registry and packet dispatch (a packet only reaches a plugin if the
//!   device is paired, the plugin is enabled and the permission is granted);
//! * encrypted payload transfers (files, screenshots) with optional resume.

pub mod api;
pub mod channel;
pub mod config;
pub mod core;
pub mod device;
pub mod discovery;
pub mod event;
pub mod ipc;
pub mod link;
pub mod pairing;
pub mod payload;
pub mod permissions;
pub mod plugin;
pub mod trust;
pub mod util;

pub use crate::core::{Konnekta, KonnektaBuilder};
pub use channel::SecureChannel;
pub use config::{Adapter, Config};
pub use device::{DeviceHandle, DeviceInfo};
pub use event::Event;
pub use permissions::Permission;
pub use plugin::{CoreHandle, Plugin, PluginInfo};

pub use konnekta_proto as proto;
