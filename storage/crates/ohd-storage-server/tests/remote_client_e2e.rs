//! Phase 1 integration test for the remote-storage feature.
//!
//! Boots an in-process `ohd-storage-server` against a temp DB and drives it
//! end-to-end through the `RemoteOhdStorage` uniffi object — the exact path
//! the OHD Connect Android app will take once Phase 3 swaps the
//! `StorageRepository` backend.
//!
//! The server boot mirrors `tests/end_to_end.rs`: it pulls in the binary's
//! `server.rs` (+ the sibling modules its codegen needs) via `#[path]` so
//! the test exercises the same router the production binary serves.
//!
//! This is a plain `#[test]`, not `#[tokio::test]`: `RemoteOhdStorage` owns
//! its own multi-thread runtime and `block_on`s each RPC, which would panic
//! inside an ambient async context. The server runs on a separate dedicated
//! runtime thread; the test thread stays sync and calls `RemoteOhdStorage`
//! exactly as Kotlin would.

use std::sync::Arc;

use ohd_storage_bindings::{
    CreateGrantInputDto, EventFilterDto, EventInputDto, ListGrantsFilterDto, RemoteOhdStorage,
    ValueKind,
};
use ohd_storage_core::{
    auth::issue_self_session_token,
    storage::{Storage, StorageConfig},
};

// Pull in the binary's `server.rs` + the codegen-emitted proto module so the
// test exercises the same router the production binary uses.
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
#[path = "../src/oauth.rs"]
mod oauth;

mod proto {
    connectrpc::include_generated!();
}

/// Boots the OHDC server on a dedicated multi-thread runtime thread, returns
/// `(base_url, self_session_token)`. The runtime is leaked deliberately —
/// the process exits at test end and joining it would just add ceremony.
fn spawn_server() -> (String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("remote_e2e.db");
    // Keep the temp dir alive for the whole process — leak the handle so the
    // SQLite file isn't unlinked while the server holds it open.
    Box::leak(Box::new(dir));

    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).expect("open storage"));
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, user_ulid, Some("remote-e2e"), None))
        .expect("issue self-session token");

    // Bind synchronously so we know the port before the server task starts.
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = std_listener.local_addr().expect("local_addr");
    std_listener.set_nonblocking(true).expect("nonblocking");

    let router_storage = storage.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("server runtime");
        rt.block_on(async move {
            let listener =
                tokio::net::TcpListener::from_std(std_listener).expect("tokio listener");
            let router = server::router(router_storage);
            let bound = connectrpc::Server::from_listener(listener);
            bound.serve(router).await.expect("server died");
        });
    });

    // Give the accept loop a beat to come up.
    std::thread::sleep(std::time::Duration::from_millis(150));

    (format!("http://{addr}"), bearer)
}

