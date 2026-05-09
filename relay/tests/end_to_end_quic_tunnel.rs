//! End-to-end test for the raw QUIC tunnel mode.
//!
//! Three scenarios:
//!
//! 1. **Roundtrip**: spin up the tunnel listener, register a storage, dial
//!    via `quinn::Endpoint`, perform the handshake, open a per-session
//!    stream, push 4 KiB of `TunnelFrame`s in both directions, verify
//!    byte-identical delivery.
//! 2. **Migration**: same setup, but after a few frames have flowed,
//!    `Endpoint::rebind()` the client to a fresh local UDP socket
//!    (simulating a phone WiFi↔cellular handoff) and assert the
//!    connection survives + bytes continue to flow.
//! 3. **Reject**: bad credentials → connection closes with
//!    `REGISTRATION_REJECTED` (close-code 1).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use ohd_relay::frame::{FrameType, TunnelFrame};
use ohd_relay::quic_tunnel::{
    self, close_code, HANDSHAKE_MAX_CRED_LEN, HANDSHAKE_VERSION, STREAM_TAG_SESSION_OPEN,
    TUNNEL_ALPN,
};
use ohd_relay::server::{generate_credential, generate_rendezvous_id};
use ohd_relay::session::attached_senders_for;
use ohd_relay::state::{now_ms, RegistrationRow, RelayState};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tokio::sync::watch;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Spin up a QUIC tunnel listener bound to a fresh ephemeral UDP port.
/// Returns `(bind_addr, relay_state, shutdown_tx, listener_task)`.
///
/// We bind via `quinn::Endpoint::server` directly rather than going
/// through `serve_quic_tunnel` so we don't have a "free the port then
/// rebind" race between tests (parallel tests on the same loopback can
/// otherwise grab each other's ports). We then expose the bound addr.
async fn spawn_tunnel(
) -> (
    SocketAddr,
    RelayState,
    watch::Sender<bool>,
    tokio::task::JoinHandle<()>,
) {
    let relay = RelayState::in_memory().await.unwrap();

    let (cert_chain, key) = ohd_relay::http3::dev_self_signed_cert().unwrap();
    let qcfg = quic_tunnel::server_config(cert_chain, key).unwrap();
    let endpoint =
        quinn::Endpoint::server(qcfg, "127.0.0.1:0".parse().unwrap()).expect("listen");
    let bind_addr = endpoint.local_addr().unwrap();

    let (tx, rx) = watch::channel(false);
    let st = Arc::new(relay.clone());
    // Drive the accept loop manually so we can pin a known endpoint.
    let task = tokio::spawn(async move {
        let mut shutdown = rx;
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    // changed() returns Err when the Sender is dropped; in
                    // either case (true value OR sender dropped), exit the
                    // loop to avoid spinning.
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                incoming = endpoint.accept() => {
                    let Some(incoming) = incoming else { break };
                    let st = st.clone();
                    let sd = shutdown.clone();
                    tokio::spawn(async move {
                        let _ = quic_tunnel::handle_connection_for_test(incoming, st, sd).await;
                    });
                }
            }
        }
        endpoint.close(0u32.into(), b"shutdown");
    });

    (bind_addr, relay, tx, task)
}

async fn register_storage(relay: &RelayState) -> (String, String) {
    let rid = generate_rendezvous_id();
    let cred = generate_credential();
    let cred_hash = sha256_32(cred.as_bytes());
    let row = RegistrationRow {
        rendezvous_id: rid.clone(),
        user_ulid: [1u8; 16],
        push_token: None,
        last_heartbeat_at_ms: now_ms(),
        long_lived_credential_hash: cred_hash,
        registered_at_ms: now_ms(),
        user_label: Some("quic-test".into()),
        storage_pubkey: vec![0xAB; 32],
        oidc_iss: None,
        oidc_sub: None,
    };
    relay.registrations.register(row).await.unwrap();
    (rid, cred)
}

fn sha256_32(input: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(input);
    let r = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&r);
    out
}

