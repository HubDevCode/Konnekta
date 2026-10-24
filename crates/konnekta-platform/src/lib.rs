//! Platform abstraction layer.
//!
//! Each OS feature is behind a small trait so plugins stay portable:
//! the desktop build uses native backends (this crate, feature `desktop`),
//! mobile builds plug in implementations backed by Kotlin / Swift through the
//! FFI crate, and tests use [`mock::MockPlatform`].

pub mod clipboard;
pub mod cmd;
pub mod display;
pub mod dnd;
pub mod input;
pub mod keys;
pub mod launcher;
pub mod media;
pub mod mixer;
pub mod mock;
pub mod notify;
#[cfg(all(feature = "desktop", target_os = "linux"))]
pub mod wlr_scroll;

use std::sync::Arc;

pub use clipboard::ClipboardBackend;
pub use dnd::DndBackend;
pub use input::{ButtonAction, InputBackend, MouseButton};
pub use keys::{Key, Modifiers, SpecialKey, parse_shortcut};
pub use launcher::{CommandOutput, LauncherBackend};
pub use media::{MediaAction, MediaBackend, MediaStatus};
pub use mixer::{MixerBackend, MixerChannel, MixerKind, MixerState};
pub use notify::{DesktopNotification, NotifierBackend};

/// Bundle of all backends handed to plugins.
#[derive(Clone)]
pub struct Platform {
    pub clipboard: Arc<dyn ClipboardBackend>,
    pub input: Arc<dyn InputBackend>,
    pub media: Arc<dyn MediaBackend>,
    pub mixer: Arc<dyn MixerBackend>,
    pub notifier: Arc<dyn NotifierBackend>,
    pub dnd: Arc<dyn DndBackend>,
    pub launcher: Arc<dyn LauncherBackend>,
}

impl Platform {
    /// Native backends for the current desktop OS.
    #[cfg(feature = "desktop")]
    pub fn native() -> Self {
        let input: Arc<dyn InputBackend> = input::native();
        Self {
            clipboard: clipboard::native(),
            media: media::native(input.clone()),
            mixer: mixer::native(),
            input,
            notifier: Arc::new(notify::NativeNotifier),
            dnd: dnd::native(),
            launcher: Arc::new(launcher::NativeLauncher),
        }
    }

    /// Backends that do nothing (headless servers, unsupported platforms).
    pub fn unsupported() -> Self {
        let m = Arc::new(mock::Unsupported);
        Self {
            clipboard: m.clone(),
            input: m.clone(),
            media: m.clone(),
            mixer: m.clone(),
            notifier: m.clone(),
            dnd: m.clone(),
            launcher: Arc::new(launcher::NativeLauncher),
        }
    }
}
