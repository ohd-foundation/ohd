//! End-to-end test:
//! - Spin up the relay axum server.
//! - Register a user (HTTP POST).
//! - Open a "storage" WebSocket on `/v1/tunnel/:rid`.
//! - Open a "consumer" WebSocket on `/v1/attach/:rid`.
//! - Push a 4 KiB chunk in each direction; verify byte-identical delivery.
//! - Close cleanly.
//!
//! The "storage" and "consumer" speak the binary `TunnelFrame` protocol over
//! the WebSocket, the same way the real components will.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use ohd_relay::frame::{FrameType, TunnelFrame};
use ohd_relay::push::PushDispatcher;
use ohd_relay::server::{build_router, AppState, RegisterRequest, RegisterResponse};
use ohd_relay::state::RelayState;
use tokio_tungstenite::tungstenite::Message as TMessage;

async fn spawn_relay() -> SocketAddr {
    let relay = RelayState::in_memory().await.expect("in-memory state");
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
    let app = build_router(app_state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    // Tiny grace period to ensure the server is accepting.
    tokio::time::sleep(Duration::from_millis(20)).await;
    addr
}

async fn register(addr: SocketAddr) -> RegisterResponse {
    let url = format!("http://{}/v1/register", addr);
    let body = RegisterRequest {
        user_ulid: "0123456789abcdef0123456789abcdef".to_string(),
        storage_pubkey_spki_hex: "deadbeef".repeat(8),
        push_token: None,
        user_label: Some("e2e-test".to_string()),
        id_token: None,
    };
    let client = reqwest::Client::new();
    let resp = client.post(&url).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    resp.json().await.unwrap()
}

#[tokio::test]
async fn end_to_end_4kib_roundtrip() {
    let addr = spawn_relay().await;

    // Register.
    let registered = register(addr).await;
    let rid = registered.rendezvous_id;

    // Storage WS connect.
    let storage_url = format!("ws://{}/v1/tunnel/{}", addr, rid);
    let (mut storage_ws, _) = tokio_tungstenite::connect_async(&storage_url)
        .await
        .expect("storage ws");

    // Consumer WS connect.
    let consumer_url = format!("ws://{}/v1/attach/{}", addr, rid);
    let (mut consumer_ws, _) = tokio_tungstenite::connect_async(&consumer_url)
        .await
        .expect("consumer ws");

    // Storage should see an OPEN frame from the relay first.
    let msg = tokio::time::timeout(Duration::from_secs(2), storage_ws.next())
        .await
        .expect("OPEN timeout")
        .expect("storage stream ended")
        .expect("ws err");
    let bytes = match msg {
        TMessage::Binary(b) => b,
        other => panic!("storage expected binary frame, got {:?}", other),
    };
    let open_frame = TunnelFrame::decode(&bytes).expect("decode OPEN");
    assert_eq!(open_frame.frame_type, FrameType::Open);
    let session_id = open_frame.session_id;
    assert!(session_id != 0);

    // Storage replies OPEN_ACK.
    let ack = TunnelFrame::open_ack(session_id).encode().unwrap();
    storage_ws.send(TMessage::Binary(ack.to_vec())).await.unwrap();

    // Consumer → storage: 4 KiB.
    let consumer_payload = vec![0xC0u8; 4096];
    let consumer_data = TunnelFrame::data(0, Bytes::from(consumer_payload.clone()))
        .encode()
        .unwrap();
    consumer_ws
        .send(TMessage::Binary(consumer_data.to_vec()))
        .await
        .unwrap();

    // Storage receives the 4 KiB DATA frame.
    let msg = tokio::time::timeout(Duration::from_secs(2), storage_ws.next())
        .await
        .expect("c→s timeout")
        .expect("storage stream ended")
        .expect("ws err");
    let bytes = match msg {
        TMessage::Binary(b) => b,
        other => panic!("storage expected binary, got {:?}", other),
    };
    let data_frame = TunnelFrame::decode(&bytes).expect("decode c→s DATA");
    assert_eq!(data_frame.frame_type, FrameType::Data);
    assert_eq!(data_frame.session_id, session_id);
    assert_eq!(&data_frame.payload[..], &consumer_payload[..]);

    // Storage → consumer: 4 KiB.
    let storage_payload = vec![0x5Au8; 4096];
    let storage_data = TunnelFrame::data(session_id, Bytes::from(storage_payload.clone()))
        .encode()
        .unwrap();
    storage_ws
        .send(TMessage::Binary(storage_data.to_vec()))
        .await
        .unwrap();

    // Consumer receives the 4 KiB DATA frame.
    let msg = tokio::time::timeout(Duration::from_secs(2), consumer_ws.next())
        .await
        .expect("s→c timeout")
        .expect("consumer stream ended")
        .expect("ws err");
    let bytes = match msg {
        TMessage::Binary(b) => b,
        other => panic!("consumer expected binary, got {:?}", other),
    };
    let data_frame = TunnelFrame::decode(&bytes).expect("decode s→c DATA");
    assert_eq!(data_frame.frame_type, FrameType::Data);
    assert_eq!(data_frame.session_id, session_id);
    assert_eq!(&data_frame.payload[..], &storage_payload[..]);

    // Clean close from consumer.
    let close = TunnelFrame::close(0, Bytes::new()).encode().unwrap();
    consumer_ws
        .send(TMessage::Binary(close.to_vec()))
        .await
        .unwrap();

    // Storage should observe the CLOSE.
    let mut saw_close = false;
    for _ in 0..5 {
        match tokio::time::timeout(Duration::from_secs(1), storage_ws.next()).await {
            Ok(Some(Ok(TMessage::Binary(b)))) => {
                if let Ok(f) = TunnelFrame::decode(&b) {
                    if f.frame_type == FrameType::Close {
                        saw_close = true;
                        break;
                    }
                }
            }
            Ok(Some(Ok(_))) => continue,
            _ => break,
        }
    }
    assert!(saw_close, "storage did not see CLOSE for the session");
}

#[tokio::test]
async fn deregister_removes_registration() {
    let addr = spawn_relay().await;
    let registered = register(addr).await;
    let url = format!("http://{}/v1/deregister", addr);
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "rendezvous_id": registered.rendezvous_id,
        "long_lived_credential": registered.long_lived_credential,
    });
    let resp = client.post(&url).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let parsed: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(parsed["ok"], serde_json::Value::Bool(true));
}

#[tokio::test]
async fn heartbeat_with_bad_credential_rejected() {
    let addr = spawn_relay().await;
    let registered = register(addr).await;
    let url = format!("http://{}/v1/heartbeat", addr);
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "rendezvous_id": registered.rendezvous_id,
        "long_lived_credential": "WRONG_CREDENTIAL",
    });
    let resp = client.post(&url).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_rendezvous_returns_not_found_on_attach() {
    let addr = spawn_relay().await;
    let url = format!("ws://{}/v1/attach/{}", addr, "NOT_A_REAL_ID");
    let result = tokio_tungstenite::connect_async(&url).await;
    assert!(result.is_err(), "expected failure on unknown rendezvous_id");
}