/// Build a quinn client endpoint with a fresh client config that accepts
/// any cert (matches `dev_self_signed_cert()` on the relay side) and
/// advertises `TUNNEL_ALPN`.
fn build_client_endpoint(local: SocketAddr) -> quinn::Endpoint {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut tls = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InsecureVerifier {
            provider: rustls::crypto::ring::default_provider(),
        }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![TUNNEL_ALPN.to_vec()];
    let quic_client_cfg: quinn::crypto::rustls::QuicClientConfig =
        tls.try_into().expect("ClientConfig → QuicClientConfig");
    let mut client_cfg = quinn::ClientConfig::new(Arc::new(quic_client_cfg));
    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(Duration::from_secs(15)));
    // Allow the relay to open server-initiated bidi streams (one per
    // consumer attach). Without this, accept_bi on the client side hangs
    // because the peer's stream-id allocation is gated.
    transport.max_concurrent_bidi_streams(quinn::VarInt::from_u32(64));
    client_cfg.transport_config(Arc::new(transport));

    let mut endpoint = quinn::Endpoint::client(local).unwrap();
    endpoint.set_default_client_config(client_cfg);
    endpoint
}

/// Connect to the relay, run the handshake. Returns the connection +
/// (control_send, control_recv).
async fn dial_and_handshake(
    endpoint: &quinn::Endpoint,
    addr: SocketAddr,
    rid: &str,
    cred: &str,
) -> (
    quinn::Connection,
    quinn::SendStream,
    quinn::RecvStream,
    [u8; 16],
) {
    let conn = endpoint
        .connect(addr, "localhost")
        .expect("connect")
        .await
        .expect("handshake");

    let (mut send, mut recv) = conn.open_bi().await.expect("open_bi");
    let mut prefix = vec![HANDSHAKE_VERSION];
    let cred_bytes = cred.as_bytes();
    assert!(cred_bytes.len() <= HANDSHAKE_MAX_CRED_LEN);
    prefix.push(cred_bytes.len() as u8);
    prefix.extend_from_slice(cred_bytes);
    let token = rid.as_bytes();
    let token_len = token.len() as u16;
    prefix.extend_from_slice(&token_len.to_be_bytes());
    prefix.extend_from_slice(token);
    send.write_all(&prefix).await.expect("write handshake");

    let mut ack = [0u8; 1 + 16];
    recv.read_exact(&mut ack).await.expect("read ack");
    assert_eq!(ack[0], 0, "handshake should be accepted");
    let mut session_base_id = [0u8; 16];
    session_base_id.copy_from_slice(&ack[1..]);
    (conn, send, recv, session_base_id)
}

