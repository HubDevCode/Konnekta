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

/// Generate a protocol-v8 style device id: 32 lowercase hex chars with `_`
/// separators replaced (KDE uses a UUID with `-` replaced by `_`).
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
        // Organisation fields match KDE Connect so its clients accept us.
        dn.push(DnType::OrganizationName, "KDE");
        dn.push(DnType::OrganizationalUnitName, "Kde connect");
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
        Ok(Self { device_id, cert_der, key_der })
    }

    pub fn key_der(&self) -> &[u8] {
        &self.key_der
    }

    pub fn fingerprint(&self) -> String {
        cert_fingerprint(&self.cert_der)
    }
}

/// SHA-256 fingerprint, colon separated upper hex (as shown by KDE Connect).
pub fn cert_fingerprint(cert_der: &[u8]) -> String {
    let digest = Sha256::digest(cert_der);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn parse(cert_der: &[u8]) -> Result<X509Certificate<'_>> {
    let (rest, cert) = X509Certificate::from_der(cert_der)
        .map_err(|e| CryptoError::InvalidCert(e.to_string()))?;
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

/// Short code both users compare during pairing (KDE Connect v8 algorithm):
/// `upper(hex(sha256(max(pkA,pkB) || min(pkA,pkB) || timestamp)))[..8]`.
///
/// Because it commits to *both* public keys, a man in the middle presenting
/// its own certificate to either side produces a different code.
pub fn verification_key(local_cert: &[u8], remote_cert: &[u8], timestamp: u64) -> Result<String> {
    let mut a = public_key_der(local_cert)?;
    let mut b = public_key_der(remote_cert)?;
    if a < b {
        std::mem::swap(&mut a, &mut b);
    }
    let mut h = Sha256::new();
    h.update(&a);
    h.update(&b);
    h.update(timestamp.to_string().as_bytes());
    Ok(hex::encode_upper(h.finalize())[..8].to_owned())
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
        let ab = verification_key(&a.cert_der, &b.cert_der, 1234).unwrap();
        let ba = verification_key(&b.cert_der, &a.cert_der, 1234).unwrap();
        assert_eq!(ab, ba);
        assert_eq!(ab.len(), 8);
        // A MITM (m) talking to both sides yields different codes.
        assert_ne!(verification_key(&a.cert_der, &m.cert_der, 1234).unwrap(), ab);
        assert_ne!(verification_key(&a.cert_der, &b.cert_der, 1235).unwrap(), ab);
    }

    #[test]
    fn from_parts_rejects_mismatched_cn() {
        let a = LocalIdentity::generate(&generate_device_id()).unwrap();
        let other = generate_device_id();
        assert!(LocalIdentity::from_parts(other, a.cert_der.clone(), a.key_der.clone()).is_err());
    }
}
