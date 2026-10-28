//! Enrollment of the phone's key (`sudo konnekta approve enroll <phone>`).
//!
//! 1. The daemon of the user asks the phone, which makes a new key that
//!    needs the fingerprint and signs the enrollment message with it.
//! 2. The signature is checked with the new public key: the phone holds it.
//! 3. The user types the code the phone shows (first 8 bytes of the key's
//!    SHA-256). It is never printed here: code running as the user can
//!    write to the terminal, but cannot make the phone show another code.
//! 4. Root writes the key file.

use crate::client::Client;
use crate::helper::{nonce, now_secs};
use crate::key::{KeyFile, PublicKey, normalize_code};
use crate::keyfile;
use crate::message::Enrollment;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;

pub const TIMEOUT: u64 = 90;
pub const CODE_TRIES: usize = 3;

pub struct Settings {
    pub key_dir: PathBuf,
    pub socket: PathBuf,
    pub user: String,
    pub uid: u32,
    pub host: String,
}

fn e<T>(m: impl Into<String>) -> Result<T, String> {
    Err(m.into())
}

/// Find a paired, connected phone by name or id through the daemon. An
/// empty `q` picks the only connected phone that can approve.
fn find_device(c: &mut Client, q: &str) -> Result<(String, String), String> {
    let devices = c.call("devices", Value::Null).map_err(|e| e.0)?;
    if q.is_empty() {
        let list: Vec<&Value> = devices
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| {
                d["paired"] == true
                    && d["connected"] == true
                    && d["incomingCapabilities"]
                        .as_array()
                        .is_some_and(|c| c.iter().any(|t| t == "konnekta.approve.request"))
            })
            .collect();
        return match list.as_slice() {
            [d] => Ok((
                d["id"].as_str().unwrap_or_default().into(),
                d["name"].as_str().unwrap_or_default().into(),
            )),
            [] => e("nessun telefono connesso con l'app Konnekta"),
            _ => e(format!(
                "più telefoni connessi: scegline uno ({})",
                list.iter()
                    .filter_map(|d| d["name"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        };
    }
    let list: Vec<&Value> = devices
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| {
            d["paired"] == true
                && (d["id"] == q
                    || d["name"]
                        .as_str()
                        .is_some_and(|n| n.eq_ignore_ascii_case(q)))
        })
        .collect();
    match list.as_slice() {
        [d] if d["connected"] == true => Ok((
            d["id"].as_str().unwrap_or_default().into(),
            d["name"].as_str().unwrap_or_default().into(),
        )),
        [_] => e(format!("{q} non è connesso")),
        [] => e(format!("nessun dispositivo associato {q}")),
        _ => e(format!("{q} corrisponde a più dispositivi: usa l'id")),
    }
}

/// Run the enrollment. `ask_code` shows the prompt and returns one typed
/// line; `say` prints progress. Returns the written key file path.
pub fn run(
    s: &Settings,
    device: &str,
    ask_code: &mut dyn FnMut(&str) -> std::io::Result<String>,
    say: &mut dyn FnMut(&str),
) -> Result<PathBuf, String> {
    let kf = request_key(s, device, say)?;
    let mut ok = false;
    for _ in 0..CODE_TRIES {
        let line = ask_code("Scrivi il codice di 16 caratteri che vedi sul telefono: ")
            .map_err(|e| e.to_string())?;
        if normalize_code(&line).as_deref() == Some(kf.key.code().as_str()) {
            ok = true;
            break;
        }
        say("Il codice non corrisponde.");
    }
    if !ok {
        return e("i codici non corrispondono: non è stato salvato nulla");
    }
    keyfile::write(&s.key_dir, &s.user, &kf).map_err(|e| e.to_string())
}

/// Steps 1 and 2: the phone makes the key after the fingerprint and proves
/// it holds it. Nothing is written; the caller checks the code (step 3)
/// and writes the key (step 4).
pub fn request_key(
    s: &Settings,
    device: &str,
    say: &mut dyn FnMut(&str),
) -> Result<KeyFile, String> {
    let mut c = Client::connect(&s.socket, s.uid, Duration::from_secs(3)).map_err(|e| e.0)?;
    let (device_id, device_name) = find_device(&mut c, device)?;
    let (time, nonce) = (now_secs(), nonce().ok_or("no randomness")?);
    let r = c
        .approve(
            &device_id,
            "request",
            json!({ "kind": "enroll", "host": s.host, "user": s.user, "time": time, "nonce": nonce, "timeout": TIMEOUT }),
        )
        .map_err(|e| e.0)?;
    let id = r["id"].as_str().ok_or("no request id")?.to_owned();
    say(&format!(
        "Conferma sul telefono {device_name} con l'impronta…"
    ));
    c.set_timeout(Duration::from_secs(TIMEOUT + 5));
    let a = c
        .approve(&device_id, "wait", json!({ "id": id }))
        .map_err(|e| e.0)?;
    match a["result"].as_str() {
        Some("approved") => {}
        Some("error") => {
            return e(format!(
                "il telefono non ha creato la chiave: {}",
                a["error"].as_str().unwrap_or("errore")
            ));
        }
        other => {
            return e(format!(
                "registrazione non confermata ({})",
                other.unwrap_or("nessuna risposta")
            ));
        }
    }
    let spki = a["publicKey"]
        .as_str()
        .and_then(|k| B64.decode(k).ok())
        .ok_or("nessuna chiave pubblica")?;
    let key = PublicKey::from_spki_der(&spki).map_err(|e| e.to_string())?;
    let sig = a["signature"]
        .as_str()
        .and_then(|s| B64.decode(s).ok())
        .ok_or("nessuna firma")?;
    let message = Enrollment {
        host: s.host.clone(),
        user: s.user.clone(),
        key: key.sha256_hex(),
        time,
        nonce,
    }
    .message()
    .map_err(|e| e.to_string())?;
    if !key.verify(&message, &sig) {
        return e("la firma della registrazione non è valida");
    }
    Ok(KeyFile {
        device_id,
        device_name,
        enrolled: now_secs(),
        key,
    })
}
