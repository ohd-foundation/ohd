//! Smoke test for the HTTP/3 client variant in `client::H3RawClient`.
//!
//! Runs a minimal in-process h3 server that responds to a single
//! `OhdcService.Health` POST with a canned `HealthResponse` payload
//! (Connect-protocol unary, `application/proto`). The CLI's `H3RawClient`
//! dials it over QUIC with `--insecure-skip-verify` (necessary because the
//! server uses an `rcgen` self-signed cert) and confirms the round-trip
//! produces the expected `status: "ok"` payload.
//!
//! This is intentionally NOT a full storage round-trip — those tests live
//! in `storage/.../tests/end_to_end_http3.rs` and require the storage core.
//! Here we only exercise the client transport: encode → send → recv →
//! decode.
//!
//! The connect CLI crate ships only as a `[[bin]]`, so we re-mount the
//! relevant source modules with `#[path]` rather than importing them as a
//! library. Mirrors the pattern used in the storage server's tests.

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::{Buf as _, Bytes};

#[allow(dead_code)]
mod proto {
    connectrpc::include_generated!();
}
use proto::ohdc::v0 as pb;

#[allow(dead_code)]
#[path = "../src/client.rs"]
mod client;

use client::H3RawClient;

#[tokio::test(flavor = "multi_thread")]
async fn h3_raw_client_health_round_trip() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // ---- Mint a self-signed cert + bring up a minimal h3 server. ----
    let names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(names).expect("rcgen self-signed");
    let cert_der = cert.der().clone();
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der()),
    );

    let mut tls_server =
        rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .expect("rustls::ServerConfig");
    tls_server.max_early_data_size = u32::MAX;
    tls_server.alpn_protocols = vec![b"h3".to_vec()];
    let quic_crypto: quinn::crypto::rustls::QuicServerConfig =
        tls_server.try_into().expect("QuicServerConfig");
    let qcfg = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));

    let std_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let bind_addr: SocketAddr = std_socket.local_addr().unwrap();
    drop(std_socket);
    let endpoint = quinn::Endpoint::server(qcfg, bind_addr).expect("Endpoint::server");

    let server_handle = tokio::spawn(async move {
        // Accept-loop: keep yielding incoming QUIC connections until the
        // task is aborted at test cleanup time.
        while let Some(incoming) = endpoint.accept().await {
            tokio::spawn(async move {
                let conn = match incoming.await {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let mut h3_conn = match h3::server::Connection::<_, Bytes>::new(
                    h3_quinn::Connection::new(conn),
                )
                .await
                {
                    Ok(c) => c,
                    Err(_) => return,
                };
                // Per-connection request loop. Spawns a task per accepted
                // request so a slow body drain on one stream doesn't block
                // the next.
                while let Ok(Some(resolver)) = h3_conn.accept().await {
                    tokio::spawn(async move {
                        let (req, mut stream) = match resolver.resolve_request().await {
                            Ok(p) => p,
                            Err(_) => return,
                        };
                        while let Ok(Some(chunk)) = stream.recv_data().await {
                            let _ = chunk.remaining();
                        }
                        use buffa::Message;
                        let payload = pb::HealthResponse {
                            status: "ok".into(),
                            server_version: "test-server".into(),
                            protocol_version: "ohdc.v0".into(),
                            server_time_ms: 1_700_000_000_000_i64,
                            ..Default::default()
                        }
                        .encode_to_vec();
                        let resp = http::Response::builder()
                            .status(http::StatusCode::OK)
                            .header("content-type", "application/proto")
                            .header("connect-protocol-version", "1")
                            .body(())
                            .unwrap();
                        if stream.send_response(resp).await.is_err() {
                            return;
                        }
                        let _ = stream.send_data(Bytes::from(payload)).await;
                        let _ = stream.finish().await;
                        let _ = req;
                    });
                }
            });
        }
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // ---- Build an H3RawClient against the dev server. ----
    let host_port = format!("127.0.0.1:{}", bind_addr.port());
    let client = H3RawClient::connect(&host_port, "test-bearer-token", true)
        .await
        .expect("H3RawClient::connect");
    let resp = client
        .health(pb::HealthRequest::default())
        .await
        .expect("H3RawClient::health");
    assert_eq!(resp.status, "ok");
    assert_eq!(resp.protocol_version, "ohdc.v0");
    assert_eq!(resp.server_version, "test-server");

    server_handle.abort();
}
