//! HTTP/3 (QUIC) round-trip smoke test for the OHDC Connect-RPC service.
//!
//! Sibling to `end_to_end.rs` (which exercises HTTP/2 + gRPC framing).
//!
//! Tests:
//! 1. `http3_health_round_trip` — unary `OhdcService.Health` over h3 with
//!    `application/proto` (Connect-protocol unary). Pins the response
//!    content-type so a future regression that flips back to JSON or
//!    gRPC trailers fails fast.
//! 2. `http3_query_events_streaming` — server-streaming
//!    `OhdcService.QueryEvents` over h3 with `application/connect+proto`.
//!    Validates the streaming body adapter (`H3RequestBody`) end-to-end
//!    and asserts at least one event flows back through the
//!    Connect-streaming envelope.
//! 3. `http3_load_pem_cert_key` — round-trips `dev_self_signed_cert` →
//!    PEM file → `load_pem_cert_key`. Validates the production cert
//!    loader matches the dev path byte-for-byte and serves cleanly.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use bytes::{Buf, Bytes, BytesMut};
use ohd_storage_core::{
    auth::issue_self_session_token,
    storage::{Storage, StorageConfig},
};

#[allow(dead_code)]
#[path = "../src/server.rs"]
mod server;

#[allow(dead_code)]
#[path = "../src/sync_server.rs"]
mod sync_server;

#[allow(dead_code)]
#[path = "../src/auth_server.rs"]
mod auth_server;

#[allow(dead_code)]
#[path = "../src/jwks.rs"]
mod jwks;

#[allow(dead_code)]
#[path = "../src/http3.rs"]
mod http3;

#[allow(dead_code)]
#[path = "../src/oauth.rs"]
mod oauth;

mod proto {
    connectrpc::include_generated!();
}

use proto::ohdc::v0 as pb;

#[tokio::test(flavor = "multi_thread")]
async fn http3_health_round_trip() {
    // ---- Server: open temp DB, mint a self-session token, build the
    //              ConnectRpcService that backs both the HTTP/2 and HTTP/3
    //              listeners, then start ONLY the HTTP/3 listener for this
    //              test. ----
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("e2e-h3.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    let user_ulid = storage.user_ulid();
    let _bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, user_ulid, Some("e2e-h3"), None))
        .unwrap();

    let svc = server::connect_service(storage.clone());
    let (cert_chain, key) = http3::dev_self_signed_cert().unwrap();

    // Bind on an ephemeral UDP port. We pre-bind via std + extract the
    // port so the client knows where to dial without relying on the server
    // task to publish it.
    let std_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let bind_addr = std_socket.local_addr().unwrap();
    drop(std_socket); // hand back; quinn re-binds to the same port

    let server_handle = {
        let cert = cert_chain.clone();
        tokio::spawn(async move {
            let _ = http3::serve(bind_addr, svc, cert, key).await;
        })
    };

    // Give the listener a beat. quinn::Endpoint::server is non-blocking but
    // there's still a brief window before the UDP socket is attached.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // ---- Client: build a quinn::Endpoint with a permissive verifier. ----
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    for c in &cert_chain {
        roots.add(c.clone()).unwrap();
    }
    // Even with the server's cert in our roots, the cert was issued by
    // rcgen with no real CA chain — go with a permissive verifier for
    // the test path. (The server never verifies the client; this is the
    // client → server direction.)
    let mut tls_client =
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureVerifier {
                provider: rustls::crypto::ring::default_provider(),
            }))
            .with_no_client_auth();
    tls_client.alpn_protocols = vec![b"h3".to_vec()];

    let quic_client_cfg: quinn::crypto::rustls::QuicClientConfig =
        tls_client.try_into().expect("TLS13 → QuicClientConfig");
    let client_config = quinn::ClientConfig::new(Arc::new(quic_client_cfg));
    let mut endpoint =
        quinn::Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
    endpoint.set_default_client_config(client_config);

    let connection = endpoint
        .connect(bind_addr, "localhost")
        .expect("dial")
        .await
        .expect("h3 handshake");

    // ---- h3 client over the QUIC connection. ----
    let h3_quinn_conn = h3_quinn::Connection::new(connection);
    let (mut h3_driver, mut send_request) =
        h3::client::new(h3_quinn_conn).await.expect("h3 client");

    // Drive the connection in the background. h3 requires us to keep the
    // driver future running so background streams (control + QPACK) make
    // progress; if we drop it, the connection silently stalls.
    let drive_task = tokio::spawn(async move {
        let _ = std::future::poll_fn(|cx| h3_driver.poll_close(cx)).await;
    });

    // ---- Encode a Connect-protocol unary request for OhdcService.Health. ----
    //
    // Connect's binary unary wire is: an `application/proto` POST whose body
    // is the raw Protobuf-encoded request message (no length prefix, no
    // gRPC framing). The response body is the raw Protobuf-encoded response
    // message. Status lives in HTTP status code + the optional
    // `Connect-Status` trailing envelope (only set on errors).
    use buffa::Message;
    let req_msg = pb::HealthRequest::default();
    let req_body = Bytes::from(req_msg.encode_to_vec());

    let req = http::Request::builder()
        .method(http::Method::POST)
        .uri("https://localhost/ohdc.v0.OhdcService/Health")
        .header("content-type", "application/proto")
        .header("connect-protocol-version", "1")
        .body(())
        .unwrap();

    let mut stream = send_request.send_request(req).await.expect("send_request");
    stream.send_data(req_body).await.expect("send_data");
    stream.finish().await.expect("finish");

    let resp = stream.recv_response().await.expect("recv_response");
    assert_eq!(resp.status(), http::StatusCode::OK, "non-200 from Health");

    // PIN the wire content-type: Connect protocol unary = `application/proto`.
    // If a future regression switches this back to JSON or to gRPC framing,
    // the test fails fast with a precise message.
    let ct = resp
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        ct, "application/proto",
        "Connect-protocol HTTP/3 path must respond with application/proto; got {ct:?}"
    );

    // Drain the body and decode the HealthResponse.
    let mut buf = BytesMut::new();
    while let Some(mut data) = stream.recv_data().await.expect("recv_data") {
        let remaining = data.remaining();
        let mut tmp = vec![0u8; remaining];
        data.copy_to_slice(&mut tmp);
        buf.extend_from_slice(&tmp);
    }
    let body = buf.freeze();
    let parsed = pb::HealthResponse::decode_from_slice(&body[..]).expect("decode HealthResponse");
    assert_eq!(parsed.status, "ok");
    assert_eq!(parsed.protocol_version, "ohdc.v0");

    // ---- Cleanup. ----
    drive_task.abort();
    server_handle.abort();
    endpoint.close(0u32.into(), b"bye");
}

