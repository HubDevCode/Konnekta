//! KDE Connect compatibility: translation of packet types at the link
//! boundary. Only the features both protocols share are translated; native
//! extensions never reach a KDE Connect peer.

pub const IDENTITY: &str = "kdeconnect.identity";
pub const PAIR: &str = "kdeconnect.pair";

const NATIVE: &str = "konnekta.";
const KDE: &str = "kdeconnect.";

/// Feature suffixes shared by both protocols (same JSON bodies).
const SHARED: &[&str] = &[
    "identity",
    "pair",
    "ping",
    "battery",
    "clipboard",
    "clipboard.connect",
    "share.request",
    "share.request.update",
    "mpris",
    "mpris.request",
    "mousepad.request",
    "mousepad.echo",
    "mousepad.keyboardstate",
    "presenter",
    "runcommand",
    "runcommand.request",
    "notification",
    "notification.request",
    "notification.reply",
    "notification.action",
    "telephony",
    "telephony.request_mute",
    "findmyphone.request",
    "sms.messages",
    "sms.request",
    "sms.request_conversations",
    "sms.request_conversation",
];

/// Canonical (`konnekta.*`) -> KDE Connect name, if KDE Connect has it.
pub fn to_kde(canonical: &str) -> Option<String> {
    let suffix = canonical.strip_prefix(NATIVE)?;
    SHARED.contains(&suffix).then(|| format!("{KDE}{suffix}"))
}

/// KDE Connect name -> canonical name, if Konnekta implements it.
pub fn from_kde(kde: &str) -> Option<String> {
    let suffix = kde.strip_prefix(KDE)?;
    SHARED
        .contains(&suffix)
        .then(|| format!("{NATIVE}{suffix}"))
}

/// Translate a capability list received from a KDE Connect peer.
pub fn caps_from_kde(caps: &[String]) -> Vec<String> {
    caps.iter().filter_map(|c| from_kde(c)).collect()
}

/// Translate our capability list for a KDE Connect peer.
pub fn caps_to_kde(caps: &[String]) -> Vec<String> {
    caps.iter().filter_map(|c| to_kde(c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types;

    #[test]
    fn translation_roundtrip() {
        assert_eq!(to_kde(types::PING).as_deref(), Some("kdeconnect.ping"));
        assert_eq!(
            from_kde("kdeconnect.mpris.request").as_deref(),
            Some(types::MPRIS_REQUEST)
        );
        assert_eq!(
            from_kde(&to_kde(types::SMS_REQUEST).unwrap()).as_deref(),
            Some(types::SMS_REQUEST)
        );
        // Native extensions never leave the native protocol.
        assert_eq!(to_kde(types::K_DECK), None);
        assert_eq!(to_kde(types::K_APPROVE_REQUEST), None);
        // Unknown KDE plugins are ignored.
        assert_eq!(from_kde("kdeconnect.sftp"), None);
        assert_eq!(from_kde("konnekta.ping"), None);
        assert!(types::is_native_only(types::K_MICROPHONE));
    }
}