/// A fake "consumer" that uses the relay's existing TunnelEndpoint to
/// open a session, then exchanges DATA frames with the QUIC-backed
/// storage. This bypasses the WS attach path entirely; we drive the
/// relay's session machinery directly.
async fn open_session_against_tunnel(
    relay: &RelayState,
    rid: &str,
) -> (u32, mpsc::Receiver<Bytes>) {
    // Wait until the tunnel registers itself.
    let endpoint = loop {
        if let Some(e) = relay.sessions.lookup(rid).await {
            break e;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let session_id = endpoint.next_session_id();
    let (tx, rx) = mpsc::channel::<Bytes>(64);
    let senders = attached_senders_for(&endpoint);
    senders.write().await.insert(session_id, tx);
    // Send OPEN to storage.
    let frame = TunnelFrame::open(session_id, Bytes::new());
    endpoint.outbound_tx.send(frame).await.unwrap();
    (session_id, rx)
}

/// Send a consumer→storage DATA frame via the tunnel's outbound queue.
async fn consumer_send_data(relay: &RelayState, rid: &str, session_id: u32, payload: Bytes) {
    let endpoint = relay.sessions.lookup(rid).await.unwrap();
    let frame = TunnelFrame::data(session_id, payload);
    endpoint.outbound_tx.send(frame).await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 1: roundtrip
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn quic_tunnel_4kib_roundtrip() {
    let (bind_addr, relay, shutdown_tx, task) = spawn_tunnel().await;
    let (rid, cred) = register_storage(&relay).await;

    let endpoint = build_client_endpoint(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0));
    let (conn, _ctrl_send, _ctrl_recv, _session_base) =
        dial_and_handshake(&endpoint, bind_addr, &rid, &cred).await;

    // Storage role: accept a new bidi stream from the relay (the per-session
    // stream the relay opens on consumer-attach), read the SESSION_OPEN
    // prefix, ack the OPEN, then echo bytes for the rest of the test.
    let storage_role = tokio::spawn(async move {
        let (mut send, mut recv) = conn.accept_bi().await.expect("accept_bi for session");
        let mut prefix = [0u8; 5];
        recv.read_exact(&mut prefix).await.expect("read prefix");
        assert_eq!(prefix[0], STREAM_TAG_SESSION_OPEN);
        let session_id = u32::from_be_bytes([prefix[1], prefix[2], prefix[3], prefix[4]]);

        let mut buf = BytesMut::new();
        // Read the OPEN envelope frame.
        let frame = read_one_frame_buffered(&mut recv, &mut buf).await;
        assert_eq!(frame.frame_type, FrameType::Open);
        assert_eq!(frame.session_id, session_id);
        // Ack.
        let ack = TunnelFrame::open_ack(session_id).encode().unwrap();
        send.write_all(&ack).await.unwrap();

        // Read the consumer DATA frame.
        let frame = read_one_frame_buffered(&mut recv, &mut buf).await;
        assert_eq!(frame.frame_type, FrameType::Data);
        assert_eq!(frame.session_id, session_id);
        let consumer_payload = frame.payload.clone();
        // Push our own DATA back.
        let storage_payload = vec![0x5Au8; 4096];
        let echo = TunnelFrame::data(session_id, Bytes::from(storage_payload.clone()))
            .encode()
            .unwrap();
        send.write_all(&echo).await.unwrap();

        (session_id, consumer_payload, Bytes::from(storage_payload))
    });

    // Consumer role: drive via the relay's session machinery.
    let (session_id, mut consumer_rx) = open_session_against_tunnel(&relay, &rid).await;
    // Send 4 KiB consumer→storage.
    let consumer_payload = vec![0xC0u8; 4096];
    consumer_send_data(
        &relay,
        &rid,
        session_id,
        Bytes::from(consumer_payload.clone()),
    )
    .await;

    let storage_seen = tokio::time::timeout(Duration::from_secs(5), storage_role)
        .await
        .expect("storage role timeout")
        .expect("storage join");
    let (sid, c2s_payload, s2c_payload) = storage_seen;
    assert_eq!(sid, session_id);
    assert_eq!(c2s_payload.as_ref(), &consumer_payload[..]);

    // Consumer should observe the storage→consumer payload.
    let inbound = tokio::time::timeout(Duration::from_secs(5), consumer_rx.recv())
        .await
        .expect("consumer recv timeout")
        .expect("channel closed");
    assert_eq!(inbound.as_ref(), s2c_payload.as_ref());

    // Cleanup.
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_millis(500), task).await;
}

// ---------------------------------------------------------------------------
// Test 2: migration via Endpoint::rebind
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn quic_tunnel_survives_endpoint_rebind() {
    let (bind_addr, relay, shutdown_tx, task) = spawn_tunnel().await;
    let (rid, cred) = register_storage(&relay).await;

    let endpoint = build_client_endpoint(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0));
    let (conn, _ctrl_send, _ctrl_recv, _) =
        dial_and_handshake(&endpoint, bind_addr, &rid, &cred).await;

    // Storage role: accept session, ack the OPEN, echo two waves of DATA
    // (one before rebind, one after) and finish. We move `conn` into the
    // spawned task; quinn::Connection is Clone-by-Arc so the test scope
    // still has access via the connection's outer state if needed.
    let storage_role = tokio::spawn(async move {
        let (mut send, mut recv) = conn
            .accept_bi()
            .await
            .expect("accept_bi for session");
        let mut prefix = [0u8; 5];
        recv.read_exact(&mut prefix).await.expect("read prefix");
        assert_eq!(prefix[0], STREAM_TAG_SESSION_OPEN);
        let session_id = u32::from_be_bytes([prefix[1], prefix[2], prefix[3], prefix[4]]);

        let mut buf = BytesMut::new();
        // OPEN envelope.
        let frame = read_one_frame_buffered(&mut recv, &mut buf).await;
        assert_eq!(frame.frame_type, FrameType::Open);
        let ack = TunnelFrame::open_ack(session_id).encode().unwrap();
        send.write_all(&ack).await.unwrap();

        // Wave 1.
        let frame = read_one_frame_buffered(&mut recv, &mut buf).await;
        assert_eq!(frame.frame_type, FrameType::Data);
        let echo1 = TunnelFrame::data(session_id, frame.payload).encode().unwrap();
        send.write_all(&echo1).await.unwrap();

        // Wave 2 (after the client rebinds).
        let frame = read_one_frame_buffered(&mut recv, &mut buf).await;
        assert_eq!(frame.frame_type, FrameType::Data);
        let echo2 = TunnelFrame::data(session_id, frame.payload).encode().unwrap();
        send.write_all(&echo2).await.unwrap();

        session_id
    });

    let (session_id, mut consumer_rx) = open_session_against_tunnel(&relay, &rid).await;

    // Wave 1.
    let payload1 = vec![0x11u8; 1024];
    consumer_send_data(&relay, &rid, session_id, Bytes::from(payload1.clone())).await;
    let recv1 = tokio::time::timeout(Duration::from_secs(5), consumer_rx.recv())
        .await
        .expect("recv1 timeout")
        .expect("channel closed");
    assert_eq!(recv1.as_ref(), &payload1[..]);

    // Now rebind the client endpoint to a fresh UDP socket, simulating a
    // network change (WiFi → cellular). Quinn handles PATH_CHALLENGE +
    // PATH_RESPONSE under the hood; the active connection should survive.
    let new_socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("rebind socket");
    let _new_local = new_socket.local_addr().unwrap();
    endpoint.rebind(new_socket).expect("Endpoint::rebind");

    // Give quinn a moment to validate the new path. This is generous —
    // typically a single PATH_CHALLENGE round-trip is sub-millisecond on
    // localhost.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Wave 2.
    let payload2 = vec![0x22u8; 1024];
    consumer_send_data(&relay, &rid, session_id, Bytes::from(payload2.clone())).await;
    let recv2 = tokio::time::timeout(Duration::from_secs(5), consumer_rx.recv())
        .await
        .expect("recv2 timeout (migration broken?)")
        .expect("channel closed after rebind");
    assert_eq!(recv2.as_ref(), &payload2[..]);

    let _ = tokio::time::timeout(Duration::from_secs(5), storage_role).await;
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_millis(500), task).await;
}

