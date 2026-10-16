//! Konnekta core runtime.
//!
//! Responsibilities:
//! * device discovery (UDP broadcast, KDE Connect compatible);
//! * secure links: TCP + mutual TLS with certificate pinning;
//! * pairing with user-confirmed verification keys;
//! * centralised, per-device, per-feature permissions;
//! * plugin registry and packet dispatch (a packet only reaches a plugin if the
//!   device is paired, the plugin is enabled and the permission is granted);
//! * encrypted payload transfers (files, screenshots) with optional resume.

pub mod config;
pub mod core;
pub mod device;
pub mod discovery;
pub mod event;
pub mod link;
pub mod pairing;
pub mod payload;
pub mod permissions;
pub mod plugin;
pub mod trust;
pub mod util;

pub use crate::core::{Konnekta, KonnektaBuilder};
pub use config::Config;
pub use device::{DeviceHandle, DeviceInfo};
pub use event::Event;
pub use permissions::Permission;
pub use plugin::{CoreHandle, Plugin, PluginInfo, PluginOrigin};

pub use konnekta_proto as proto;
