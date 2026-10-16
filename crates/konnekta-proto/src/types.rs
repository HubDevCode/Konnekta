//! Packet type constants.

// --- Core (KDE Connect) -------------------------------------------------------
pub const IDENTITY: &str = "kdeconnect.identity";
pub const PAIR: &str = "kdeconnect.pair";

// --- Standard KDE Connect plugins ---------------------------------------------
pub const PING: &str = "kdeconnect.ping";
pub const BATTERY: &str = "kdeconnect.battery";
pub const CLIPBOARD: &str = "kdeconnect.clipboard";
pub const CLIPBOARD_CONNECT: &str = "kdeconnect.clipboard.connect";
pub const SHARE_REQUEST: &str = "kdeconnect.share.request";
pub const SHARE_REQUEST_UPDATE: &str = "kdeconnect.share.request.update";
pub const MPRIS: &str = "kdeconnect.mpris";
pub const MPRIS_REQUEST: &str = "kdeconnect.mpris.request";
pub const MOUSEPAD_REQUEST: &str = "kdeconnect.mousepad.request";
pub const MOUSEPAD_ECHO: &str = "kdeconnect.mousepad.echo";
pub const MOUSEPAD_KEYBOARDSTATE: &str = "kdeconnect.mousepad.keyboardstate";
pub const PRESENTER: &str = "kdeconnect.presenter";
pub const RUNCOMMAND: &str = "kdeconnect.runcommand";
pub const RUNCOMMAND_REQUEST: &str = "kdeconnect.runcommand.request";
pub const NOTIFICATION: &str = "kdeconnect.notification";
pub const NOTIFICATION_REQUEST: &str = "kdeconnect.notification.request";
pub const NOTIFICATION_REPLY: &str = "kdeconnect.notification.reply";
pub const NOTIFICATION_ACTION: &str = "kdeconnect.notification.action";
pub const TELEPHONY: &str = "kdeconnect.telephony";
pub const TELEPHONY_REQUEST_MUTE: &str = "kdeconnect.telephony.request_mute";
pub const FINDMYPHONE_REQUEST: &str = "kdeconnect.findmyphone.request";

// --- Konnekta proprietary extensions ------------------------------------------
pub const K_DECK: &str = "konnekta.deck";
pub const K_DECK_REQUEST: &str = "konnekta.deck.request";
pub const K_DND: &str = "konnekta.dnd";
pub const K_SCREENSHOT: &str = "konnekta.screenshot";
pub const K_CALL_ACTION: &str = "konnekta.telephony.action";
pub const K_STREAM_REQUEST: &str = "konnekta.stream.request";
pub const K_STREAM_OFFER: &str = "konnekta.stream.offer";
pub const K_STREAM_STOP: &str = "konnekta.stream.stop";
pub const K_STREAM_STATS: &str = "konnekta.stream.stats";
pub const K_COMMAND_STATE: &str = "konnekta.command.state";

pub fn is_proprietary(packet_type: &str) -> bool {
    packet_type.starts_with("konnekta.")
}
