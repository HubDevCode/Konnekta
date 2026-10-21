//! The phone's public key: EC P-256 only, signatures `SHA256withECDSA` in
//! ASN.1 DER (what Android Keystore and the Secure Enclave produce).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use sha2::{Digest, Sha256};
use std::fmt;
use x509_parser::prelude::FromDer;
use x509_parser::x509::SubjectPublicKeyInfo;

const OID_EC_PUBLIC_KEY: &str = "1.2.840.10045.2.1";
const OID_P256: &str = "1.2.840.10045.3.1.7";
/// Key file: metadata plus one PEM block, at most this size.
pub const MAX_KEY_FILE: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyError(pub &'static str);

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for KeyError {}

/// A validated P-256 public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    /// SubjectPublicKeyInfo, DER.
    spki: Vec<u8>,
    /// Uncompressed point (65 bytes, `0x04 || X || Y`).
    point: Vec<u8>,
}

impl PublicKey {
    pub fn from_spki_der(der: &[u8]) -> Result<Self, KeyError> {
        let (rest, spki) =
            SubjectPublicKeyInfo::from_der(der).map_err(|_| KeyError("not a public key"))?;
        if !rest.is_empty() {
            return Err(KeyError("trailing data after the key"));
        }
        if spki.algorithm.algorithm.to_id_string() != OID_EC_PUBLIC_KEY {
            return Err(KeyError("not an EC key"));
        }
        let curve = spki
            .algorithm
            .parameters
            .as_ref()
            .and_then(|p| p.as_oid().ok())
            .map(|o| o.to_id_string());
        if curve.as_deref() != Some(OID_P256) {
            return Err(KeyError("not a P-256 key"));
        }
        let point = spki.subject_public_key.data.to_vec();
        if point.len() != 65 || point[0] != 0x04 {
            return Err(KeyError("bad EC point"));
        }
        Ok(Self {
            spki: der.to_vec(),
            point,
        })
    }

    pub fn spki_der(&self) -> &[u8] {
        &self.spki
    }

    /// SHA-256 of the SubjectPublicKeyInfo, 64 lowercase hex (enrollment).
    pub fn sha256_hex(&self) -> String {
        hex::encode(Sha256::digest(&self.spki))
    }

    /// Code the phone shows and the user types at enrollment: first 8 bytes
    /// of the SHA-256, 16 uppercase hex digits.
    pub fn code(&self) -> String {
        hex::encode_upper(&Sha256::digest(&self.spki)[..8])
    }

    pub fn verify(&self, message: &[u8], signature_der: &[u8]) -> bool {
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_ASN1,
            &self.point,
        )
        .verify(message, signature_der)
        .is_ok()
    }

    pub fn to_pem(&self) -> String {
        let b = B64.encode(&self.spki);
        let mut out = String::from("-----BEGIN PUBLIC KEY-----\n");
        for chunk in b.as_bytes().chunks(64) {
            out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
            out.push('\n');
        }
        out.push_str("-----END PUBLIC KEY-----\n");
        out
    }
}

/// Typed code -> 16 uppercase hex, ignoring whitespace (also the newline
/// of the typed line), hyphens and case.
pub fn normalize_code(typed: &str) -> Option<String> {
    let s: String = typed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_uppercase();
    (s.len() == 16 && s.chars().all(|c| c.is_ascii_hexdigit())).then_some(s)
}

/// Contents of `/etc/konnekta/approve/<user>.pub`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFile {
    pub device_id: String,
    pub device_name: String,
    pub enrolled: u64,
    pub key: PublicKey,
}

