//! End-to-end tests for the new emergency-flow endpoints:
//!
//! - `GET  /v1/emergency/status/{request_id}`
//! - `POST /v1/emergency/handoff`
//!
//! Authority-mode is NOT required for these endpoints (status reads from
//! the local SQLite; handoff forwards through the storage tunnel
//! client). The `/v1/emergency/initiate` endpoint that mints a request
//! IS authority-gated; under the no-authority configuration the
//! integration tests below seed `_emergency_requests` directly via the
//! library API to exercise the polling + handoff paths without booting
//! Fulcio.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ohd_relay::emergency_endpoints::{
    run_ttl_sweep, EmergencyConfigResponse, EmergencyDefaultAction, EmergencyHandoffResponse,
    EmergencyRequestRow, EmergencyState, EmergencyStateTable, EmergencyStatusResponse,
    HandoffCaseResponse, MockStorageTunnel, StorageTunnelClient, REQUEST_GC_GRACE,
};
use ohd_relay::push::PushDispatcher;
use ohd_relay::server::{
    build_router, AppState, RegisterRequest, RegisterResponse, RegistrationAuthState,
};
use ohd_relay::state::{now_ms, RelayState};

async fn spawn_relay_with_mock(
    mock: Arc<MockStorageTunnel>,
) -> (SocketAddr, EmergencyStateTable) {
    let relay = RelayState::in_memory().await.unwrap();
    let emergency = EmergencyStateTable::new(relay.registrations.conn_for_emergency());
    // Upcast `Arc<MockStorageTunnel>` → `Arc<dyn StorageTunnelClient>`.
    // Without the explicit cast `Some(mock)` infers to `Option<Arc<MockStorageTunnel>>`
    // which doesn't match `AppState::storage_tunnel`'s trait-object shape.
    let storage_tunnel: Option<Arc<dyn StorageTunnelClient>> =
        Some(mock as Arc<dyn StorageTunnelClient>);
    let app_state = AppState {
        relay,
        push: Arc::new(PushDispatcher::new()),
        public_host: "127.0.0.1:0".to_string(),
        registration_auth: RegistrationAuthState::permissive(),
        #[cfg(feature = "authority")]
        authority: None,
        emergency: emergency.clone(),
        storage_tunnel,
    };
    let app = build_router(app_state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (addr, emergency)
}

async fn register(addr: SocketAddr) -> RegisterResponse {
    let url = format!("http://{}/v1/register", addr);
    let body = RegisterRequest {
        user_ulid: "0123456789abcdef0123456789abcdef".to_string(),
        storage_pubkey_spki_hex: "deadbeef".repeat(8),
        push_token: None,
        user_label: Some("emergency-test".to_string()),
        id_token: None,
    };
    let client = reqwest::Client::new();
    let resp = client.post(&url).json(&body).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    resp.json().await.unwrap()
}

fn seed_request(rendezvous_id: &str, request_id: &str, ttl_ms: i64) -> EmergencyRequestRow {
    let now = now_ms();
    EmergencyRequestRow {
        request_id: request_id.into(),
        rendezvous_id: rendezvous_id.into(),
        state: EmergencyState::Waiting,
        patient_label: Some("Patient X".into()),
        grant_token: None,
        case_ulid: None,
        rejected_reason: None,
        default_action: None,
        created_at_ms: now,
        decided_at_ms: None,
        expires_at_ms: now + ttl_ms,
        gc_after_ms: now + ttl_ms + REQUEST_GC_GRACE.as_millis() as i64,
    }
}

#[tokio::test]
async fn status_404_on_unknown_request_id() {
    let mock = Arc::new(MockStorageTunnel::new());
    let (addr, _emergency) = spawn_relay_with_mock(mock).await;
    let url = format!("http://{}/v1/emergency/status/no-such-request", addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn full_flow_initiate_to_status_to_handoff() {
    let mock = Arc::new(MockStorageTunnel::new());
    let (addr, emergency) = spawn_relay_with_mock(mock.clone()).await;
    let registered = register(addr).await;
    let rid = registered.rendezvous_id;
    // Now seed the canned HandoffCase response keyed by the actual
    // (registered) rendezvous_id — the one the handoff handler will
    // forward to.
    mock.set_handoff(
        &rid,
        HandoffCaseResponse {
            successor_case_ulid: "01HY_SUCCESSOR".into(),
            successor_operator_label: "Motol ER".into(),
            predecessor_read_only_grant: "ohdg_RO_TOKEN".into(),
        },
    )
    .await;

    // --- Phase 1: seed a waiting emergency request directly. The
    // initiate endpoint requires authority mode; for the no-authority
    // integration test we exercise the persisted-row path the way the
    // initiate handler would after signing.
    let request_id = "req_full_flow";
    let mut row = seed_request(&rid, request_id, 30_000);
    row.default_action = Some(EmergencyDefaultAction::Allow);
    emergency.insert_request(row).await.unwrap();

    // --- Phase 2: the tablet polls and sees `waiting`.
    let url = format!("http://{}/v1/emergency/status/{}", addr, request_id);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let status: EmergencyStatusResponse = resp.json().await.unwrap();
    assert_eq!(status.state, EmergencyState::Waiting);
    assert_eq!(status.grant_token, None);
    assert_eq!(status.case_ulid, None);
    assert_eq!(status.decided_at_ms, None);

    // --- Phase 3: simulate the patient phone approving (the relay's
    // notification handler would do this; we drive it via the table
    // API).
    emergency
        .approve_request(
            request_id,
            "ohdg_PATIENTGRANT".into(),
            "01HX_CASE".into(),
            Some("Patient X".into()),
            now_ms(),
        )
        .await
        .unwrap();

    // --- Phase 4: tablet polls again and sees `approved` + grant.
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let status: EmergencyStatusResponse = resp.json().await.unwrap();
    assert_eq!(status.state, EmergencyState::Approved);
    assert_eq!(status.grant_token.as_deref(), Some("ohdg_PATIENTGRANT"));
    assert_eq!(status.case_ulid.as_deref(), Some("01HX_CASE"));
    assert!(status.decided_at_ms.is_some());

    // --- Phase 5: tablet calls the handoff endpoint after the
    // intervention. The relay forwards to storage (the mock returns a
    // canned response) and returns the audit row.
    let handoff_url = format!("http://{}/v1/emergency/handoff", addr);
    let body = serde_json::json!({
        "source_case_ulid": "01HX_CASE",
        "target_operator": "Motol ER",
        "handoff_note": "intubated en-route",
        "responder_label": "P. Horak",
        "rendezvous_id": rid,
    });
    let resp = reqwest::Client::new()
        .post(&handoff_url)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let handoff: EmergencyHandoffResponse = resp.json().await.unwrap();
    assert_eq!(handoff.successor_case_ulid, "01HY_SUCCESSOR");
    assert_eq!(handoff.successor_operator_label, "Motol ER");
    assert_eq!(handoff.predecessor_read_only_grant, "ohdg_RO_TOKEN");
    assert_eq!(handoff.read_only_grant_token, "ohdg_RO_TOKEN");
    assert!(!handoff.audit_entry_ulid.is_empty());

    // The handoff row should now exist in the audit table.
    assert_eq!(emergency.count_handoffs().await.unwrap(), 1);
    let stored = emergency
        .lookup_handoff_by_source("01HX_CASE")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.successor_case_ulid, "01HY_SUCCESSOR");
    assert_eq!(stored.target_operator, "Motol ER");
    assert_eq!(stored.handoff_note.as_deref(), Some("intubated en-route"));
}

#[tokio::test]
async fn handoff_returns_503_when_storage_tunnel_unconfigured() {
    let relay = RelayState::in_memory().await.unwrap();
    let emergency = EmergencyStateTable::new(relay.registrations.conn_for_emergency());
    let app_state = AppState {
        relay,
        push: Arc::new(PushDispatcher::new()),
        public_host: "127.0.0.1:0".to_string(),
        registration_auth: RegistrationAuthState::permissive(),
        #[cfg(feature = "authority")]
        authority: None,
        emergency,
        storage_tunnel: None,
    };
    let app = build_router(app_state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let registered = register(addr).await;
    let rid = registered.rendezvous_id;
    let url = format!("http://{}/v1/emergency/handoff", addr);
    let body = serde_json::json!({
        "source_case_ulid": "01HX",
        "target_operator": "Motol ER",
        "rendezvous_id": rid,
    });
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let body_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body_json.get("code").and_then(|v| v.as_str()),
        Some("storage_tunnel_unavailable")
    );
}

#[tokio::test]
async fn handoff_404_on_unknown_rendezvous() {
    let mock = Arc::new(MockStorageTunnel::new());
    let (addr, _emergency) = spawn_relay_with_mock(mock).await;
    let url = format!("http://{}/v1/emergency/handoff", addr);
    let body = serde_json::json!({
        "source_case_ulid": "01HX",
        "target_operator": "Motol ER",
        "rendezvous_id": "rzv-nonexistent",
    });
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let body_json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body_json.get("code").and_then(|v| v.as_str()),
        Some("rendezvous_unknown")
    );
}

#[tokio::test]
async fn ttl_sweep_auto_grants_after_timeout_then_status_reflects() {
    let mock = Arc::new(MockStorageTunnel::new());
    let (addr, emergency) = spawn_relay_with_mock(mock).await;
    let _registered = register(addr).await;

    // Seed a request with a TTL that has already elapsed and
    // default_action=Allow — i.e. the patient's emergency profile says
    // "auto-grant on timeout".
    let request_id = "req_auto_grant";
    let mut row = seed_request("rzv-emerg", request_id, -1_000); // already expired
    row.default_action = Some(EmergencyDefaultAction::Allow);
    emergency.insert_request(row).await.unwrap();

    // Drive the sweep deterministically.
    let stats = run_ttl_sweep(&emergency, now_ms()).await.unwrap();
    assert_eq!(stats.auto_granted, 1);

    // Tablet polls and sees auto_granted_timeout + a grant.
    let url = format!("http://{}/v1/emergency/status/{}", addr, request_id);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let status: EmergencyStatusResponse = resp.json().await.unwrap();
    assert_eq!(status.state, EmergencyState::AutoGrantedTimeout);
    assert!(status.grant_token.is_some());
    assert!(status.case_ulid.is_some());
    assert!(status.decided_at_ms.is_some());
}

#[tokio::test]
async fn ttl_sweep_expires_when_default_deny() {
    let mock = Arc::new(MockStorageTunnel::new());
    let (addr, emergency) = spawn_relay_with_mock(mock).await;
    let _registered = register(addr).await;

    let request_id = "req_deny";
    let mut row = seed_request("rzv-emerg", request_id, -1_000);
    row.default_action = Some(EmergencyDefaultAction::Deny);
    emergency.insert_request(row).await.unwrap();

    let stats = run_ttl_sweep(&emergency, now_ms()).await.unwrap();
    assert_eq!(stats.expired, 1);

    let url = format!("http://{}/v1/emergency/status/{}", addr, request_id);
    let status: EmergencyStatusResponse = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status.state, EmergencyState::Expired);
    assert_eq!(status.grant_token, None);
    assert_eq!(status.case_ulid, None);
    assert_eq!(status.rejected_reason.as_deref(), Some("ttl_elapsed"));
}

#[tokio::test]
async fn record_initiated_request_pulls_emergency_config_when_tunnel_present() {
    use ohd_relay::emergency_endpoints::record_initiated_request;
    let mock = Arc::new(MockStorageTunnel::new());
    mock.set_config(
        "rzv-cfg",
        EmergencyConfigResponse {
            default_action: EmergencyDefaultAction::Allow,
            patient_label: Some("Patient Cfg".into()),
        },
    )
    .await;
    let relay = RelayState::in_memory().await.unwrap();
    let emergency = EmergencyStateTable::new(relay.registrations.conn_for_emergency());
    let storage_tunnel: Arc<dyn StorageTunnelClient> = mock;
    let now = now_ms();
    let row = record_initiated_request(
        &emergency,
        Some(&storage_tunnel),
        "req_init".into(),
        "rzv-cfg".into(),
        now + 30_000,
        now,
    )
    .await
    .unwrap();
    assert_eq!(row.default_action, Some(EmergencyDefaultAction::Allow));
    assert_eq!(row.patient_label.as_deref(), Some("Patient Cfg"));

    // After insertion, lookup hits the same row.
    let fetched = emergency.lookup_request("req_init").await.unwrap().unwrap();
    assert_eq!(fetched, row);
}