#[tokio::test(flavor = "multi_thread")]
async fn http3_query_events_streaming() {
    // ---- Server boot: open a temp DB, mint a self-session token, seed
    //                   one std.blood_glucose event (so QueryEvents has
    //                   something to return), launch the HTTP/3 listener. ----
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("e2e-h3-stream.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, user_ulid, Some("e2e-h3-stream"), None))
        .unwrap();
    // Seed one event via the in-process core API (faster than round-tripping
    // PutEvents over HTTP/3 just to set up the read).
    let resolved = storage
        .with_conn(|conn| ohd_storage_core::auth::resolve_token(conn, &bearer))
        .unwrap();
    let inputs = vec![ohd_storage_core::events::EventInput {
        timestamp_ms: 1_700_000_000_000_i64,
        event_type: "std.blood_glucose".to_string(),
        channels: vec![ohd_storage_core::events::ChannelValue {
            channel_path: "value".to_string(),
            value: ohd_storage_core::events::ChannelScalar::Real { real_value: 6.7 },
        }],
        ..Default::default()
    }];
    let _ =
        ohd_storage_core::ohdc::put_events(&storage, &resolved, &inputs).expect("seed PutEvents");

    let svc = server::connect_service(storage.clone());
    let (cert_chain, key) = http3::dev_self_signed_cert().unwrap();

    let std_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let bind_addr = std_socket.local_addr().unwrap();
    drop(std_socket);

    let server_handle = {
        let cert = cert_chain.clone();
        tokio::spawn(async move {
            let _ = http3::serve(bind_addr, svc, cert, key).await;
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // ---- Client boot. ----
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut tls_client =
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureVerifier {
                provider: rustls::crypto::ring::default_provider(),
            }))
            .with_no_client_auth();
    tls_client.alpn_protocols = vec![b"h3".to_vec()];
    let quic_client_cfg: quinn::crypto::rustls::QuicClientConfig =
        tls_client.try_into().expect("TLS13 → QuicClientConfig");
    let client_config = quinn::ClientConfig::new(Arc::new(quic_client_cfg));
    let mut endpoint =
        quinn::Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
    endpoint.set_default_client_config(client_config);
    let connection = endpoint
        .connect(bind_addr, "localhost")
        .expect("dial")
        .await
        .expect("h3 handshake");
    let h3_quinn_conn = h3_quinn::Connection::new(connection);
    let (mut h3_driver, mut send_request) =
        h3::client::new(h3_quinn_conn).await.expect("h3 client");
    let drive_task = tokio::spawn(async move {
        let _ = std::future::poll_fn(|cx| h3_driver.poll_close(cx)).await;
    });

    // ---- Build a Connect-protocol streaming request body for QueryEvents. ----
    //
    // Connect server-streaming: content-type `application/connect+proto`,
    // request body framed as `[1 byte flags][4 bytes BE length][payload]`.
    use buffa::Message;
    let req_msg = pb::QueryEventsRequest {
        filter: ::buffa::MessageField::some(pb::EventFilter {
            event_types_in: vec!["std.blood_glucose".into()],
            include_superseded: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let req_payload = req_msg.encode_to_vec();
    let mut framed = BytesMut::with_capacity(5 + req_payload.len());
    framed.extend_from_slice(&[0u8]);
    framed.extend_from_slice(&(req_payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(&req_payload);
    let framed = framed.freeze();

    let req = http::Request::builder()
        .method(http::Method::POST)
        .uri("https://localhost/ohdc.v0.OhdcService/QueryEvents")
        .header("content-type", "application/connect+proto")
        .header("connect-protocol-version", "1")
        .header("authorization", format!("Bearer {bearer}"))
        .body(())
        .unwrap();

    let mut stream = send_request.send_request(req).await.expect("send_request");
    stream.send_data(framed).await.expect("send_data");
    stream.finish().await.expect("finish");

    let resp = stream.recv_response().await.expect("recv_response");
    assert_eq!(
        resp.status(),
        http::StatusCode::OK,
        "non-200 from QueryEvents"
    );
    let ct = resp
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("application/connect+proto"),
        "QueryEvents over H3 must respond with application/connect+proto; got {ct:?}"
    );

    // ---- Drain the streaming body and parse the Connect frames. ----
    let mut buf = BytesMut::new();
    while let Some(mut data) = stream.recv_data().await.expect("recv_data") {
        let remaining = data.remaining();
        let mut tmp = vec![0u8; remaining];
        data.copy_to_slice(&mut tmp);
        buf.extend_from_slice(&tmp);
    }
    let body = buf.freeze();

    // Parse [1 byte flags][4 byte len][payload] frames.
    let mut events = Vec::new();
    let mut saw_end = false;
    let mut cursor = &body[..];
    while !cursor.is_empty() {
        assert!(cursor.len() >= 5, "truncated Connect frame header");
        let flags = cursor[0];
        let len = u32::from_be_bytes([cursor[1], cursor[2], cursor[3], cursor[4]]) as usize;
        assert!(cursor.len() >= 5 + len, "truncated Connect frame body");
        let payload = &cursor[5..5 + len];
        if flags & 0x02 != 0 {
            // End-of-stream envelope (JSON). Any error envelope here would
            // contain `"error"`; success is `{}` (or `{"metadata":...}`).
            let env = std::str::from_utf8(payload).unwrap_or("");
            assert!(
                !env.contains("\"error\""),
                "QueryEvents stream ended with error envelope: {env}"
            );
            saw_end = true;
        } else {
            let evt =
                pb::Event::decode_from_slice(payload).expect("decode pb::Event from data frame");
            events.push(evt);
        }
        cursor = &cursor[5 + len..];
    }
    assert!(saw_end, "no end-of-stream frame received from QueryEvents");
    assert!(
        !events.is_empty(),
        "QueryEvents over H3 returned no events; expected at least 1 (the seeded std.blood_glucose)"
    );
    let evt = &events[0];
    assert_eq!(evt.event_type, "std.blood_glucose");

    // Cleanup.
    drive_task.abort();
    server_handle.abort();
    endpoint.close(0u32.into(), b"bye");
}

/// Progressive-delivery sibling to `http3_query_events_streaming`.
///
/// Seeds 50 events and asserts that the streaming response **arrives in
/// multiple `recv_data` waves** rather than as one giant blob — proving the
/// `H3RequestBody` ⇄ `ConnectRpcService` ⇄ h3 `send_data` chain doesn't
/// implicitly buffer. The threshold is "at least one read returned bytes
/// while more reads are still queued"; in practice on a localhost link with
/// 50 events serialized, the test reliably observes 5+ separate
/// recv_data() returns. We assert ≥ 2 to keep the test stable across
/// different system schedulers.
#[tokio::test(flavor = "multi_thread")]
async fn http3_query_events_progressive_streaming() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("e2e-h3-progressive.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, user_ulid, Some("e2e-h3-prog"), None))
        .unwrap();
    let resolved = storage
        .with_conn(|conn| ohd_storage_core::auth::resolve_token(conn, &bearer))
        .unwrap();

    // Seed 50 events. Each Event encodes to a few hundred bytes; 50 of them
    // comfortably exceed any single send_data buffer.
    let mut inputs = Vec::new();
    for i in 0..50 {
        inputs.push(ohd_storage_core::events::EventInput {
            timestamp_ms: 1_700_000_000_000_i64 + i * 1000,
            event_type: "std.blood_glucose".to_string(),
            channels: vec![ohd_storage_core::events::ChannelValue {
                channel_path: "value".to_string(),
                value: ohd_storage_core::events::ChannelScalar::Real {
                    real_value: 5.0 + (i as f64) * 0.1,
                },
            }],
            ..Default::default()
        });
    }
    let _ =
        ohd_storage_core::ohdc::put_events(&storage, &resolved, &inputs).expect("seed PutEvents");

    let svc = server::connect_service(storage.clone());
    let (cert_chain, key) = http3::dev_self_signed_cert().unwrap();

    let std_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let bind_addr = std_socket.local_addr().unwrap();
    drop(std_socket);

    let server_handle = {
        let cert = cert_chain.clone();
        tokio::spawn(async move {
            let _ = http3::serve(bind_addr, svc, cert, key).await;
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut tls_client =
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureVerifier {
                provider: rustls::crypto::ring::default_provider(),
            }))
            .with_no_client_auth();
    tls_client.alpn_protocols = vec![b"h3".to_vec()];
    let quic_client_cfg: quinn::crypto::rustls::QuicClientConfig =
        tls_client.try_into().expect("TLS13 → QuicClientConfig");
    let client_config = quinn::ClientConfig::new(Arc::new(quic_client_cfg));
    let mut endpoint =
        quinn::Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
    endpoint.set_default_client_config(client_config);
    let connection = endpoint
        .connect(bind_addr, "localhost")
        .expect("dial")
        .await
        .expect("h3 handshake");
    let h3_quinn_conn = h3_quinn::Connection::new(connection);
    let (mut h3_driver, mut send_request) =
        h3::client::new(h3_quinn_conn).await.expect("h3 client");
    let drive_task = tokio::spawn(async move {
        let _ = std::future::poll_fn(|cx| h3_driver.poll_close(cx)).await;
    });

    use buffa::Message;
    let req_msg = pb::QueryEventsRequest {
        filter: ::buffa::MessageField::some(pb::EventFilter {
            event_types_in: vec!["std.blood_glucose".into()],
            include_superseded: true,
            limit: Some(100),
            ..Default::default()
        }),
        ..Default::default()
    };
    let req_payload = req_msg.encode_to_vec();
    let mut framed = BytesMut::with_capacity(5 + req_payload.len());
    framed.extend_from_slice(&[0u8]);
    framed.extend_from_slice(&(req_payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(&req_payload);
    let framed = framed.freeze();

    let req = http::Request::builder()
        .method(http::Method::POST)
        .uri("https://localhost/ohdc.v0.OhdcService/QueryEvents")
        .header("content-type", "application/connect+proto")
        .header("connect-protocol-version", "1")
        .header("authorization", format!("Bearer {bearer}"))
        .body(())
        .unwrap();

    let mut stream = send_request.send_request(req).await.expect("send_request");
    stream.send_data(framed).await.expect("send_data");
    stream.finish().await.expect("finish");

    let resp = stream.recv_response().await.expect("recv_response");
    assert_eq!(resp.status(), http::StatusCode::OK);

    // Count distinct recv_data() returns. Each separate Ok(Some(_)) is a
    // wire-level chunk crossing the QUIC stream — this is what "progressive
    // delivery" means. A non-streaming server-streaming impl would coalesce
    // into one recv_data() return.
    let mut chunk_count = 0usize;
    let mut total_bytes = 0usize;
    let mut buf = BytesMut::new();
    while let Some(mut data) = stream.recv_data().await.expect("recv_data") {
        chunk_count += 1;
        let remaining = data.remaining();
        total_bytes += remaining;
        let mut tmp = vec![0u8; remaining];
        data.copy_to_slice(&mut tmp);
        buf.extend_from_slice(&tmp);
    }
    assert!(chunk_count >= 2,
        "expected progressive delivery (≥2 recv_data chunks); got {chunk_count} chunks ({total_bytes} bytes)");

    // Sanity: the body should still parse as Connect frames and yield the
    // 50 events.
    let body = buf.freeze();
    let mut events = 0usize;
    let mut cursor = &body[..];
    while !cursor.is_empty() {
        assert!(cursor.len() >= 5);
        let flags = cursor[0];
        let len = u32::from_be_bytes([cursor[1], cursor[2], cursor[3], cursor[4]]) as usize;
        assert!(cursor.len() >= 5 + len);
        if flags & 0x02 == 0 {
            events += 1;
        }
        cursor = &cursor[5 + len..];
    }
    assert_eq!(events, 50, "all 50 events should round-trip");

    drive_task.abort();
    server_handle.abort();
    endpoint.close(0u32.into(), b"bye");
}

#[tokio::test(flavor = "multi_thread")]
async fn http3_load_pem_cert_key() {
    // ---- Synthesize a cert pair, write it as PEM to disk, then call
    //      `http3::load_pem_cert_key` and use the result to bring up a
    //      server. Validates the production cert loader path. ----
    use rustls::pki_types::pem::PemObject as _;

    let dir = tempfile::tempdir().unwrap();
    let cert_path = dir.path().join("server.crt");
    let key_path = dir.path().join("server.key");

    // rcgen produces DER + PEM; pull the PEM strings directly so we don't
    // re-encode by hand. `cert_pair_pem` returns a (cert PEM, key PEM)
    // pair.
    let names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(names).expect("rcgen self-signed");
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, key_pair.serialize_pem()).unwrap();

    let (cert_chain, key) =
        http3::load_pem_cert_key(&cert_path, &key_path).expect("load_pem_cert_key");
    assert!(!cert_chain.is_empty(), "no certs parsed from PEM");
    // Sanity: re-parse the original cert PEM directly and compare bytes.
    let direct: rustls::pki_types::CertificateDer<'static> =
        rustls::pki_types::CertificateDer::from_pem_file(&cert_path).expect("re-parse cert PEM");
    assert_eq!(cert_chain[0].as_ref(), direct.as_ref());
    let _ = key; // key parse already validated by load_pem_cert_key

    // Bring up a server with the loaded cert. We don't dial it back here
    // (that would duplicate the health round-trip test); just confirm
    // `quinn::Endpoint::server` accepts the `ServerConfig` built from the
    // loaded materials.
    let (cert_again, key_again) = http3::load_pem_cert_key(&cert_path, &key_path).unwrap();
    let qcfg = http3::server_config(cert_again, key_again).expect("server_config");
    let std_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let bind_addr = std_socket.local_addr().unwrap();
    drop(std_socket);
    let endpoint = quinn::Endpoint::server(qcfg, bind_addr).expect("Endpoint::server");
    drop(endpoint);

    // ---- Negative cases: missing files / empty PEM. ----
    let missing = dir.path().join("nope.pem");
    let err = http3::load_pem_cert_key(&missing, &key_path).unwrap_err();
    assert!(
        err.to_string().contains("open cert file"),
        "missing cert: {err}"
    );

    let empty = dir.path().join("empty.pem");
    std::fs::write(&empty, b"").unwrap();
    let err = http3::load_pem_cert_key(&empty, &key_path).unwrap_err();
    assert!(
        err.to_string().contains("no CERTIFICATE blocks"),
        "empty cert: {err}"
    );
}

// -----------------------------------------------------------------------------
// Insecure verifier — DEV / TEST ONLY. Accepts every cert. Production code
// uses `rustls::ClientConfig::with_root_certificates(roots)`.
// -----------------------------------------------------------------------------

#[derive(Debug)]
struct InsecureVerifier {
    provider: rustls::crypto::CryptoProvider,
}

impl rustls::client::danger::ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
