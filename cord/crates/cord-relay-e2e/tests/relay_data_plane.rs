//! Phase 4f — genuine end-to-end integration test of the relay data plane.
//!
//! The path under test, every hop real:
//!
//! ```text
//!   cord_agent::McpClient (relay transport)
//!        │  wss://127.0.0.1:PORT/v1/attach/<rid>   (WebSocket attach)
//!        ▼
//!   ohd_relay   ── axum router: /v1/register + /v1/attach
//!        │       ── raw QUIC tunnel listener (ALPN ohd-tnl1)
//!        ▼  per-session stream, opaque DATA frames
//!   ohd_relay_client::ShareResponder  (the phone-side share responder)
//!        │  terminates inner TLS 1.3 (storage identity cert, pinned)
//!        ▼  newline-delimited MCP JSON-RPC, ALPN ohd-mcp1
//!   ohd_mcp_core::{catalog_scoped, dispatch_scoped}  scoped by the grant
//!        ▼
//!   ohd_storage_core::Storage   (a real SQLite-backed storage core)
//! ```
//!
//! Phases 4d (responder) and 4e (CORD's relay client) each tested their own
//! half against an in-memory duplex stream. This is the first test that runs
//! the *live* path through a real `ohd-relay` instance — which decodes and
//! re-encodes every OPEN / OPEN_ACK / DATA / CLOSE envelope with its own
//! `TunnelFrame` codec. If the two frame codecs disagree on the wire (magic,
//! header length, payload-length width, type discriminants) the tunnel
//! breaks here and nowhere else.
//!
//! Assertions, per `cord/spec/data-link.md` "The phone-side share responder":
//!
//! 1. `tools/list` returns a non-empty catalog; write tools absent for a
//!    read-only grant.
//! 2. A read tool call returns the in-scope events.
//! 3. A query for a denied event type comes back "not permitted" — not
//!    empty-data, not a crash.
//! 4. A connection built with a WRONG pin fails closed at the inner-TLS
//!    handshake.
//! 5. Suspending the grant mid-session causes subsequent calls to be denied.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use cord_agent::{McpClient, RelayTarget};
use ohd_relay::server::{build_router, AppState};
use ohd_relay::state::RelayState;
use ohd_relay_client::responder::{register_share_rendezvous, ShareResponder};
use ohd_storage_core::events::{put_events, ChannelScalar, ChannelValue, EventInput};
use ohd_storage_core::grants::{NewGrant, RuleEffect};
use ohd_storage_core::{Storage, StorageConfig};
use serde_json::json;
use tokio::sync::watch;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A live relay: the axum router (register + WS attach) on a TCP port and
/// the raw QUIC tunnel listener on a UDP port, sharing one `RelayState`.
struct LiveRelay {
    /// `127.0.0.1:PORT` of the axum HTTP listener — the consumer-attach +
    /// registration surface.
    http_addr: SocketAddr,
    /// `127.0.0.1:PORT` of the QUIC tunnel listener — where the responder
    /// dials its storage tunnel in.
    quic_addr: SocketAddr,
    shutdown: watch::Sender<bool>,
}

