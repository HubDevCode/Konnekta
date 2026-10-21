//! Packet types of the Konnekta protocol. Plugins use only these names; the
//! optional interop adapters (see [`crate::interop`]) translate them for
//! KDE Connect and Flux peers.

// --- Link ----------------------------------------------------------------------
pub const IDENTITY: &str = "konnekta.identity";
pub const PAIR: &str = "konnekta.pair";
/// The permissions of a pair, kept the same on both devices (handled by the
/// core, never by plugins). Body: `{"permissions": [...], "changedAt": ms}`.
pub const PERMISSIONS: &str = "konnekta.permissions";

// --- Basics --------------------------------------------------------------------
pub const PING: &str = "konnekta.ping";
pub const BATTERY: &str = "konnekta.battery";
/// Ring the device to find it.
pub const FIND_DEVICE: &str = "konnekta.finddevice";

// --- Clipboard -----------------------------------------------------------------
pub const CLIPBOARD: &str = "konnekta.clipboard";
/// Current clipboard sent right after connecting (with its timestamp).
pub const CLIPBOARD_INITIAL: &str = "konnekta.clipboard.initial";
pub const CLIPBOARD_IMAGE: &str = "konnekta.clipboard.image";

// --- Files ---------------------------------------------------------------------
pub const SHARE: &str = "konnekta.share";
pub const SHARE_UPDATE: &str = "konnekta.share.update";
/// The phone reads selected folders of the computer.
pub const BROWSE: &str = "konnekta.browse";
pub const BROWSE_REQUEST: &str = "konnekta.browse.request";
pub const SCREENSHOT: &str = "konnekta.screenshot";
/// Ask the phone for a camera photo, with `mode`: photo, scan (OCR, the
/// answer carries `text`) or qr (the answer carries `text`).
pub const PHOTO_REQUEST: &str = "konnekta.photo.request";
pub const PHOTO: &str = "konnekta.photo";
pub const APP_UPDATE: &str = "konnekta.app.update";

// --- Media and control ---------------------------------------------------------
pub const MEDIA: &str = "konnekta.media";
pub const MEDIA_REQUEST: &str = "konnekta.media.request";
/// Volume of every output, input and app of the computer. Body: mixer state.
pub const MIXER: &str = "konnekta.mixer";
/// Body: [`crate::bodies::MixerRequestBody`].
pub const MIXER_REQUEST: &str = "konnekta.mixer.request";
/// Pointer, clicks, scroll and keys.
pub const INPUT: &str = "konnekta.input";
pub const INPUT_ECHO: &str = "konnekta.input.echo";
pub const INPUT_STATE: &str = "konnekta.input.state";
pub const PRESENTER: &str = "konnekta.presenter";
/// List of the computer's custom commands.
pub const COMMANDS: &str = "konnekta.commands";
pub const COMMANDS_REQUEST: &str = "konnekta.commands.request";
pub const COMMAND_STATE: &str = "konnekta.command.state";
pub const DECK: &str = "konnekta.deck";
pub const DECK_REQUEST: &str = "konnekta.deck.request";

// --- Phone ---------------------------------------------------------------------
pub const NOTIFICATION: &str = "konnekta.notification";
pub const NOTIFICATION_REQUEST: &str = "konnekta.notification.request";
pub const NOTIFICATION_REPLY: &str = "konnekta.notification.reply";
pub const NOTIFICATION_ACTION: &str = "konnekta.notification.action";
pub const CALL: &str = "konnekta.call";
pub const CALL_MUTE: &str = "konnekta.call.mute";
/// Answer, reject or hang up, where the phone OS allows it.
pub const CALL_ACTION: &str = "konnekta.call.action";
pub const SMS_MESSAGES: &str = "konnekta.sms.messages";
pub const SMS_SEND: &str = "konnekta.sms.send";
pub const SMS_CONVERSATIONS_REQUEST: &str = "konnekta.sms.conversations.request";
pub const SMS_THREAD_REQUEST: &str = "konnekta.sms.thread.request";
pub const DND: &str = "konnekta.dnd";

// --- Security ------------------------------------------------------------------
pub const APPROVE: &str = "konnekta.approve";
pub const APPROVE_REQUEST: &str = "konnekta.approve.request";

/// Real-time streams: one packet type per stream kind so each is guarded by
/// its own permission. Body: [`crate::bodies::StreamMessage`].
pub const MICROPHONE: &str = "konnekta.microphone";
pub const CAMERA: &str = "konnekta.camera";
pub const DESKTOP: &str = "konnekta.desktop";
pub const SYSTEM_AUDIO: &str = "konnekta.systemaudio";
pub const PHONE_SCREEN: &str = "konnekta.phonescreen";
/// Audio playing on the phone, played on the computer's speakers.
pub const PHONE_AUDIO: &str = "konnekta.phoneaudio";
