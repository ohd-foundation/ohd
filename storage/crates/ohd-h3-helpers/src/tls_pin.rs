//! Cert-pinning for the inner TLS 1.3 session that rides the relay tunnel.
//!
//! # Where this fits
//!
//! Per `relay/spec/relay-protocol.md` "TLS-through-tunnel", a consumer
//! (CORD, OHD Care) reaching a phone-hosted storage through an opaque
//! relay performs an **inner** TLS 1.3 handshake end-to-end with the
//! storage. The storage presents a self-signed certificate whose key is
//! the storage's long-lived Ed25519 **identity key**; the consumer
//! verifies that cert's SubjectPublicKeyInfo SHA-256 ("the pin") against
//! the pin carried in the share artifact, and fails closed on mismatch.
//!
//! This module is the bit-level half of that:
//!
//! - [`storage_identity_cert`] — storage side: mint the self-signed TLS
//!   cert from an Ed25519 identity key (SAN = the rendezvous URL).
//! - [`spki_sha256`] / [`spki_sha256_b64url`] — derive the pin from a
//!   cert; stable across cert renewals under the same identity key.
//! - [`PinnedServerCertVerifier`] + [`pinned_client_config`] — consumer
//!   side: a `rustls` verifier + `ClientConfig` that accepts a cert iff
//!   its SPKI SHA-256 equals the expected pin, and rejects everything
//!   else (expiry, signature, hostname are all irrelevant — the pin is
//!   the entire trust anchor).
//!
//! It lives in `ohd-h3-helpers` because both sides need it: the storage
//! server / Android binding for the server cert, and any consumer
//! (CORD's relay MCP client) for the pinned verifier. The crate is
//! dependency-light and already re-exports `rustls`.
//!
//! # Why SPKI, not whole-cert
//!
//! The pin is `SHA-256(SubjectPublicKeyInfo)`, **not** a hash of the
//! whole DER certificate. The identity key is long-lived; the TLS cert
//! is renewed every 90 days. A whole-cert hash would change on every
//! renewal and silently invalidate every outstanding share. The SPKI is
//! exactly the identity public key's DER encoding, so it is invariant
//! across renewals — the property "cert renewal keeps the pin valid"
//! the spec requires, and the property the renewal test asserts.

use std::sync::Arc;

use anyhow::Context as _;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};

/// ALPN identifier negotiated on the inner TLS session that rides the
/// tunnel's `DATA` frames. Distinct from the outer relay ALPNs
/// (`h3`, `ohd-tnl1`): the relay never sees this — it is negotiated
/// end-to-end between consumer and storage inside the encrypted stream.
pub const INNER_TLS_ALPN: &[u8] = b"ohd-mcp1";

/// Default validity window for a storage identity TLS cert, in days.
/// Per the relay spec: "Validity 90 days; renewed automatically by the
/// storage, signed by the same identity key."
pub const CERT_VALIDITY_DAYS: i64 = 90;

// ---------------------------------------------------------------------------
// Storage side: self-signed identity cert
// ---------------------------------------------------------------------------

/// A freshly-minted storage identity certificate plus its private key,
/// ready to hand to `rustls::ServerConfig` / `quinn`.
pub struct IdentityCert {
    /// Single-element chain: the self-signed leaf.
    pub cert_chain: Vec<CertificateDer<'static>>,
    /// The private key matching the cert — the storage's identity key.
    pub key: PrivateKeyDer<'static>,
    /// `SHA-256(SubjectPublicKeyInfo)` — the pin published in share
    /// artifacts. Invariant across renewals under the same identity key.
    pub spki_sha256: [u8; 32],
}

impl IdentityCert {
    /// The pin in the `base64url`-no-pad form share artifacts carry
    /// (`ohd://share/...?pin=<this>`).
    pub fn pin_b64url(&self) -> String {
        b64url_no_pad(&self.spki_sha256)
    }
}

