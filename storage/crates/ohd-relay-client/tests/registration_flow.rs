//! Registration-flow integration test.
//!
//! Spins up a minimal mock relay over a raw `TcpListener` (no extra
//! dependencies), drives the full `register` → `heartbeat` → `deregister`
//! sequence through [`RegistrationClient`], and asserts the request bodies
//! the relay receives and the responses the client parses back.

use std::sync::Arc;

use ohd_relay_client::registration::{
    CredentialedRequest, PushToken, RegisterRequest, RegistrationClient,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// One HTTP request the mock relay observed: (path, json body).
type Captured = (String, serde_json::Value);

/// Minimal HTTP/1.1 mock relay. Reads one request, captures the path +
/// JSON body, replies with `status` + `body`, then closes the connection.
async fn mock_relay(
    listener: TcpListener,
    captured: Arc<Mutex<Vec<Captured>>>,
    replies: Vec<(u16, String)>,
) {
    for (status, body) in replies {
        let (mut sock, _) = listener.accept().await.expect("accept");

        // Read the full request (headers + body). The client always sends
        // a Content-Length, so read headers first, then the body.
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        let (headers_end, content_len, path) = loop {
            let n = sock.read(&mut tmp).await.expect("read");
            assert!(n > 0, "client closed before sending request");
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                let path = head
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("")
                    .to_string();
                let cl = head
                    .lines()
                    .find_map(|l| {
                        let l = l.to_ascii_lowercase();
                        l.strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                break (pos + 4, cl, path);
            }
        };
        while buf.len() < headers_end + content_len {
            let n = sock.read(&mut tmp).await.expect("read body");
            assert!(n > 0, "client closed mid-body");
            buf.extend_from_slice(&tmp[..n]);
        }
        let body_bytes = &buf[headers_end..headers_end + content_len];
        let json: serde_json::Value =
            serde_json::from_slice(body_bytes).expect("request body is json");
        captured.lock().await.push((path, json));

        let reason = match status {
            200 => "OK",
            201 => "Created",
            401 => "Unauthorized",
            _ => "Status",
        };
        let resp = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        sock.write_all(resp.as_bytes()).await.expect("write resp");
        sock.flush().await.expect("flush");
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[tokio::test]
async fn register_heartbeat_deregister_roundtrip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let captured = Arc::new(Mutex::new(Vec::new()));
    let replies = vec![
        (
            201,
            r#"{"rendezvous_id":"rdv22charbase32xyz","rendezvous_url":"ws://127.0.0.1/v1/tunnel/rdv22charbase32xyz","long_lived_credential":"llc_secret_token"}"#
                .to_string(),
        ),
        (200, r#"{"ok":true}"#.to_string()),
        (200, r#"{"ok":true}"#.to_string()),
    ];
    let server = tokio::spawn(mock_relay(listener, Arc::clone(&captured), replies));

    let client = RegistrationClient::new(format!("http://{addr}")).expect("client");

    // -- register --
    let reg = client
        .register(&RegisterRequest {
            user_ulid: "0123456789abcdef0123456789abcdef".into(),
            storage_pubkey_spki_hex: "deadbeef".into(),
            push_token: Some(PushToken::Fcm("fcm-token".into())),
            user_label: Some("Pixel 9".into()),
            id_token: None,
        })
        .await
        .expect("register");
    assert_eq!(reg.rendezvous_id, "rdv22charbase32xyz");
    assert_eq!(reg.long_lived_credential, "llc_secret_token");

    // -- heartbeat --
    let hb = client
        .heartbeat(&CredentialedRequest {
            rendezvous_id: reg.rendezvous_id.clone(),
            long_lived_credential: reg.long_lived_credential.clone(),
        })
        .await
        .expect("heartbeat");
    assert!(hb.ok);

    // -- deregister --
    let dr = client
        .deregister(&CredentialedRequest {
            rendezvous_id: reg.rendezvous_id.clone(),
            long_lived_credential: reg.long_lived_credential.clone(),
        })
        .await
        .expect("deregister");
    assert!(dr.ok);

    server.await.expect("server task");

    // -- assert the relay saw the right paths + bodies --
    let cap = captured.lock().await;
    assert_eq!(cap.len(), 3);

    assert_eq!(cap[0].0, "/v1/register");
    assert_eq!(
        cap[0].1["storage_pubkey_spki_hex"].as_str(),
        Some("deadbeef")
    );
    assert_eq!(cap[0].1["push_token"]["platform"].as_str(), Some("fcm"));
    assert_eq!(cap[0].1["user_label"].as_str(), Some("Pixel 9"));
    // Optional `id_token` elided when None.
    assert!(cap[0].1.get("id_token").is_none());

    assert_eq!(cap[1].0, "/v1/heartbeat");
    assert_eq!(
        cap[1].1["rendezvous_id"].as_str(),
        Some("rdv22charbase32xyz")
    );
    assert_eq!(
        cap[1].1["long_lived_credential"].as_str(),
        Some("llc_secret_token")
    );

    assert_eq!(cap[2].0, "/v1/deregister");
}

#[tokio::test]
async fn register_surfaces_rejection_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let captured = Arc::new(Mutex::new(Vec::new()));
    let replies = vec![(401, r#"{"error":"OIDC_REQUIRED"}"#.to_string())];
    let server = tokio::spawn(mock_relay(listener, Arc::clone(&captured), replies));

    let client = RegistrationClient::new(format!("http://{addr}")).expect("client");
    let err = client
        .register(&RegisterRequest {
            user_ulid: "00".into(),
            storage_pubkey_spki_hex: "11".into(),
            push_token: None,
            user_label: None,
            id_token: None,
        })
        .await
        .expect_err("should reject");

    match err {
        ohd_relay_client::registration::RegistrationError::Rejected { status, body } => {
            assert_eq!(status, 401);
            assert!(body.contains("OIDC_REQUIRED"));
        }
        other => panic!("expected Rejected, got {other:?}"),
    }

    server.await.expect("server task");
}

#[tokio::test]
async fn custom_base_path_is_used() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");

    let captured = Arc::new(Mutex::new(Vec::new()));
    let replies = vec![(200, r#"{"ok":true}"#.to_string())];
    let server = tokio::spawn(mock_relay(listener, Arc::clone(&captured), replies));

    let client = RegistrationClient::new(format!("http://{addr}"))
        .expect("client")
        .with_base_path("/relay/v1");
    client
        .heartbeat(&CredentialedRequest {
            rendezvous_id: "r".into(),
            long_lived_credential: "c".into(),
        })
        .await
        .expect("heartbeat");

    server.await.expect("server task");
    let cap = captured.lock().await;
    assert_eq!(cap[0].0, "/relay/v1/heartbeat");
}
