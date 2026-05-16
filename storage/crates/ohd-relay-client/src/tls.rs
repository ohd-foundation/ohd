//! TLS verifiers for the relay QUIC tunnel.
//!
//! The relay's tunnel cert is operator-supplied. Three modes (see the
//! module docs in [`crate::tunnel`]):
//!
//! - **Pinned**: verify the leaf cert's SHA-256 matches an operator pin.
//! - **Webpki + native trust**: verify against the OS trust store via
//!   [`rustls_platform_verifier`].
//! - **Insecure** (dev only): accept any cert.
//!
//! This module is portable — `rustls` (ring) + `rustls-platform-verifier`
//! both cross-compile for the Android targets.

use std::sync::Arc;

use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};

/// Insecure verifier — accepts any cert. Dev only.
#[derive(Debug)]
pub struct InsecureCertVerifier {
    provider: rustls::crypto::CryptoProvider,
}

impl InsecureCertVerifier {
    pub fn new() -> Self {
        Self {
            provider: rustls::crypto::ring::default_provider(),
        }
    }
}

impl Default for InsecureCertVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl rustls::client::danger::ServerCertVerifier for InsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Pinned verifier — accepts a cert iff its SHA-256 hash matches the pin.
///
/// Sidesteps the WebPKI machinery for an operator we already trust by way
/// of registration. The pin is delivered out-of-band.
#[derive(Debug)]
pub struct SpkiPinVerifier {
    pin: [u8; 32],
    provider: rustls::crypto::CryptoProvider,
}

impl SpkiPinVerifier {
    /// Build a pin verifier. `pin` must be 32 bytes (SHA-256).
    pub fn new(pin: Vec<u8>) -> anyhow::Result<Self> {
        if pin.len() != 32 {
            anyhow::bail!(
                "expected_relay_pubkey_pin must be 32 bytes (SHA-256), got {}",
                pin.len()
            );
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&pin);
        Ok(Self {
            pin: arr,
            provider: rustls::crypto::ring::default_provider(),
        })
    }
}

impl rustls::client::danger::ServerCertVerifier for SpkiPinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        // Hash the entire DER-encoded leaf cert. Matches the common "cert
        // pin" convention; swappable for a true SPKI pin without a wire
        // format change.
        let mut hasher = Sha256::new();
        hasher.update(end_entity.as_ref());
        let got = hasher.finalize();
        // Constant-time compare.
        let mut acc = 0u8;
        for i in 0..32 {
            acc |= got[i] ^ self.pin[i];
        }
        if acc == 0 {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Build the rustls `ClientConfig` for the QUIC tunnel handshake, picking
/// the verifier per [`crate::tunnel::RelayClientOptions`].
pub(crate) fn build_client_tls_config(
    allow_insecure_dev: bool,
    expected_relay_pubkey_pin: Option<Vec<u8>>,
    alpn: &[u8],
) -> anyhow::Result<rustls::ClientConfig> {
    // Install ring as the rustls default crypto provider if none is
    // registered. Unconditional call is safe — subsequent calls no-op.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut tls = if allow_insecure_dev {
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureCertVerifier::new()))
            .with_no_client_auth()
    } else if let Some(pin) = expected_relay_pubkey_pin {
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SpkiPinVerifier::new(pin)?))
            .with_no_client_auth()
    } else {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = rustls_platform_verifier::Verifier::new(provider)
            .map_err(|e| anyhow::anyhow!("rustls-platform-verifier init: {e}"))?;
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth()
    };
    tls.alpn_protocols = vec![alpn.to_vec()];
    Ok(tls)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::client::danger::ServerCertVerifier;

    #[test]
    fn pin_verifier_rejects_wrong_pin() {
        let pin = [0u8; 32];
        let v = SpkiPinVerifier::new(pin.to_vec()).unwrap();
        let cert = CertificateDer::from(vec![0x30, 0x82, 0x00, 0x01, 0xAA]);
        let res = v.verify_server_cert(
            &cert,
            &[],
            &ServerName::try_from("localhost").unwrap(),
            &[],
            UnixTime::since_unix_epoch(std::time::Duration::from_secs(0)),
        );
        assert!(res.is_err());
    }

    #[test]
    fn pin_verifier_accepts_matching_pin() {
        let cert_der = vec![0x30, 0x82, 0x00, 0x01, 0xAA, 0xBB];
        let mut h = Sha256::new();
        h.update(&cert_der);
        let pin = h.finalize().to_vec();
        let v = SpkiPinVerifier::new(pin).unwrap();
        let cert = CertificateDer::from(cert_der);
        let res = v.verify_server_cert(
            &cert,
            &[],
            &ServerName::try_from("localhost").unwrap(),
            &[],
            UnixTime::since_unix_epoch(std::time::Duration::from_secs(0)),
        );
        assert!(res.is_ok());
    }

    #[test]
    fn pin_verifier_rejects_wrong_length() {
        assert!(SpkiPinVerifier::new(vec![0u8; 16]).is_err());
        assert!(SpkiPinVerifier::new(vec![0u8; 33]).is_err());
    }

    #[test]
    fn build_config_insecure_ok() {
        let cfg = build_client_tls_config(true, None, b"ohd-tnl1").unwrap();
        assert_eq!(cfg.alpn_protocols, vec![b"ohd-tnl1".to_vec()]);
    }

    #[test]
    fn build_config_pin_ok() {
        let cfg = build_client_tls_config(false, Some(vec![0u8; 32]), b"ohd-tnl1").unwrap();
        assert_eq!(cfg.alpn_protocols, vec![b"ohd-tnl1".to_vec()]);
    }
}