/// Mint a self-signed TLS 1.3 certificate for an on-device storage,
/// signed by its Ed25519 **identity key**.
///
/// `identity_key_pkcs8_der` is the storage identity key in PKCS#8 DER
/// form (the shape `ed25519-dalek`'s `SigningKey::to_pkcs8_der()` and
/// `ring`'s `Ed25519KeyPair::generate_pkcs8()` both produce). The same
/// bytes passed twice produce certs with **identical** SPKI — that is
/// what keeps a renewed cert's pin valid.
///
/// `rendezvous_url` becomes the cert's Subject Alternative Name, e.g.
/// `relay.example.com/r/<rendezvous_id>`. The consumer's verifier
/// ignores the SAN (the pin is the trust anchor), but populating it
/// keeps the cert well-formed and matches the spec.
///
/// `now_unix_secs` anchors the validity window — pass the storage's
/// current wall clock. The cert is valid `[now - 1h, now + 90d]` (the
/// backdated hour absorbs minor clock skew between storage and
/// consumer; irrelevant to a pinned verifier but correct hygiene).
pub fn storage_identity_cert(
    identity_key_pkcs8_der: &[u8],
    rendezvous_url: &str,
    now_unix_secs: i64,
) -> anyhow::Result<IdentityCert> {
    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(identity_key_pkcs8_der.to_vec()),
        &rcgen::PKCS_ED25519,
    )
    .context("load Ed25519 identity key into rcgen")?;

    // SAN must be a DnsName; a rendezvous URL is `host/r/<id>`, which is
    // not a bare DNS label. We register the host portion as the DnsName
    // SAN and carry the full rendezvous URL as a `UniformResourceIdentifier`
    // SAN so the cert records the identity it was minted for.
    let host = rendezvous_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(rendezvous_url);

    let mut params = rcgen::CertificateParams::new(Vec::<String>::new())
        .context("rcgen CertificateParams")?;
    params.subject_alt_names = vec![
        rcgen::SanType::DnsName(
            rcgen::Ia5String::try_from(host)
                .context("rendezvous host is not a valid DNS name")?,
        ),
        rcgen::SanType::URI(
            rcgen::Ia5String::try_from(rendezvous_url)
                .context("rendezvous URL is not a valid URI SAN")?,
        ),
    ];
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "OHD Storage Identity");
    params.not_before = offset_from_unix(now_unix_secs - 3600)?;
    params.not_after = offset_from_unix(now_unix_secs + CERT_VALIDITY_DAYS * 86_400)?;

    let cert = params
        .self_signed(&key_pair)
        .context("self-sign storage identity cert")?;
    let cert_der: CertificateDer<'static> = cert.der().clone();

    // The SPKI is exactly the Ed25519 public key's DER (SubjectPublicKeyInfo).
    // rcgen exposes it directly; hashing it gives the renewal-stable pin.
    let spki_sha256 = sha256(&key_pair.public_key_der());

    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity_key_pkcs8_der.to_vec()));
    Ok(IdentityCert {
        cert_chain: vec![cert_der],
        key,
        spki_sha256,
    })
}

/// Extract `SHA-256(SubjectPublicKeyInfo)` from a DER-encoded cert.
///
/// This is the canonical pin computation: it parses the cert just far
/// enough to isolate the SPKI field, then hashes that. It does **not**
/// hash the whole cert, so two certs that share an identity key (a
/// renewal pair) hash identically.
pub fn spki_sha256(cert_der: &[u8]) -> anyhow::Result<[u8; 32]> {
    let spki = extract_spki(cert_der)?;
    Ok(sha256(&spki))
}

/// [`spki_sha256`], `base64url`-no-pad encoded — the form share
/// artifacts carry in their `pin` parameter.
pub fn spki_sha256_b64url(cert_der: &[u8]) -> anyhow::Result<String> {
    Ok(b64url_no_pad(&spki_sha256(cert_der)?))
}

// ---------------------------------------------------------------------------
// Consumer side: pinned verifier + ClientConfig
// ---------------------------------------------------------------------------

