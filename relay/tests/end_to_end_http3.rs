//! HTTP/3 (QUIC) smoke test for the relay's REST endpoints.
//!
//! Boots the relay router on a UDP/QUIC listener, dials it from a
//! `quinn`-backed `h3` client, and asserts that `GET /health` returns the
//! expected payload over HTTP/3. The test pins the response status so a
//! future regression that breaks the QUIC path fails loudly.
//!
//! WebSocket-over-HTTP/3 is intentionally NOT exercised here: the relay's
//! tunnel + attach paths stay on the HTTP/2 listener (see
//! `src/http3.rs` for the RFC 9220 caveat).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use bytes::{Buf, BytesMut};
use ohd_relay::http3;
use ohd_relay::push::PushDispatcher;
use ohd_relay::server::{build_router, AppState};
use ohd_relay::state::RelayState;

#[tokio::test(flavor = "multi_thread")]
async fn http3_health_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("relay-h3.db");
    let relay = RelayState::open(db_path.to_str().unwrap()).await.unwrap();
    let emergency = ohd_relay::emergency_endpoints::EmergencyStateTable::new(
        relay.registrations.conn_for_emergency(),
    );
    let app_state = AppState {
        relay,
        push: Arc::new(PushDispatcher::new()),
        public_host: "127.0.0.1:0".to_string(),
        registration_auth: ohd_relay::server::RegistrationAuthState::permissive(),
        #[cfg(feature = "authority")]
        authority: None,
        emergency,
        storage_tunnel: None,
    };
    let router = build_router(app_state);

    let (cert_chain, key) = http3::dev_self_signed_cert().unwrap();

    let std_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let bind_addr = std_socket.local_addr().unwrap();
    drop(std_socket);

    let server_handle = {
        let cert = cert_chain.clone();
        tokio::spawn(async move {
            let _ = http3::serve(bind_addr, router, cert, key).await;
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // ---- h3 client over quinn ----
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut tls_client = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InsecureVerifier {
            provider: rustls::crypto::ring::default_provider(),
        }))
        .with_no_client_auth();
    tls_client.alpn_protocols = vec![b"h3".to_vec()];

    let quic_client_cfg: quinn::crypto::rustls::QuicClientConfig =
        tls_client.try_into().expect("TLS13 → QuicClientConfig");
    let client_config = quinn::ClientConfig::new(Arc::new(quic_client_cfg));
    let mut endpoint = quinn::Endpoint::client(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
    ))
    .unwrap();
    endpoint.set_default_client_config(client_config);

    let connection = endpoint
        .connect(bind_addr, "localhost")
        .expect("dial")
        .await
        .expect("h3 handshake");
    let h3_quinn_conn = h3_quinn::Connection::new(connection);
    let (mut driver, mut send_request) =
        h3::client::new(h3_quinn_conn).await.expect("h3 client");
    let drive_task = tokio::spawn(async move {
        let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
    });

    let req = http::Request::builder()
        .method(http::Method::GET)
        .uri("https://localhost/health")
        .body(())
        .unwrap();
    let mut stream = send_request.send_request(req).await.expect("send_request");
    stream.finish().await.expect("finish");

    let resp = stream.recv_response().await.expect("recv_response");
    assert_eq!(resp.status(), http::StatusCode::OK);

    let mut buf = BytesMut::new();
    while let Some(mut data) = stream.recv_data().await.expect("recv_data") {
        let remaining = data.remaining();
        let mut tmp = vec![0u8; remaining];
        data.copy_to_slice(&mut tmp);
        buf.extend_from_slice(&tmp);
    }
    let body = String::from_utf8(buf.to_vec()).expect("utf8 body");
    assert!(
        body.contains("OHD Relay") && body.contains("ok"),
        "unexpected /health body over HTTP/3: {body:?}"
    );

    drive_task.abort();
    server_handle.abort();
    endpoint.close(0u32.into(), b"bye");
}

// -----------------------------------------------------------------------------
// Insecure verifier — DEV / TEST ONLY. Accepts every cert.
// -----------------------------------------------------------------------------

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
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}
