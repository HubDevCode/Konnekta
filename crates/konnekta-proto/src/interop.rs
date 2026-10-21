//! Optional interoperability with other ecosystems, applied only at the link
//! boundary. Plugins never see foreign names: a packet from a KDE Connect or
//! Flux peer is translated to its Konnekta type (and body, where they
//! differ) on the way in, and back on the way out. Types without an entry
//! never cross the boundary.
//!
//! Both foreign protocols are "v8 style": clear-text identity on TCP, then
//! TLS with the roles reversed (see `konnekta-core::link`).

use crate::{Protocol, types as t};
use serde_json::{Map, Value, json};

/// (foreign suffix, Konnekta type). The foreign name is `<prefix><suffix>`.
type Table = &'static [(&'static str, &'static str)];

const KDE_CONNECT: Table = &[
    ("identity", t::IDENTITY),
    ("pair", t::PAIR),
    ("ping", t::PING),
    ("battery", t::BATTERY),
    ("findmyphone.request", t::FIND_DEVICE),
    ("clipboard", t::CLIPBOARD),
    ("clipboard.connect", t::CLIPBOARD_INITIAL),
    ("share.request", t::SHARE),
    ("share.request.update", t::SHARE_UPDATE),
    ("mpris", t::MEDIA),
    ("mpris.request", t::MEDIA_REQUEST),
    ("mousepad.request", t::INPUT),
    ("mousepad.echo", t::INPUT_ECHO),
    ("mousepad.keyboardstate", t::INPUT_STATE),
    ("presenter", t::PRESENTER),
    ("runcommand", t::COMMANDS),
    ("runcommand.request", t::COMMANDS_REQUEST),
    ("notification", t::NOTIFICATION),
    ("notification.request", t::NOTIFICATION_REQUEST),
    ("notification.reply", t::NOTIFICATION_REPLY),
    ("notification.action", t::NOTIFICATION_ACTION),
    ("telephony", t::CALL),
    ("telephony.request_mute", t::CALL_MUTE),
    ("sms.messages", t::SMS_MESSAGES),
    ("sms.request", t::SMS_SEND),
    ("sms.request_conversations", t::SMS_CONVERSATIONS_REQUEST),
    ("sms.request_conversation", t::SMS_THREAD_REQUEST),
    ("photo.request", t::PHOTO_REQUEST),
    ("photo", t::PHOTO),
];

const FLUX: Table = &[
    ("identity", t::IDENTITY),
    ("pair", t::PAIR),
    ("ping", t::PING),
    ("battery", t::BATTERY),
    ("findmyphone.request", t::FIND_DEVICE),
    ("clipboard", t::CLIPBOARD),
    ("clipboard.connect", t::CLIPBOARD_INITIAL),
    ("clipboard.image", t::CLIPBOARD_IMAGE),
    ("share.request", t::SHARE),
    ("share.request.update", t::SHARE_UPDATE),
    ("mpris", t::MEDIA),
    ("mpris.request", t::MEDIA_REQUEST),
    ("mousepad.request", t::INPUT),
    ("runcommand", t::COMMANDS),
    ("runcommand.request", t::COMMANDS_REQUEST),
    ("notification", t::NOTIFICATION),
    ("notification.request", t::NOTIFICATION_REQUEST),
    ("notification.reply", t::NOTIFICATION_REPLY),
    ("notification.action", t::NOTIFICATION_ACTION),
    ("telephony", t::CALL),
    ("sms.messages", t::SMS_MESSAGES),
    ("sms.request", t::SMS_SEND),
    ("sms.request_conversations", t::SMS_CONVERSATIONS_REQUEST),
    ("sms.request_conversation", t::SMS_THREAD_REQUEST),
    ("dnd", t::DND),
];

fn dialect(p: Protocol) -> Option<(&'static str, Table)> {
    match p {
        Protocol::Konnekta => None,
        Protocol::KdeConnect => Some(("kdeconnect.", KDE_CONNECT)),
        Protocol::Flux => Some(("flux.", FLUX)),
    }
}

/// Konnekta type -> wire name for `p`; `None` if `p` has no equivalent.
pub fn to_wire(p: Protocol, canonical: &str) -> Option<String> {
    let Some((prefix, table)) = dialect(p) else {
        return Some(canonical.to_owned());
    };
    table
        .iter()
        .find(|(_, c)| *c == canonical)
        .map(|(s, _)| format!("{prefix}{s}"))
}