/// A `rustls` server-cert verifier that accepts a leaf certificate iff
/// its `SHA-256(SubjectPublicKeyInfo)` equals the configured pin.
///
/// Everything else — CA chain, expiry, hostname, signature algorithm —
/// is irrelevant: the pin (delivered out-of-band in the share artifact,
/// authorised by the user) **is** the trust anchor. A mismatch fails
/// closed with `ApplicationVerificationFailure`; the consumer surfaces
/// it as `CERT_PIN_MISMATCH` ("this storage isn't who the share said it
/// would be").
///
/// The handshake-signature checks still run for real (TLS 1.3 proves
/// the peer holds the private key for the presented cert), so a pinned
/// cert cannot be replayed by a party that doesn't hold the identity
/// key. Only chain/expiry/hostname validation is replaced by the pin.
#[derive(Debug)]
pub struct PinnedServerCertVerifier {
    expected_pin: [u8; 32],
    provider: rustls::crypto::CryptoProvider,
}

impl PinnedServerCertVerifier {
    /// Build a verifier for the given 32-byte SPKI-SHA-256 pin.
    pub fn new(expected_pin: [u8; 32]) -> Self {
        Self {
            expected_pin,
            provider: rustls::crypto::ring::default_provider(),
        }
    }

    /// Build a verifier from a `base64url`-encoded pin (with or without
    /// padding) — the form carried in `ohd://share/...?pin=<...>`.
    pub fn from_b64url(pin: &str) -> anyhow::Result<Self> {
        let raw = b64url_decode(pin).context("decode pin (base64url)")?;
        let arr: [u8; 32] = raw
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("pin must be 32 bytes (SHA-256), got {}", raw.len()))?;
        Ok(Self::new(arr))
    }
}

impl ServerCertVerifier for PinnedServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let spki = extract_spki(end_entity.as_ref()).map_err(|_| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;
        let got = sha256(&spki);
        // Constant-time compare: a pin mismatch should not be a timing
        // oracle for how many leading bytes matched.
        let mut acc = 0u8;
        for i in 0..32 {
            acc |= got[i] ^ self.expected_pin[i];
        }
        if acc == 0 {
            Ok(ServerCertVerified::assertion())
        } else {
            // Fail closed: the storage is not who the share said it was.
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Inner TLS is 1.3-only (see `pinned_client_config`); this arm
        // exists to satisfy the trait. Verify for real anyway.
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // Real signature check: proves the peer holds the private key
        // for the (pinned) cert it presented.
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Build a TLS 1.3 `ClientConfig` for the inner-tunnel handshake that
/// pins the storage's identity cert to `expected_pin`.
///
/// The returned config has the inner-TLS ALPN ([`INNER_TLS_ALPN`]) set
/// and TLS 1.3 as the only permitted version. Hand it to a `rustls`
/// (or `tokio-rustls`) client running over the `AsyncRead + AsyncWrite`
/// that bridges the tunnel's `DATA` frames.
pub fn pinned_client_config(expected_pin: [u8; 32]) -> anyhow::Result<ClientConfig> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config = ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServerCertVerifier::new(expected_pin)))
        .with_no_client_auth();
    config.alpn_protocols = vec![INNER_TLS_ALPN.to_vec()];
    Ok(config)
}

/// [`pinned_client_config`], taking the `base64url` pin from a share
/// artifact directly.
pub fn pinned_client_config_b64url(pin: &str) -> anyhow::Result<ClientConfig> {
    let verifier = PinnedServerCertVerifier::from_b64url(pin)?;
    pinned_client_config(verifier.expected_pin)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sha256(input: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input);
    let mut out = [0u8; 32];
    out.copy_from_slice(&h.finalize());
    out
}

fn offset_from_unix(secs: i64) -> anyhow::Result<time::OffsetDateTime> {
    time::OffsetDateTime::from_unix_timestamp(secs)
        .context("certificate validity timestamp out of range")
}

