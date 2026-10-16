//! Mutual TLS with application-level certificate verification.
//!
//! There is no CA: devices use self-signed certificates. Security therefore
//! comes from *pinning*: once two devices are paired, each stores the other's
//! exact certificate and any different certificate is rejected during the
//! handshake. Before pairing the certificate is accepted only if its CN equals
//! the device id announced in the identity packet; such a link is restricted to
//! pairing packets until the user confirms the verification key.
//!
//! Handshake signatures are always verified, so the peer must possess the
//! private key of the presented certificate.

use crate::{LocalIdentity, Result, cert::cert_common_name};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, Error, ServerConfig,
    SignatureScheme,
};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use std::sync::Arc;

/// What we expect the peer to present.
#[derive(Clone, PartialEq, Eq)]
pub enum PeerExpectation {
    /// Paired device: certificate must match byte for byte.
    Pinned {
        device_id: String,
        cert_der: Vec<u8>,
    },
    /// Not (yet) paired: any certificate whose CN is `device_id`.
    Unpaired { device_id: String },
}

impl std::fmt::Debug for PeerExpectation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pinned {
                device_id,
                cert_der,
            } => f
                .debug_struct("Pinned")
                .field("device_id", device_id)
                .field("fingerprint", &crate::cert_fingerprint(cert_der))
                .finish(),
            Self::Unpaired { device_id } => f
                .debug_struct("Unpaired")
                .field("device_id", device_id)
                .finish(),
        }
    }
}

impl PeerExpectation {
    pub fn device_id(&self) -> &str {
        match self {
            Self::Pinned { device_id, .. } | Self::Unpaired { device_id } => device_id,
        }
    }

    fn check(&self, presented: &[u8]) -> std::result::Result<(), Error> {
        match self {
            Self::Pinned { cert_der, .. } => {
                if presented == cert_der.as_slice() {
                    Ok(())
                } else {
                    tracing::warn!(
                        expected = ?self,
                        got = %crate::cert_fingerprint(presented),
                        "certificate pin mismatch: possible MITM or reinstalled device"
                    );
                    Err(Error::InvalidCertificate(
                        CertificateError::ApplicationVerificationFailure,
                    ))
                }
            }
            Self::Unpaired { device_id } => match cert_common_name(presented) {
                Ok(cn) if cn == *device_id => Ok(()),
                _ => Err(Error::InvalidCertificate(CertificateError::NotValidForName)),
            },
        }
    }
}