/// Wire name of `p` -> Konnekta type; `None` if Konnekta has no equivalent.
pub fn from_wire(p: Protocol, wire: &str) -> Option<&'static str> {
    let (prefix, table) = dialect(p)?;
    let suffix = wire.strip_prefix(prefix)?;
    table.iter().find(|(s, _)| *s == suffix).map(|(_, c)| *c)
}

/// Capabilities announced by a foreign peer, as Konnekta types.
pub fn caps_from_wire(p: Protocol, caps: &[String]) -> Vec<String> {
    if p == Protocol::Konnekta {
        return caps.to_vec();
    }
    caps.iter()
        .filter_map(|c| from_wire(p, c))
        .map(str::to_owned)
        .collect()
}

/// Our capabilities, as announced to a peer speaking `p`.
pub fn caps_to_wire(p: Protocol, caps: &[String]) -> Vec<String> {
    caps.iter().filter_map(|c| to_wire(p, c)).collect()
}

/// Foreign protocols in which this Konnekta type exists.
pub fn supported_by(canonical: &str) -> Vec<Protocol> {
    [Protocol::KdeConnect, Protocol::Flux]
        .into_iter()
        .filter(|p| to_wire(*p, canonical).is_some())
        .collect()
}

/// Body of a packet arriving from `p`, rewritten to the Konnekta shape.
pub fn body_in(p: Protocol, canonical: &str, body: &mut Map<String, Value>, now_ms: i64) {
    if p == Protocol::Flux && canonical == t::DND {
        // Flux: {"on": bool}, a user change on the phone.
        let on = body.get("on").and_then(Value::as_bool).unwrap_or(false);
        *body = object(json!({ "enabled": on, "changedAt": now_ms, "userInitiated": true }));
    }
}

fn object(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// Body of a packet leaving for `p`, rewritten to the foreign shape.
pub fn body_out(p: Protocol, canonical: &str, body: &mut Map<String, Value>) {
    if p == Protocol::Flux && canonical == t::DND {
        let on = body
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        *body = object(json!({ "on": on }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_roundtrip() {
        for p in [Protocol::KdeConnect, Protocol::Flux] {
            for c in [t::PING, t::SHARE, t::MEDIA, t::INPUT, t::SMS_SEND] {
                let w = to_wire(p, c).unwrap();
                assert_eq!(from_wire(p, &w), Some(c), "{w}");
            }
        }
        assert_eq!(
            to_wire(Protocol::KdeConnect, t::MEDIA).as_deref(),
            Some("kdeconnect.mpris")
        );
        assert_eq!(
            to_wire(Protocol::Flux, t::SMS_SEND).as_deref(),
            Some("flux.sms.request")
        );
        assert_eq!(
            to_wire(Protocol::Konnekta, t::MEDIA).as_deref(),
            Some(t::MEDIA)
        );
    }

    #[test]
    fn konnekta_only_types_never_leave() {
        for c in [t::DECK, t::APPROVE_REQUEST, t::BROWSE, t::MICROPHONE] {
            assert_eq!(to_wire(Protocol::KdeConnect, c), None);
            assert_eq!(to_wire(Protocol::Flux, c), None);
        }
        // Wrong namespace or unknown suffix: dropped.
        assert_eq!(from_wire(Protocol::Flux, "kdeconnect.ping"), None);
        assert_eq!(from_wire(Protocol::KdeConnect, "kdeconnect.sftp"), None);
        assert_eq!(from_wire(Protocol::Flux, "flux.approve"), None);
    }

    #[test]
    fn capabilities_and_support() {
        let caps = vec![
            "flux.ping".to_string(),
            "flux.herdr".to_string(),
            "flux.dnd".to_string(),
        ];
        assert_eq!(caps_from_wire(Protocol::Flux, &caps), [t::PING, t::DND]);
        assert_eq!(supported_by(t::DND), [Protocol::Flux]);
        assert_eq!(supported_by(t::PRESENTER), [Protocol::KdeConnect]);
        assert!(supported_by(t::DECK).is_empty());
    }

    #[test]
    fn flux_dnd_body() {
        let mut b = object(json!({ "on": true }));
        body_in(Protocol::Flux, t::DND, &mut b, 5);
        assert_eq!(
            Value::Object(b.clone()),
            json!({ "enabled": true, "changedAt": 5, "userInitiated": true })
        );
        body_out(Protocol::Flux, t::DND, &mut b);
        assert_eq!(Value::Object(b), json!({ "on": true }));
    }
}