/// Spin up a complete relay on two ephemeral ports.
async fn spawn_relay() -> LiveRelay {
    let relay = RelayState::in_memory().await.expect("in-memory relay state");

    // --- axum HTTP listener: /v1/register + /v1/attach (WebSocket) -------
    let emergency = ohd_relay::emergency_endpoints::EmergencyStateTable::new(
        relay.registrations.conn_for_emergency(),
    );
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind relay http");
    let http_addr = http_listener.local_addr().unwrap();
    let app_state = AppState {
        relay: relay.clone(),
        push: Arc::new(ohd_relay::push::PushDispatcher::new()),
        // A literal `127.0.0.1:PORT` host makes the relay compose
        // `ws://` rendezvous URLs (its dev-bind heuristic) — which is
        // exactly what we want for the test.
        public_host: http_addr.to_string(),
        registration_auth: ohd_relay::server::RegistrationAuthState::permissive(),
        // `ohd-relay` is built without the `authority` feature here, so
        // `AppState` carries no `authority` field — nothing to set.
        emergency,
        storage_tunnel: None,
    };
    let app = build_router(app_state);
    tokio::spawn(async move {
        let _ = axum::serve(http_listener, app).await;
    });

    // --- raw QUIC tunnel listener (ALPN ohd-tnl1) ------------------------
    let (cert_chain, key) =
        ohd_relay::http3::dev_self_signed_cert().expect("dev self-signed cert");
    let qcfg = ohd_relay::quic_tunnel::server_config(cert_chain, key)
        .expect("quic server config");
    let endpoint = quinn::Endpoint::server(qcfg, "127.0.0.1:0".parse().unwrap())
        .expect("bind quic tunnel");
    let quic_addr = endpoint.local_addr().unwrap();

    let (shutdown, rx) = watch::channel(false);
    let tunnel_state = Arc::new(relay);
    tokio::spawn(async move {
        let mut sd = rx;
        loop {
            tokio::select! {
                changed = sd.changed() => {
                    if changed.is_err() || *sd.borrow() { break; }
                }
                incoming = endpoint.accept() => {
                    let Some(incoming) = incoming else { break };
                    let st = tunnel_state.clone();
                    let sd = sd.clone();
                    tokio::spawn(async move {
                        let _ = ohd_relay::quic_tunnel::handle_connection_for_test(
                            incoming, st, sd,
                        )
                        .await;
                    });
                }
            }
        }
        endpoint.close(0u32.into(), b"shutdown");
    });

    // Tiny grace period so both listeners are accepting before tests dial.
    tokio::time::sleep(Duration::from_millis(30)).await;

    LiveRelay {
        http_addr,
        quic_addr,
        shutdown,
    }
}

/// Generate a fresh Ed25519 storage identity key in PKCS#8 DER form.
fn fresh_identity_key() -> Vec<u8> {
    rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .expect("generate identity key")
        .serialize_der()
}

/// Open a fresh on-disk storage core, seed it with events of several event
/// types, and create a **read-only** grant whose scope allows
/// `measurement.glucose` + `measurement.heart_rate` and denies everything
/// else (deny default). Returns `(storage, grant_id, _tempdir)`.
///
/// `_tempdir` is returned so the caller keeps the DB file alive for the
/// duration of the test.
fn seeded_storage() -> (Arc<Storage>, i64, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let storage = Storage::open(StorageConfig {
        path: dir.path().join("phone.db"),
        cipher_key: vec![],
        create_if_missing: true,
        create_mode: ohd_storage_core::format::DeploymentMode::Primary,
        create_user_ulid: None,
    })
    .expect("open storage core");

    // Seed top-level events of three distinct types. Two glucose readings
    // are in the read-only grant's scope; the heart-rate reading is too;
    // the headache symptom is out of scope (deny default).
    seed(&storage, "measurement.glucose", 5.4);
    seed(&storage, "measurement.glucose", 6.1);
    seed(&storage, "measurement.heart_rate", 64.0);
    seed(&storage, "symptom.headache", 4.0);

    use ohd_storage_core::grants::create_grant;
    let new_grant = NewGrant {
        grantee_label: "CORD".into(),
        grantee_kind: "agent".into(),
        approval_mode: "never_required".into(),
        // Deny by default — only the explicitly-allowed types are visible.
        default_action: RuleEffect::Deny,
        event_type_rules: vec![
            ("measurement.glucose".into(), RuleEffect::Allow),
            ("measurement.heart_rate".into(), RuleEffect::Allow),
            // symptom.headache is deliberately NOT listed → denied.
        ],
        ..Default::default()
    };
    let (grant_id, _ulid) = storage
        .with_conn_mut(|conn| create_grant(conn, &new_grant))
        .expect("create grant");

    (Arc::new(storage), grant_id, dir)
}