fn valid_device_id(id: &str) -> bool {
    (32..=38).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl KeyFile {
    pub fn render(&self) -> String {
        format!(
            "# Konnekta: phone key that approves sudo / polkit for this user.\n\
             # Written by `konnekta approve enroll`; owned by root.\n\
             device-id={}\ndevice-name={}\nenrolled={}\n{}",
            self.device_id,
            self.device_name,
            self.enrolled,
            self.key.to_pem()
        )
    }

    pub fn parse(text: &str) -> Result<Self, KeyError> {
        if text.len() as u64 > MAX_KEY_FILE {
            return Err(KeyError("key file too large"));
        }
        let (mut id, mut name, mut enrolled) = (None, None, 0u64);
        let mut lines = text.lines();
        let mut b64 = String::new();
        let mut in_block = false;
        let mut done = false;
        for l in lines.by_ref() {
            let l = l.trim_end_matches('\r');
            if done {
                if !l.trim().is_empty() {
                    return Err(KeyError("data after the key"));
                }
                continue;
            }
            if in_block {
                if l == "-----END PUBLIC KEY-----" {
                    done = true;
                } else {
                    b64.push_str(l.trim());
                }
                continue;
            }
            if l.starts_with('#') || l.trim().is_empty() {
                continue;
            }
            if l == "-----BEGIN PUBLIC KEY-----" {
                in_block = true;
                continue;
            }
            let (k, v) = l.split_once('=').ok_or(KeyError("bad metadata line"))?;
            if v.chars().any(crate::message::is_control) || v.len() > 128 {
                return Err(KeyError("bad metadata value"));
            }
            match k {
                "device-id" => id = Some(v.to_owned()),
                "device-name" => name = Some(v.to_owned()),
                "enrolled" => enrolled = v.parse().map_err(|_| KeyError("bad enrolled time"))?,
                _ => return Err(KeyError("unknown metadata")),
            }
        }
        if !done {
            return Err(KeyError("missing PEM public key"));
        }
        let der = B64
            .decode(b64.as_bytes())
            .map_err(|_| KeyError("bad base64"))?;
        let device_id = id
            .filter(|i| valid_device_id(i))
            .ok_or(KeyError("bad device-id"))?;
        Ok(Self {
            device_id,
            device_name: name.unwrap_or_default(),
            enrolled,
            key: PublicKey::from_spki_der(&der)?,
        })
    }
}

#[cfg(test)]
pub(crate) mod test_key {
    //! A software "phone" key for tests.
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};

    /// SubjectPublicKeyInfo prefix for an uncompressed P-256 point.
    const P256_SPKI_PREFIX: [u8; 26] = [
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];

    pub struct Phone(pub EcdsaKeyPair);

    impl Phone {
        pub fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
            Self(
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
                    .unwrap(),
            )
        }
        pub fn spki(&self) -> Vec<u8> {
            [&P256_SPKI_PREFIX[..], self.0.public_key().as_ref()].concat()
        }
        pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
            self.0
                .sign(&SystemRandom::new(), msg)
                .unwrap()
                .as_ref()
                .to_vec()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_key::Phone;
    use super::*;

    #[test]
    fn verify_and_reject() {
        let phone = Phone::new();
        let key = PublicKey::from_spki_der(&phone.spki()).unwrap();
        let sig = phone.sign(b"hello");
        assert!(key.verify(b"hello", &sig));
        assert!(!key.verify(b"hellO", &sig));
        let other = PublicKey::from_spki_der(&Phone::new().spki()).unwrap();
        assert!(!other.verify(b"hello", &sig));
        assert!(!key.verify(b"hello", b"garbage"));
        assert_eq!(key.code().len(), 16);
        assert_eq!(key.sha256_hex()[..16].to_uppercase(), key.code());
    }

    #[test]
    fn only_p256_is_accepted() {
        // An RSA SubjectPublicKeyInfo from a real certificate.
        let id = rcgen_free_rsa_like();
        assert!(PublicKey::from_spki_der(&id).is_err());
        assert!(PublicKey::from_spki_der(b"junk").is_err());
        let mut spki = Phone::new().spki();
        spki.push(0);
        assert_eq!(
            PublicKey::from_spki_der(&spki),
            Err(KeyError("trailing data after the key"))
        );
    }

    /// An Ed25519 SPKI (OID 1.3.101.112): a valid key of the wrong kind.
    fn rcgen_free_rsa_like() -> Vec<u8> {
        let mut v = vec![
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        v.extend([7u8; 32]);
        v
    }

    #[test]
    fn key_file_roundtrip_and_strict_parsing() {
        let phone = Phone::new();
        let f = KeyFile {
            device_id: "0123456789abcdef0123456789abcdef".into(),
            device_name: "Pixel 8".into(),
            enrolled: 1_760_000_000,
            key: PublicKey::from_spki_der(&phone.spki()).unwrap(),
        };
        let text = f.render();
        assert_eq!(KeyFile::parse(&text).unwrap(), f);
        assert!(KeyFile::parse(&text.replace("device-id=", "device-id=../")).is_err());
        assert!(KeyFile::parse(&format!("{text}extra\n")).is_err());
        assert!(KeyFile::parse(&text.replace("-----END PUBLIC KEY-----\n", "")).is_err());
        assert!(
            KeyFile::parse(&format!("path=/tmp/x\n{text}")).is_err(),
            "unknown keys refused"
        );
    }

    #[test]
    fn typed_codes() {
        assert_eq!(
            normalize_code("5ee6 825f-974e d59a").as_deref(),
            Some("5EE6825F974ED59A")
        );
        assert_eq!(
            normalize_code("5EE6 825F 974E D59A\n").as_deref(),
            Some("5EE6825F974ED59A"),
            "line from read_line"
        );
        assert_eq!(normalize_code("5EE6 825F"), None);
        assert_eq!(normalize_code("5EE6825F974ED59Z"), None);
    }
}
