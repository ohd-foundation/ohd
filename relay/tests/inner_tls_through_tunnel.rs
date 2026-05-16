//! Inner-TLS-through-tunnel: end-to-end proof that the relay forwards the
//! consumer↔storage TLS 1.3 handshake (and the cert-pinning check) opaquely.
//!
//! Per `relay/spec/relay-protocol.md` "TLS-through-tunnel", the relay must
//! never terminate TLS — it forwards `DATA`-frame ciphertext only. The
//! storage presents a self-signed cert keyed by its Ed25519 identity key;
//! the consumer pins that cert's SPKI SHA-256 against the value carried in
//! the share artifact and fails closed on mismatch.
//!
//! This test drives a *real* `rustls` TLS 1.3 handshake between a consumer
//! [`rustls::ClientConnection`] and a storage [`rustls::ServerConnection`],
//! but routes **every** byte the two sides exchange through the relay's
//! [`TunnelFrame`] `DATA` encode/decode path — exactly the codec the relay
//! uses to forward bytes on a session. If the relay layer parsed,
//! terminated, or mutated TLS, the handshake would fail. It does not.
//!
//! Three scenarios:
//!
//! 1. **Pin match** — consumer holds the storage's real pin; handshake
//!    completes and application bytes flow end-to-end.
//! 2. **Pin mismatch** — consumer holds a *different* storage's pin; the
//!    handshake aborts (fail closed) during cert verification.
//! 3. **Cert renewal** — the storage renews its 90-day cert under the same
//!    identity key; a consumer holding the *old* pin still completes the
//!    handshake (the pin is SPKI-based, hence renewal-stable).

use std::io::{Read, Write};
use std::sync::Arc;

use bytes::Bytes;
use ohd_h3_helpers::tls_pin::{pinned_client_config, storage_identity_cert, INNER_TLS_ALPN};
use ohd_relay::frame::{FrameType, TunnelFrame};
use rustls::pki_types::ServerName;
use rustls::{ClientConnection, ServerConnection};

const RENDEZVOUS_URL: &str = "relay.example.com/r/quux42rendezvousid";
/// A fixed wall-clock anchor in 2026 for cert validity windows.
const NOW: i64 = 1_770_000_000;
/// Arbitrary session id; the relay tags every DATA frame for a session
/// with the same id.
const SESSION_ID: u32 = 7;

/// Generate a fresh Ed25519 storage identity key in PKCS#8 DER form.
fn fresh_identity_key() -> Vec<u8> {
    rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .expect("generate identity key")
        .serialize_der()
}

/// Build a storage-side `rustls::ServerConnection` whose leaf cert is the
/// self-signed identity cert minted from `identity_key` at time `mint_at`.
fn storage_server(identity_key: &[u8], mint_at: i64) -> ServerConnection {
    let ident = storage_identity_cert(identity_key, RENDEZVOUS_URL, mint_at)
        .expect("mint storage identity cert");
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut config =
        rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_no_client_auth()
            .with_single_cert(ident.cert_chain, ident.key)
            .expect("storage ServerConfig");
    config.alpn_protocols = vec![INNER_TLS_ALPN.to_vec()];
    ServerConnection::new(Arc::new(config)).expect("ServerConnection")
}

/// Build a consumer-side `rustls::ClientConnection` pinned to `pin`.
fn consumer_client(pin: [u8; 32]) -> ClientConnection {
    let config = pinned_client_config(pin).expect("pinned client config");
    // The pinned verifier ignores the SNI; any well-formed name works.
    let sni = ServerName::try_from("ohd-storage").expect("server name");
    ClientConnection::new(Arc::new(config), sni).expect("ClientConnection")
}

/// Move one batch of TLS bytes from `src` to `dst`, *through the relay's
/// `DATA`-frame codec*.
///
/// Whatever `src` wants to send is wrapped in a [`TunnelFrame::data`]
/// envelope, encoded to wire bytes, then decoded back out — the exact
/// round trip the relay performs for a session's bytes. The relay never
/// looks inside the payload; this helper asserts as much. Returns the
/// number of TLS bytes forwarded.
fn relay_forward(src: &mut dyn TlsEndpoint, dst: &mut dyn TlsEndpoint) -> usize {
    let mut tls_bytes = Vec::new();
    src.drain_tls(&mut tls_bytes);
    if tls_bytes.is_empty() {
        return 0;
    }

    // --- The relay's entire job for a session: wrap opaque bytes in a
    // --- DATA frame, forward, never inspect or terminate TLS. We run the
    // --- real codec from `ohd_relay::frame`.
    let frame = TunnelFrame::data(SESSION_ID, Bytes::from(tls_bytes.clone()));
    let wire = frame.encode().expect("relay encodes DATA frame");
    let decoded = TunnelFrame::decode(&wire).expect("relay decodes DATA frame");
    assert_eq!(
        decoded.frame_type,
        FrameType::Data,
        "inner TLS must ride opaque DATA frames"
    );
    assert_eq!(decoded.session_id, SESSION_ID);
    assert_eq!(
        decoded.payload.as_ref(),
        &tls_bytes[..],
        "relay must forward the inner-TLS payload byte-identically"
    );
    // --- End of the relay's job.

    dst.inject_tls(decoded.payload.as_ref());
    tls_bytes.len()
}