/// Seed one **top-level** event of `event_type` carrying a single numeric
/// `value` channel, timestamped now.
///
/// Written straight through `ohd_storage_core::put_events` with
/// `top_level: true` so the seeded events appear in a normal
/// `query_events` (default `visibility = top_level`) — exactly how a real
/// timeline event behaves.
fn seed(storage: &Storage, event_type: &str, value: f64) {
    let input = EventInput {
        timestamp_ms: ohd_storage_core::format::now_ms(),
        event_type: event_type.to_string(),
        channels: vec![ChannelValue {
            channel_path: "value".to_string(),
            value: ChannelScalar::Real { real_value: value },
        }],
        source: Some("e2e-seed".to_string()),
        top_level: true,
        ..Default::default()
    };
    let envelope_key = storage.envelope_key().cloned();
    let results = storage
        .with_conn_mut(|conn| put_events(conn, &[input], None, false, envelope_key.as_ref()))
        .expect("put_events");
    assert!(
        matches!(
            results.first(),
            Some(ohd_storage_core::events::PutEventResult::Committed { .. })
        ),
        "seeding {event_type} did not commit: {results:?}"
    );
}

/// Register a per-share rendezvous on the relay and start the responder
/// serving the grant's scope. Returns `(RelayTarget, responder_shutdown)`.
///
/// `RelayTarget` is what CORD builds from a share link — rendezvous id,
/// relay host, grant token, cert pin. `pin_override`, when `Some`, replaces
/// the real pin so a caller can exercise the fail-closed path.
async fn start_share(
    relay: &LiveRelay,
    storage: Arc<Storage>,
    grant_id: i64,
    identity_key: Vec<u8>,
    pin_override: Option<String>,
) -> (RelayTarget, watch::Sender<bool>) {
    // The responder registers the per-share rendezvous over real HTTP.
    let relay_origin = format!("http://{}", relay.http_addr);
    let rendezvous = register_share_rendezvous(
        &relay_origin,
        // A 16-byte user ULID, hex.
        "0123456789abcdef0123456789abcdef",
        &identity_key,
        Some("e2e-share".into()),
    )
    .await
    .expect("register share rendezvous");

    let pin = pin_override.unwrap_or_else(|| rendezvous.spki_pin_b64url.clone());

    // Start the responder: it dials the QUIC tunnel and serves scoped MCP.
    let (resp_shutdown, resp_rx) = watch::channel(false);
    let responder = ShareResponder::new(
        storage,
        grant_id,
        identity_key,
        &rendezvous,
        relay.quic_addr.to_string(),
        // The relay's QUIC tunnel uses a dev self-signed cert; accept it.
        true,
    );
    tokio::spawn(async move {
        let _ = responder.serve(resp_rx).await;
    });

    // Wait for the responder's tunnel to register with the relay before
    // the consumer attaches (otherwise the attach push-waits + times out).
    // The relay records the tunnel under the rendezvous id.
    tokio::time::sleep(Duration::from_millis(250)).await;

    let target = RelayTarget {
        // An `http://` scheme makes CORD's `attach_url()` resolve to a
        // plaintext `ws://` attach — correct for this loopback relay,
        // which runs axum without outer TLS (production fronts the relay
        // with Caddy, so a real share link carries `https://`/`wss://`).
        relay_host: format!("http://{}", relay.http_addr),
        rendezvous_id: rendezvous.rendezvous_id.clone(),
        pin,
        token: "ohdg_e2e_test_grant_token".into(),
    };
    (target, resp_shutdown)
}

// ---------------------------------------------------------------------------
// Test 1 — full happy path: catalog scope + in-scope read + denied read.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn relay_data_plane_scoped_catalog_and_calls() {
    let relay = spawn_relay().await;
    let (storage, grant_id, _dir) = seeded_storage();
    let identity_key = fresh_identity_key();
    let (target, resp_shutdown) =
        start_share(&relay, Arc::clone(&storage), grant_id, identity_key, None).await;

    let client = McpClient::relay(target);

    // --- Assertion 1: tools/list is a non-empty, read-only catalog -------
    let tools = client
        .list_tools()
        .await
        .expect("tools/list over the live relay tunnel");
    assert!(!tools.is_empty(), "catalog must not be empty");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"query_events"),
        "read tool present, got {names:?}"
    );
    // Write tools omitted — the grant carries no write rules.
    assert!(
        !names.contains(&"log_food"),
        "write tool must be absent for a read-only grant, got {names:?}"
    );
    assert!(
        !names.contains(&"create_grant"),
        "operator tool must be absent, got {names:?}"
    );

    // --- Assertion 2: an in-scope read returns the seeded events ---------
    let (text, is_error) = client
        .call_tool("query_events", json!({ "event_type": "measurement.glucose" }))
        .await
        .expect("tools/call query_events over the tunnel");
    assert!(!is_error, "in-scope read must not be an error: {text}");
    let body: serde_json::Value = serde_json::from_str(&text).expect("query_events json");
    let count = body
        .get("count")
        .and_then(|c| c.as_u64())
        .expect("count field");
    assert_eq!(count, 2, "both seeded glucose events must be in scope: {body}");

    // --- Assertion 3: a denied event type → "not permitted" --------------
    let (text, is_error) = client
        .call_tool("query_events", json!({ "event_type": "symptom.headache" }))
        .await
        .expect("tools/call for a denied type still completes the RPC");
    assert!(
        is_error,
        "a denied event type must be flagged isError, not return empty data: {text}"
    );
    assert!(
        text.contains("not permitted"),
        "denied read must say 'not permitted', got: {text}"
    );

    let _ = resp_shutdown.send(true);
    let _ = relay.shutdown.send(true);
}

