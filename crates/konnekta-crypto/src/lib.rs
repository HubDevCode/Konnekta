//! Cryptographic building blocks for Konnekta.
//!
//! * every device owns a self-signed certificate whose CN is its device id;
//! * connections use mutual TLS (1.2/1.3, AEAD + forward secrecy only);
//! * paired devices are verified by **certificate pinning** (exact DER match),
//!   unpaired devices are only allowed to exchange pairing packets and must be
//!   confirmed by the user through a short verification key shown on both
//!   screens;
//! * private keys are stored with OS protection (credential store when the
//!   `os-keyring` feature is enabled, otherwise an owner-only file).

pub mod cert;
pub mod store;
pub mod tls;

pub use cert::{
    LocalIdentity, cert_common_name, cert_fingerprint, format_key, generate_device_id,
    public_key_der, verification_key,
};
pub use store::{FileKeyStore, KeyStore, write_private_file};
pub use tls::{PeerExpectation, client_config, server_config};

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("certificate generation failed: {0}")]
    Generate(#[from] rcgen::Error),
    #[error("invalid certificate: {0}")]
    InvalidCert(String),
    #[error("tls configuration error: {0}")]
    Tls(#[from] rustls::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("key store error: {0}")]
    KeyStore(String),
}

pub type Result<T> = std::result::Result<T, CryptoError>;

/// Install the ring crypto provider as process default (idempotent).
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