// ---------------------------------------------------------------------------
// Test 3: reject on bad credentials
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn quic_tunnel_rejects_bad_credentials() {
    let (bind_addr, relay, shutdown_tx, task) = spawn_tunnel().await;
    let (rid, _real_cred) = register_storage(&relay).await;

    let endpoint = build_client_endpoint(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0));
    let conn = endpoint
        .connect(bind_addr, "localhost")
        .expect("connect")
        .await
        .expect("handshake");

    let (mut send, mut recv) = conn.open_bi().await.expect("open_bi");
    // Send handshake with the wrong credential.
    let mut prefix = vec![HANDSHAKE_VERSION];
    let bad_cred = b"WRONG_CRED";
    prefix.push(bad_cred.len() as u8);
    prefix.extend_from_slice(bad_cred);
    let token = rid.as_bytes();
    prefix.extend_from_slice(&(token.len() as u16).to_be_bytes());
    prefix.extend_from_slice(token);
    send.write_all(&prefix).await.expect("write handshake");

    // Reading the ack OR the connection-close happens almost simultaneously
    // — the relay writes the reject ack and then immediately closes the
    // connection with REGISTRATION_REJECTED. Either outcome is correct for
    // a rejected handshake; we accept both.
    let mut ack = [0u8; 1 + 16];
    let read_result = recv.read_exact(&mut ack).await;
    if read_result.is_ok() {
        assert_eq!(ack[0], 0x01, "ack should signal reject");
    } else {
        // Read aborted by the connection closing — that's fine, the close
        // code below is the source of truth.
    }

    // Connection should close with REGISTRATION_REJECTED.
    let close_reason = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
    let close = close_reason.expect("connection should close after reject");
    let code = match close {
        quinn::ConnectionError::ApplicationClosed(app) => Some(u64::from(app.error_code)),
        other => panic!("unexpected close: {other:?}"),
    };
    assert_eq!(code, Some(close_code::REGISTRATION_REJECTED as u64));

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_millis(500), task).await;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read a single frame from the recv stream, preserving any extra bytes
/// in `buf` for the next call. Critical: the caller MUST reuse the same
/// `buf` across calls — `quinn::RecvStream` chunks can carry multiple
/// frames at once, and a fresh-buf-per-call helper would silently drop
/// trailing bytes after the first frame parses.
async fn read_one_frame_buffered(
    recv: &mut quinn::RecvStream,
    buf: &mut BytesMut,
) -> TunnelFrame {
    let mut chunk = vec![0u8; 8 * 1024];
    loop {
        match TunnelFrame::decode_one(buf) {
            Ok((f, consumed)) => {
                let _ = buf.split_to(consumed);
                return f;
            }
            Err(ohd_relay::frame::FrameError::Truncated { .. }) => {
                let n = recv
                    .read(&mut chunk)
                    .await
                    .expect("read")
                    .expect("stream ended early");
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(err) => panic!("frame decode err: {err}"),
        }
    }
}

#[derive(Debug)]
struct InsecureVerifier {
    provider: rustls::crypto::CryptoProvider,
}

impl rustls::client::danger::ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
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