// ---------------------------------------------------------------------------
// Test 2 — wrong pin: the inner-TLS handshake fails closed.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn relay_data_plane_wrong_pin_fails_closed() {
    let relay = spawn_relay().await;
    let (storage, grant_id, _dir) = seeded_storage();
    let identity_key = fresh_identity_key();

    // The pin a consumer would hold for a *different* storage identity —
    // a well-formed pin that simply does not match the responder's cert.
    let wrong_pin = {
        let other_key = fresh_identity_key();
        let ident = ohd_h3_helpers::tls_pin::storage_identity_cert(
            &other_key,
            "relay.example.com/r/wrong",
            1_770_000_000,
        )
        .expect("mint cert for the wrong pin");
        ident.pin_b64url()
    };

    let (target, resp_shutdown) = start_share(
        &relay,
        Arc::clone(&storage),
        grant_id,
        identity_key,
        Some(wrong_pin),
    )
    .await;

    let client = McpClient::relay(target);

    // The pinned verifier is fail-closed: the inner-TLS handshake aborts
    // during certificate verification, surfacing as a connect error — not
    // a silent fallback, not a crash.
    let result = client.list_tools().await;
    let err = result.expect_err("a wrong pin must fail the connection closed");
    let msg = format!("{err}");
    assert!(
        msg.contains("inner TLS") || msg.contains("pin") || msg.contains("handshake"),
        "wrong-pin failure must point at the inner-TLS pin check, got: {msg}"
    );

    let _ = resp_shutdown.send(true);
    let _ = relay.shutdown.send(true);
}

// ---------------------------------------------------------------------------
// Test 3 — suspending the grant mid-session denies subsequent calls.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn relay_data_plane_suspend_denies_subsequent_calls() {
    let relay = spawn_relay().await;
    let (storage, grant_id, _dir) = seeded_storage();
    let identity_key = fresh_identity_key();
    let (target, resp_shutdown) = start_share(
        &relay,
        Arc::clone(&storage),
        grant_id,
        identity_key,
        None,
    )
    .await;

    let client = McpClient::relay(target);

    // Before suspension: an in-scope read succeeds.
    let (text, is_error) = client
        .call_tool("query_events", json!({ "event_type": "measurement.glucose" }))
        .await
        .expect("pre-suspend read");
    assert!(!is_error, "pre-suspend read must succeed: {text}");

    // Flip the grant's `suspended_at_ms`. The responder resolves the share
    // scope fresh per request, so this takes effect on the very next call.
    storage
        .with_conn(|conn| {
            ohd_storage_core::grants::set_grant_suspended(conn, grant_id, true)
        })
        .expect("suspend grant");

    // After suspension: the same read is now denied.
    let (text, is_error) = client
        .call_tool("query_events", json!({ "event_type": "measurement.glucose" }))
        .await
        .expect("post-suspend read still completes the RPC");
    assert!(
        is_error,
        "a suspended grant must deny every read, got non-error: {text}"
    );
    assert!(
        text.contains("not permitted"),
        "suspended read must say 'not permitted', got: {text}"
    );

    let _ = resp_shutdown.send(true);
    let _ = relay.shutdown.send(true);
}