#[test]
fn remote_storage_round_trip() {
    let (base_url, token) = spawn_server();

    // ---- Construct the remote handle exactly as Kotlin will. ----
    let remote = RemoteOhdStorage::connect(base_url.clone(), token.clone())
        .expect("RemoteOhdStorage::connect");

    // ---- whoami: confirms auth + the WhoAmIDto surface. ----
    let who = remote.whoami().expect("whoami");
    assert_eq!(who.token_kind, "self_session");
    assert!(!who.user_ulid.is_empty(), "whoami returned a user ULID");

    // ---- protocol_version (Health RPC under the hood). ----
    let proto_version = remote.protocol_version().expect("protocol_version");
    assert_eq!(proto_version, "ohdc.v0");

    // ---- put_event: write one glucose reading. ----
    let outcome = remote
        .put_event(EventInputDto {
            timestamp_ms: 1_700_000_000_000,
            duration_ms: None,
            tz_offset_minutes: None,
            tz_name: None,
            event_type: "std.blood_glucose".to_string(),
            channels: vec![ohd_storage_bindings::ChannelValueDto {
                channel_path: "value".to_string(),
                value_kind: ValueKind::Real,
                real_value: Some(6.7),
                int_value: None,
                bool_value: None,
                text_value: None,
                enum_ordinal: None,
            }],
            device_id: None,
            app_name: Some("remote-e2e".to_string()),
            app_version: None,
            source: Some("remote-e2e".to_string()),
            source_id: None,
            notes: Some("first remote write".to_string()),
            top_level: Some(true),
        })
        .expect("put_event");
    assert_eq!(outcome.outcome, "committed", "self-session write commits");
    assert!(!outcome.ulid.is_empty(), "committed event has a ULID");
    let written_ulid = outcome.ulid.clone();

    // ---- query_events: read it back. ----
    let events = remote
        .query_events(EventFilterDto {
            from_ms: None,
            to_ms: None,
            event_types_in: vec!["std.blood_glucose".to_string()],
            event_types_not_in: vec![],
            include_deleted: false,
            limit: None,
            visibility: None,
            source_in: vec![],
        })
        .expect("query_events");
    assert_eq!(events.len(), 1, "exactly one glucose event round-trips");
    assert_eq!(events[0].ulid, written_ulid);
    assert_eq!(events[0].event_type, "std.blood_glucose");
    assert_eq!(events[0].notes.as_deref(), Some("first remote write"));
    assert_eq!(
        events[0].channels.first().and_then(|c| c.real_value),
        Some(6.7),
    );

    // ---- count_events: the no-count-RPC fallback path. ----
    let count = remote
        .count_events(EventFilterDto {
            from_ms: None,
            to_ms: None,
            event_types_in: vec!["std.blood_glucose".to_string()],
            event_types_not_in: vec![],
            include_deleted: false,
            limit: None,
            visibility: None,
            source_in: vec![],
        })
        .expect("count_events");
    assert_eq!(count, 1);

    // ---- create_grant: mint a doctor grant over the wire. ----
    let grant_token = remote
        .create_grant(CreateGrantInputDto {
            grantee_label: "Dr. Remote".to_string(),
            grantee_kind: "human".to_string(),
            purpose: Some("phase-1 e2e".to_string()),
            default_action: "deny".to_string(),
            approval_mode: "never_required".to_string(),
            expires_at_ms: None,
            event_type_rules: vec![ohd_storage_bindings::GrantEventTypeRuleDto {
                event_type: "std.blood_glucose".to_string(),
                effect: "allow".to_string(),
            }],
            channel_rules: vec![],
            sensitivity_rules: vec![],
            write_event_type_rules: vec![],
            auto_approve_event_types: vec![],
            aggregation_only: false,
            strip_notes: false,
            notify_on_access: false,
        })
        .expect("create_grant");
    assert!(!grant_token.grant_ulid.is_empty(), "grant has a ULID");
    assert!(
        grant_token.token.starts_with("ohdg_"),
        "grant token is an ohdg_ bearer, got {:?}",
        grant_token.token,
    );

    // ---- list_grants: the freshly-created grant is visible. ----
    let grants = remote
        .list_grants(ListGrantsFilterDto {
            include_revoked: false,
            include_expired: false,
            grantee_kind: None,
            limit: None,
        })
        .expect("list_grants");
    assert!(
        grants.iter().any(|g| g.ulid == grant_token.grant_ulid),
        "created grant appears in list_grants",
    );
    let created = grants
        .iter()
        .find(|g| g.ulid == grant_token.grant_ulid)
        .expect("created grant present");
    assert_eq!(created.grantee_label, "Dr. Remote");
    assert_eq!(created.grantee_kind, "human");
    assert!(
        created
            .event_type_rules
            .iter()
            .any(|r| r.event_type == "std.blood_glucose" && r.effect == "allow"),
        "grant carries the std.blood_glucose allow rule",
    );

    // ---- token refresh path: re-inject the same token, RPC still works. ----
    remote.set_bearer_token(token);
    let who_again = remote.whoami().expect("whoami after set_bearer_token");
    assert_eq!(who_again.user_ulid, who.user_ulid);

    // ---- expired-token signal: a bogus token surfaces an Auth error so the
    //      Android layer can decide whether to refresh. ----
    remote.set_bearer_token("ohds_not_a_real_token".to_string());
    let err = remote
        .whoami()
        .expect_err("bogus token must be rejected");
    match err {
        ohd_storage_bindings::OhdError::Auth { .. } => {}
        other => panic!("expected OhdError::Auth for a bad token, got {other:?}"),
    }
}