#[derive(Debug)]
struct Verifier {
    expect: PeerExpectation,
    algs: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, Error> {
        if !intermediates.is_empty() {
            return Err(Error::InvalidCertificate(CertificateError::BadEncoding));
        }
        self.expect.check(end_entity.as_ref())?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

impl ClientCertVerifier for Verifier {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, Error> {
        if !intermediates.is_empty() {
            return Err(Error::InvalidCertificate(CertificateError::BadEncoding));
        }
        self.expect.check(end_entity.as_ref())?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

fn provider() -> Arc<CryptoProvider> {
    // ring provider: only AEAD cipher suites with (EC)DHE key exchange.
    Arc::new(rustls::crypto::ring::default_provider())
}

const VERSIONS: &[&rustls::SupportedProtocolVersion] =
    &[&rustls::version::TLS13, &rustls::version::TLS12];

fn key(local: &LocalIdentity) -> PrivateKeyDer<'static> {
    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(local.key_der().to_vec()))
}

/// TLS client config presenting our certificate and verifying the server
/// against `expect`.
pub fn client_config(local: &LocalIdentity, expect: PeerExpectation) -> Result<Arc<ClientConfig>> {
    let provider = provider();
    let verifier = Arc::new(Verifier {
        expect,
        algs: provider.signature_verification_algorithms,
    });
    let cfg = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(VERSIONS)?
        .dangerous() // custom verifier: pinning instead of WebPKI, see module docs
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(
            vec![CertificateDer::from(local.cert_der.clone())],
            key(local),
        )?;
    Ok(Arc::new(cfg))
}

/// TLS server config requiring a client certificate matching `expect`.
pub fn server_config(local: &LocalIdentity, expect: PeerExpectation) -> Result<Arc<ServerConfig>> {
    let provider = provider();
    let verifier = Arc::new(Verifier {
        expect,
        algs: provider.signature_verification_algorithms,
    });
    let cfg = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(VERSIONS)?
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![CertificateDer::from(local.cert_der.clone())],
            key(local),
        )?;
    Ok(Arc::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_device_id;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::{TlsAcceptor, TlsConnector};

    async fn handshake(
        server_id: &LocalIdentity,
        server_expect: PeerExpectation,
        client_id: &LocalIdentity,
        client_expect: PeerExpectation,
    ) -> bool {
        let (a, b) = tokio::io::duplex(64 * 1024);
        let acceptor = TlsAcceptor::from(server_config(server_id, server_expect).unwrap());
        let connector = TlsConnector::from(client_config(client_id, client_expect).unwrap());
        let server = tokio::spawn(async move {
            let mut s = acceptor.accept(a).await.ok()?;
            let mut buf = [0u8; 5];
            s.read_exact(&mut buf).await.ok()?;
            Some(buf)
        });
        let name = ServerName::IpAddress(std::net::Ipv4Addr::LOCALHOST.into());
        let client = async move {
            let mut c = connector.connect(name, b).await.ok()?;
            c.write_all(b"hello").await.ok()?;
            c.flush().await.ok()?;
            Some(c) // keep the stream open until the server is done
        };
        let (c, s) = tokio::join!(client, server);
        c.is_some() && s.unwrap() == Some(*b"hello")
    }

    fn ids() -> (LocalIdentity, LocalIdentity) {
        (
            LocalIdentity::generate(&generate_device_id()).unwrap(),
            LocalIdentity::generate(&generate_device_id()).unwrap(),
        )
    }

    fn pinned(id: &LocalIdentity) -> PeerExpectation {
        PeerExpectation::Pinned {
            device_id: id.device_id.clone(),
            cert_der: id.cert_der.clone(),
        }
    }

    fn unpaired(id: &LocalIdentity) -> PeerExpectation {
        PeerExpectation::Unpaired {
            device_id: id.device_id.clone(),
        }
    }

    #[tokio::test]
    async fn pinned_mutual_tls_succeeds() {
        let (a, b) = ids();
        assert!(handshake(&a, pinned(&b), &b, pinned(&a)).await);
    }

    #[tokio::test]
    async fn unpaired_requires_matching_cn() {
        let (a, b) = ids();
        assert!(handshake(&a, unpaired(&b), &b, unpaired(&a)).await);
        // Peer claims to be `a` but presents b's cert.
        assert!(!handshake(&a, unpaired(&a), &b, unpaired(&a)).await);
    }

    #[tokio::test]
    async fn mitm_with_same_device_id_is_rejected_when_pinned() {
        let (a, b) = ids();
        // Attacker generates a certificate with b's device id but its own key.
        let evil = LocalIdentity::generate(&b.device_id).unwrap();
        assert!(!handshake(&a, pinned(&b), &evil, pinned(&a)).await);
        assert!(!handshake(&evil, pinned(&a), &a, pinned(&b)).await);
    }

    #[tokio::test]
    async fn client_without_certificate_is_rejected() {
        let (a, b) = ids();
        let (s, c) = tokio::io::duplex(64 * 1024);
        let acceptor = TlsAcceptor::from(server_config(&a, pinned(&b)).unwrap());
        let cfg = ClientConfig::builder_with_provider(provider())
            .with_protocol_versions(VERSIONS)
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Verifier {
                expect: pinned(&a),
                algs: provider().signature_verification_algorithms,
            }))
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(cfg));
        let server = tokio::spawn(async move {
            let mut st = acceptor.accept(s).await.ok()?;
            let mut buf = [0u8; 1];
            st.read_exact(&mut buf).await.ok()
        });
        let name = ServerName::IpAddress(std::net::Ipv4Addr::LOCALHOST.into());
        if let Ok(mut st) = connector.connect(name, c).await {
            let _ = st.write_all(b"x").await;
            let _ = st.flush().await;
        }
        assert!(server.await.unwrap().is_none());
    }
}