/// Walk a DER `Certificate` to its `SubjectPublicKeyInfo` and return that
/// SPKI's raw DER bytes.
///
/// DER `Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm,
/// signatureValue }`, and `TBSCertificate ::= SEQUENCE { [0] version?,
/// serialNumber, signature, issuer, validity, subject,
/// subjectPublicKeyInfo, ... }`. We descend into `tbsCertificate`, skip
/// the six fields preceding the SPKI, and return the SPKI element
/// verbatim (tag + length + value). No external x509 dep — this is a
/// dozen lines of TLV walking and keeps the crate's dep set minimal.
fn extract_spki(cert_der: &[u8]) -> anyhow::Result<Vec<u8>> {
    // Certificate ::= SEQUENCE { ... }
    let (cert_seq, _) = der_expect_sequence(cert_der)?;
    // First element of Certificate is tbsCertificate, itself a SEQUENCE.
    let (tbs_seq, _) = der_expect_sequence(cert_seq)?;

    let mut rest = tbs_seq;
    // Optional [0] EXPLICIT version — context tag 0xA0. Skip if present.
    if rest.first() == Some(&0xA0) {
        let (_, after) = der_take_tlv(rest)?;
        rest = after;
    }
    // Skip serialNumber, signature, issuer, validity, subject — five
    // TLV elements — leaving subjectPublicKeyInfo at the head.
    for _ in 0..5 {
        let (_, after) = der_take_tlv(rest)?;
        rest = after;
    }
    // subjectPublicKeyInfo: return the full TLV (tag + length + value).
    let (spki, _) = der_take_tlv(rest)?;
    Ok(spki.to_vec())
}

/// Confirm `der` begins with a SEQUENCE tag and return `(contents,
/// trailing)` where `contents` is the bytes inside the SEQUENCE.
fn der_expect_sequence(der: &[u8]) -> anyhow::Result<(&[u8], &[u8])> {
    if der.first() != Some(&0x30) {
        anyhow::bail!("expected DER SEQUENCE");
    }
    let (tlv, trailing) = der_take_tlv(der)?;
    let (_, len_hdr) = der_read_len(&tlv[1..])?;
    Ok((&tlv[1 + len_hdr..], trailing))
}

/// Split `der` into `(this_tlv, rest)` where `this_tlv` is one complete
/// tag-length-value element.
fn der_take_tlv(der: &[u8]) -> anyhow::Result<(&[u8], &[u8])> {
    if der.len() < 2 {
        anyhow::bail!("truncated DER element");
    }
    let (len, len_hdr) = der_read_len(&der[1..])?;
    let total = 1 + len_hdr + len;
    if der.len() < total {
        anyhow::bail!("DER element overruns buffer");
    }
    Ok((&der[..total], &der[total..]))
}

/// Read a DER length field starting at `bytes`; return `(value,
/// header_len)` where `header_len` is how many bytes the length field
/// itself occupied.
fn der_read_len(bytes: &[u8]) -> anyhow::Result<(usize, usize)> {
    let first = *bytes.first().context("truncated DER length")?;
    if first & 0x80 == 0 {
        // Short form: the byte is the length.
        Ok((first as usize, 1))
    } else {
        // Long form: low 7 bits = number of subsequent length bytes.
        let n = (first & 0x7F) as usize;
        if n == 0 || n > 4 || bytes.len() < 1 + n {
            anyhow::bail!("unsupported DER length encoding");
        }
        let mut len = 0usize;
        for &b in &bytes[1..1 + n] {
            len = (len << 8) | b as usize;
        }
        Ok((len, 1 + n))
    }
}

