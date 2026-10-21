use crate::{CryptoError, Result};
use rand::Rng;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SerialNumber};
use sha2::{Digest, Sha256};
use x509_parser::prelude::*;

/// The local device identity: id + self-signed certificate + private key.
#[derive(Clone)]
pub struct LocalIdentity {
    pub device_id: String,
    pub cert_der: Vec<u8>,
    key_der: Vec<u8>,
}

impl std::fmt::Debug for LocalIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print key material.
        f.debug_struct("LocalIdentity")
            .field("device_id", &self.device_id)
            .field("fingerprint", &cert_fingerprint(&self.cert_der))
            .finish_non_exhaustive()
    }
}

/// Generate a device id: 32 random lowercase hex chars (also accepted by the
/// KDE Connect and Flux apps).
pub fn generate_device_id() -> String {
    let bytes: [u8; 16] = rand::thread_rng().r#gen();
    hex::encode(bytes)
}

impl LocalIdentity {
    pub fn generate(device_id: &str) -> Result<Self> {
        let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)?;
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, device_id);
        dn.push(DnType::OrganizationName, "Konnekta");
        params.distinguished_name = dn;
        let now = ::time::OffsetDateTime::now_utc();
        params.not_before = now - ::time::Duration::days(365);
        params.not_after = now + ::time::Duration::days(365 * 10);
        let serial: [u8; 16] = rand::thread_rng().r#gen();
        params.serial_number = Some(SerialNumber::from_slice(&serial));
        let cert = params.self_signed(&key)?;
        Ok(Self {
            device_id: device_id.to_owned(),
            cert_der: cert.der().to_vec(),
            key_der: key.serialize_der(),
        })
    }

    pub fn from_parts(device_id: String, cert_der: Vec<u8>, key_der: Vec<u8>) -> Result<Self> {
        let cn = cert_common_name(&cert_der)?;
        if cn != device_id {
            return Err(CryptoError::InvalidCert(
                "certificate CN does not match device id".into(),
            ));
        }
        Ok(Self {
            device_id,
            cert_der,
            key_der,
        })
    }

    pub fn key_der(&self) -> &[u8] {
        &self.key_der
    }

    pub fn fingerprint(&self) -> String {
        cert_fingerprint(&self.cert_der)
    }
}

/// Group a key for display: `5EE6 825F 974E D59A`.
pub fn format_key(key: &str) -> String {
    key.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// SHA-256 fingerprint, colon separated upper hex.
pub fn cert_fingerprint(cert_der: &[u8]) -> String {
    let digest = Sha256::digest(cert_der);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn parse(cert_der: &[u8]) -> Result<X509Certificate<'_>> {
    let (rest, cert) =
        X509Certificate::from_der(cert_der).map_err(|e| CryptoError::InvalidCert(e.to_string()))?;
    if !rest.is_empty() {
        return Err(CryptoError::InvalidCert("trailing data".into()));
    }
    Ok(cert)
}

pub fn cert_common_name(cert_der: &[u8]) -> Result<String> {
    let cert = parse(cert_der)?;
    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .ok_or_else(|| CryptoError::InvalidCert("missing CN".into()))?;
    Ok(cn.to_owned())
}

/// DER encoded SubjectPublicKeyInfo.
pub fn public_key_der(cert_der: &[u8]) -> Result<Vec<u8>> {
    Ok(parse(cert_der)?.tbs_certificate.subject_pki.raw.to_vec())
}

/// Code both users compare during pairing:
/// `upper(hex(sha256(max(pkA,pkB) || min(pkA,pkB) || timestamp)))[..len]`.
/// Konnekta and the Flux bridge use 16 hex characters (64 bits); KDE Connect apps
/// show the first 8.
///
/// Because it commits to *both* public keys, a man in the middle presenting
/// its own certificate to either side produces a different code, and with 64
/// bits it cannot search for a colliding certificate while a request is open.
pub fn verification_key(
    local_cert: &[u8],
    remote_cert: &[u8],
    timestamp: u64,
    len: usize,
) -> Result<String> {
    let mut a = public_key_der(local_cert)?;
    let mut b = public_key_der(remote_cert)?;
    if a < b {
        std::mem::swap(&mut a, &mut b);
    }
    let mut h = Sha256::new();
    h.update(&a);
    h.update(&b);
    h.update(timestamp.to_string().as_bytes());
    Ok(hex::encode_upper(h.finalize())[..len.clamp(8, 64)].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_identity_has_matching_cn() {
        let id = generate_device_id();
        assert!(konnekta_valid(&id));
        let li = LocalIdentity::generate(&id).unwrap();
        assert_eq!(cert_common_name(&li.cert_der).unwrap(), id);
        assert_eq!(li.fingerprint().len(), 32 * 3 - 1);
        assert!(!format!("{li:?}").contains("key_der"));
    }

    fn konnekta_valid(id: &str) -> bool {
        id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())
    }

    #[test]
    fn verification_key_is_symmetric_and_binds_keys() {
        let a = LocalIdentity::generate(&generate_device_id()).unwrap();
        let b = LocalIdentity::generate(&generate_device_id()).unwrap();
        let m = LocalIdentity::generate(&generate_device_id()).unwrap();
        let ab = verification_key(&a.cert_der, &b.cert_der, 1234, 16).unwrap();
        let ba = verification_key(&b.cert_der, &a.cert_der, 1234, 16).unwrap();
        assert_eq!(ab, ba);
        assert_eq!(ab.len(), 16);
        // KDE Connect shows the prefix of the same hash.
        assert_eq!(
            verification_key(&a.cert_der, &b.cert_der, 1234, 8).unwrap(),
            ab[..8]
        );
        assert_eq!(format_key(&ab).len(), 19);
        // A MITM (m) talking to both sides yields different codes.
        assert_ne!(
            verification_key(&a.cert_der, &m.cert_der, 1234, 16).unwrap(),
            ab
        );
        assert_ne!(
            verification_key(&a.cert_der, &b.cert_der, 1235, 16).unwrap(),
            ab
        );
    }

    #[test]
    fn from_parts_rejects_mismatched_cn() {
        let a = LocalIdentity::generate(&generate_device_id()).unwrap();
        let other = generate_device_id();
        assert!(LocalIdentity::from_parts(other, a.cert_der.clone(), a.key_der.clone()).is_err());
    }
}