/// Run the handshake (and any queued data) to completion by ping-ponging
/// bytes through [`relay_forward`] until neither side has more to send.
/// Returns `Ok(())` once both sides are out of the handshake, or the
/// first TLS error a side raises (the fail-closed path).
fn drive(
    client: &mut ClientConnection,
    server: &mut ServerConnection,
) -> Result<(), rustls::Error> {
    let mut c = ClientEndpoint(client);
    let mut s = ServerEndpoint(server);
    for _ in 0..32 {
        let moved_c2s = relay_forward(&mut c, &mut s);
        s.0.process_new_packets()?;
        let moved_s2c = relay_forward(&mut s, &mut c);
        c.0.process_new_packets()?;
        if moved_c2s == 0 && moved_s2c == 0 {
            return Ok(());
        }
    }
    panic!("handshake did not settle within the iteration budget");
}

// ---------------------------------------------------------------------------
// Thin object-safe wrapper so `relay_forward` works for both directions.
// ---------------------------------------------------------------------------

trait TlsEndpoint {
    /// Append any TLS bytes this side wants to transmit into `out`.
    fn drain_tls(&mut self, out: &mut Vec<u8>);
    /// Feed received TLS bytes into this side.
    fn inject_tls(&mut self, bytes: &[u8]);
}

struct ClientEndpoint<'a>(&'a mut ClientConnection);
struct ServerEndpoint<'a>(&'a mut ServerConnection);

impl TlsEndpoint for ClientEndpoint<'_> {
    fn drain_tls(&mut self, out: &mut Vec<u8>) {
        if self.0.wants_write() {
            self.0.write_tls(out).expect("client write_tls");
        }
    }
    fn inject_tls(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let n = self.0.read_tls(&mut bytes).expect("client read_tls");
            if n == 0 {
                break;
            }
        }
    }
}

impl TlsEndpoint for ServerEndpoint<'_> {
    fn drain_tls(&mut self, out: &mut Vec<u8>) {
        if self.0.wants_write() {
            self.0.write_tls(out).expect("server write_tls");
        }
    }
    fn inject_tls(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let n = self.0.read_tls(&mut bytes).expect("server read_tls");
            if n == 0 {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Test 1: pin match — handshake completes, app data flows end-to-end.
// ---------------------------------------------------------------------------

#[test]
fn inner_tls_pin_match_completes_handshake_through_relay() {
    let identity_key = fresh_identity_key();
    let ident = storage_identity_cert(&identity_key, RENDEZVOUS_URL, NOW).unwrap();

    let mut server = storage_server(&identity_key, NOW);
    // The consumer pins the storage's real SPKI hash — the value the
    // share artifact would carry.
    let mut client = consumer_client(ident.spki_sha256);

    drive(&mut client, &mut server).expect("handshake should complete on pin match");
    assert!(!client.is_handshaking(), "client handshake incomplete");
    assert!(!server.is_handshaking(), "server handshake incomplete");

    // ALPN: the inner session negotiated the inner-TLS protocol id, all
    // end-to-end — the relay never saw it.
    assert_eq!(client.alpn_protocol(), Some(INNER_TLS_ALPN));

    // Application bytes flow over the established inner-TLS session,
    // still relayed as opaque DATA frames.
    let mut c = ClientEndpoint(&mut client);
    let mut s = ServerEndpoint(&mut server);
    c.0.writer().write_all(b"GET /tools/list").unwrap();
    relay_forward(&mut c, &mut s);
    s.0.process_new_packets().unwrap();

    let mut got = Vec::new();
    s.0.reader().read_to_end(&mut got).ok();
    assert_eq!(&got, b"GET /tools/list", "app payload survived the relay");
}

// ---------------------------------------------------------------------------
// Test 2: pin mismatch — handshake fails closed during cert verification.
// ---------------------------------------------------------------------------

#[test]
fn inner_tls_pin_mismatch_fails_closed_through_relay() {
    // Storage A is the real endpoint. Storage B is a different identity
    // key — its pin is what the consumer (wrongly) holds.
    let identity_a = fresh_identity_key();
    let identity_b = fresh_identity_key();
    let pin_b = storage_identity_cert(&identity_b, RENDEZVOUS_URL, NOW)
        .unwrap()
        .spki_sha256;

    let mut server = storage_server(&identity_a, NOW);
    let mut client = consumer_client(pin_b);

    let result = drive(&mut client, &mut server);
    let err = result.expect_err("pin mismatch must abort the handshake");
    match err {
        rustls::Error::InvalidCertificate(
            rustls::CertificateError::ApplicationVerificationFailure,
        ) => {}
        other => panic!("expected fail-closed pin rejection, got {other:?}"),
    }
    assert!(
        client.is_handshaking(),
        "client must not consider the session established on pin mismatch"
    );
}

// ---------------------------------------------------------------------------
// Test 3: cert renewal under the same identity key keeps the pin valid.
// ---------------------------------------------------------------------------

#[test]
fn inner_tls_cert_renewal_keeps_old_pin_valid_through_relay() {
    let identity_key = fresh_identity_key();
    // The original cert + the pin baked into a share artifact months ago.
    let original_pin = storage_identity_cert(&identity_key, RENDEZVOUS_URL, NOW)
        .unwrap()
        .spki_sha256;

    // 80 days later the storage renews its 90-day cert ahead of expiry,
    // under the SAME identity key. A brand-new certificate — but the
    // consumer still holds `original_pin`.
    let renewed_at = NOW + 80 * 86_400;
    let mut server = storage_server(&identity_key, renewed_at);
    let mut client = consumer_client(original_pin);

    drive(&mut client, &mut server)
        .expect("old pin must still accept the renewed cert");
    assert!(!client.is_handshaking(), "renewed-cert handshake incomplete");
    assert!(!server.is_handshaking(), "renewed-cert handshake incomplete");
}