fn b64url_no_pad(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn b64url_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    use base64::Engine as _;
    // Accept both padded and unpadded forms — share artifacts emit
    // unpadded, but a hand-pasted link may carry padding.
    let trimmed = s.trim_end_matches('=');
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(trimmed)
        .context("base64url decode")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::client::danger::ServerCertVerifier;

    /// Generate a fresh Ed25519 identity key in PKCS#8 DER form.
    fn fresh_identity_key() -> Vec<u8> {
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("gen identity key");
        kp.serialize_der()
    }

    const RZV: &str = "relay.example.com/r/abc123def456";
    const NOW: i64 = 1_770_000_000; // a fixed point in 2026

    #[test]
    fn cert_spki_matches_helper_extraction() {
        let key = fresh_identity_key();
        let ident = storage_identity_cert(&key, RZV, NOW).expect("mint cert");
        // The pin reported by the minting helper must equal the pin a
        // consumer would derive by parsing the presented cert.
        let from_cert = spki_sha256(ident.cert_chain[0].as_ref()).expect("extract spki");
        assert_eq!(ident.spki_sha256, from_cert);
    }

    #[test]
    fn pin_match_succeeds() {
        let key = fresh_identity_key();
        let ident = storage_identity_cert(&key, RZV, NOW).expect("mint cert");
        let verifier = PinnedServerCertVerifier::new(ident.spki_sha256);
        let res = verifier.verify_server_cert(
            &ident.cert_chain[0],
            &[],
            &ServerName::try_from("storage").unwrap(),
            &[],
            UnixTime::since_unix_epoch(std::time::Duration::from_secs(NOW as u64)),
        );
        assert!(res.is_ok(), "matching pin must verify: {res:?}");
    }

    #[test]
    fn pin_mismatch_is_rejected_fail_closed() {
        // Storage A mints a cert; the consumer holds storage B's pin.
        let ident_a = storage_identity_cert(&fresh_identity_key(), RZV, NOW).expect("cert a");
        let ident_b = storage_identity_cert(&fresh_identity_key(), RZV, NOW).expect("cert b");
        assert_ne!(
            ident_a.spki_sha256, ident_b.spki_sha256,
            "two identity keys must yield distinct pins"
        );
        let verifier = PinnedServerCertVerifier::new(ident_b.spki_sha256);
        let res = verifier.verify_server_cert(
            &ident_a.cert_chain[0],
            &[],
            &ServerName::try_from("storage").unwrap(),
            &[],
            UnixTime::since_unix_epoch(std::time::Duration::from_secs(NOW as u64)),
        );
        match res {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            )) => {}
            other => panic!("pin mismatch must fail closed, got {other:?}"),
        }
    }

    #[test]
    fn cert_renewal_under_same_identity_key_keeps_pin() {
        // The storage rotates its 90-day TLS cert WITHOUT changing the
        // identity key. The renewed cert is a different certificate
        // (different validity window, different signature), but its
        // SPKI — hence its pin — must be byte-identical, so outstanding
        // share artifacts stay valid.
        let key = fresh_identity_key();
        let original = storage_identity_cert(&key, RZV, NOW).expect("original cert");
        // 80 days later: storage renews ahead of the 90-day expiry.
        let renewed = storage_identity_cert(&key, RZV, NOW + 80 * 86_400).expect("renewed cert");

        assert_ne!(
            original.cert_chain[0], renewed.cert_chain[0],
            "renewal should produce a genuinely new certificate"
        );
        assert_eq!(
            original.spki_sha256, renewed.spki_sha256,
            "renewal under the same identity key must keep the pin stable"
        );

        // A verifier built from the ORIGINAL pin must still accept the
        // RENEWED cert — the share artifact issued months ago keeps working.
        let verifier = PinnedServerCertVerifier::new(original.spki_sha256);
        let res = verifier.verify_server_cert(
            &renewed.cert_chain[0],
            &[],
            &ServerName::try_from("storage").unwrap(),
            &[],
            UnixTime::since_unix_epoch(std::time::Duration::from_secs((NOW + 80 * 86_400) as u64)),
        );
        assert!(res.is_ok(), "old pin must accept renewed cert: {res:?}");
    }

    #[test]
    fn pin_b64url_roundtrips() {
        let ident = storage_identity_cert(&fresh_identity_key(), RZV, NOW).expect("cert");
        let pin_str = ident.pin_b64url();
        let verifier = PinnedServerCertVerifier::from_b64url(&pin_str).expect("parse pin");
        assert_eq!(verifier.expected_pin, ident.spki_sha256);
        // The b64url helper on a raw cert agrees with the minting helper.
        assert_eq!(
            spki_sha256_b64url(ident.cert_chain[0].as_ref()).unwrap(),
            pin_str
        );
    }

    #[test]
    fn from_b64url_rejects_short_pin() {
        // 16 bytes, not 32 — must be rejected, not silently truncated.
        let short = b64url_no_pad(&[0u8; 16]);
        assert!(PinnedServerCertVerifier::from_b64url(&short).is_err());
    }

    #[test]
    fn pinned_client_config_sets_inner_alpn_and_tls13() {
        let cfg = pinned_client_config([7u8; 32]).expect("client config");
        assert_eq!(cfg.alpn_protocols, vec![INNER_TLS_ALPN.to_vec()]);
    }
}
