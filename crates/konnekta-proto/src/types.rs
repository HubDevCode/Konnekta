//! Canonical packet types. Plugins use only these names; KDE Connect links
//! translate them (see [`crate::kde`]).

// --- Core -----------------------------------------------------------------------
pub const IDENTITY: &str = "konnekta.identity";
pub const PAIR: &str = "konnekta.pair";

// --- Features shared with KDE Connect (translated in compatibility mode) --------
pub const PING: &str = "konnekta.ping";
pub const BATTERY: &str = "konnekta.battery";
pub const CLIPBOARD: &str = "konnekta.clipboard";
pub const CLIPBOARD_CONNECT: &str = "konnekta.clipboard.connect";
pub const SHARE_REQUEST: &str = "konnekta.share.request";
pub const SHARE_REQUEST_UPDATE: &str = "konnekta.share.request.update";
pub const MPRIS: &str = "konnekta.mpris";
pub const MPRIS_REQUEST: &str = "konnekta.mpris.request";
pub const MOUSEPAD_REQUEST: &str = "konnekta.mousepad.request";
pub const MOUSEPAD_ECHO: &str = "konnekta.mousepad.echo";
pub const MOUSEPAD_KEYBOARDSTATE: &str = "konnekta.mousepad.keyboardstate";
pub const PRESENTER: &str = "konnekta.presenter";
pub const RUNCOMMAND: &str = "konnekta.runcommand";
pub const RUNCOMMAND_REQUEST: &str = "konnekta.runcommand.request";
pub const NOTIFICATION: &str = "konnekta.notification";
pub const NOTIFICATION_REQUEST: &str = "konnekta.notification.request";
pub const NOTIFICATION_REPLY: &str = "konnekta.notification.reply";
pub const NOTIFICATION_ACTION: &str = "konnekta.notification.action";
pub const TELEPHONY: &str = "konnekta.telephony";
pub const TELEPHONY_REQUEST_MUTE: &str = "konnekta.telephony.request_mute";
pub const FINDMYPHONE_REQUEST: &str = "konnekta.findmyphone.request";
pub const SMS_MESSAGES: &str = "konnekta.sms.messages";
pub const SMS_REQUEST: &str = "konnekta.sms.request";
pub const SMS_REQUEST_CONVERSATIONS: &str = "konnekta.sms.request_conversations";
pub const SMS_REQUEST_CONVERSATION: &str = "konnekta.sms.request_conversation";

// --- Konnekta-only features ----------------------------------------------------
pub const K_DECK: &str = "konnekta.deck";
pub const K_DECK_REQUEST: &str = "konnekta.deck.request";
pub const K_DND: &str = "konnekta.dnd";
pub const K_SCREENSHOT: &str = "konnekta.screenshot";
pub const K_CALL_ACTION: &str = "konnekta.telephony.action";
pub const K_COMMAND_STATE: &str = "konnekta.command.state";
pub const K_CLIPBOARD_IMAGE: &str = "konnekta.clipboard.image";
pub const K_BROWSE: &str = "konnekta.browse";
pub const K_BROWSE_REQUEST: &str = "konnekta.browse.request";
pub const K_APPROVE: &str = "konnekta.approve";
pub const K_APPROVE_REQUEST: &str = "konnekta.approve.request";
pub const K_APP_UPDATE: &str = "konnekta.app.update";

/// Real-time streams: one packet type per stream kind so each is guarded by
/// its own permission. Body: [`crate::bodies::StreamMessage`].
pub const K_MICROPHONE: &str = "konnekta.microphone";
pub const K_CAMERA: &str = "konnekta.camera";
pub const K_DESKTOP: &str = "konnekta.desktop";
pub const K_SYSTEM_AUDIO: &str = "konnekta.systemaudio";
pub const K_PHONE_SCREEN: &str = "konnekta.phonescreen";

/// True for types that only exist in the native protocol.
pub fn is_native_only(packet_type: &str) -> bool {
    crate::kde::to_kde(packet_type).is_none()
}
