//! End-to-end smoke test — covers Storage deliverable #5.
//!
//! Flow: open storage → mint a self-session token → put one std.glucose event
//! → query it back → get-by-ULID → verify audit log has at least 3 rows.

use ohd_storage_core::auth;
use ohd_storage_core::events::{
    ChannelScalar, ChannelValue, EventFilter, EventInput, PutEventResult,
};
use ohd_storage_core::ohdc;
use ohd_storage_core::{Storage, StorageConfig};

#[test]
fn smoke_self_session_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("smoke.db");

    // 32-byte cipher key (SQLCipher).
    let key: Vec<u8> = (0u8..32).collect();
    let storage = Storage::open(StorageConfig::new(&path).with_cipher_key(key)).expect("open");
    let user_ulid = storage.user_ulid();
    assert_ne!(user_ulid, [0u8; 16], "user ulid was stamped on creation");

    // Issue a self-session token.
    let bearer = storage
        .with_conn(|conn| auth::issue_self_session_token(conn, user_ulid, Some("smoke"), None))
        .expect("issue token");
    let token = storage
        .with_conn(|conn| auth::resolve_token(conn, &bearer))
        .expect("resolve");

    // Put one event: std.blood_glucose value=6.7 mmol/L.
    let input = EventInput {
        timestamp_ms: 1_700_000_000_000,
        event_type: "std.blood_glucose".into(),
        channels: vec![ChannelValue {
            channel_path: "value".into(),
            value: ChannelScalar::Real { real_value: 6.7 },
        }],
        ..Default::default()
    };
    let results = ohdc::put_events(&storage, &token, &[input]).expect("put_events");
    assert_eq!(results.len(), 1);
    let ulid_str = match &results[0] {
        PutEventResult::Committed { ulid, .. } => ulid.clone(),
        other => panic!("expected committed result, got {:?}", other),
    };

    // Query it back by event_type.
    let resp = ohdc::query_events(
        &storage,
        &token,
        &EventFilter {
            event_types_in: vec!["std.blood_glucose".into()],
            include_superseded: true,
            ..Default::default()
        },
    )
    .expect("query_events");
    assert_eq!(resp.events.len(), 1);
    assert_eq!(resp.events[0].ulid, ulid_str);
    assert_eq!(resp.events[0].channels.len(), 1);

    // Get by ULID.
    let one = ohdc::get_event_by_ulid(&storage, &token, &ulid_str).expect("get");
    assert_eq!(one.ulid, ulid_str);
    assert_eq!(one.event_type, "std.blood_glucose");

    // Audit log should have at least 3 rows: put_events, query_events, get_event_by_ulid.
    use ohd_storage_core::audit;
    let rows = storage
        .with_conn(|conn| audit::query(conn, &audit::AuditQuery::default()))
        .expect("audit query");
    assert!(
        rows.len() >= 3,
        "expected >= 3 audit rows, got {}: {:?}",
        rows.len(),
        rows
    );
}

#[test]
fn smoke_alias_resolution() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("alias.db");
    let storage = Storage::open(StorageConfig::new(&path)).expect("open");

    // 'std.glucose' is an alias for 'std.blood_glucose' per the seed.
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| auth::issue_self_session_token(conn, user_ulid, None, None))
        .expect("issue");
    let token = storage
        .with_conn(|conn| auth::resolve_token(conn, &bearer))
        .expect("resolve");

    let input = EventInput {
        timestamp_ms: 1_700_000_001_000,
        event_type: "std.glucose".into(),
        channels: vec![ChannelValue {
            channel_path: "value".into(),
            value: ChannelScalar::Real { real_value: 7.2 },
        }],
        ..Default::default()
    };
    let results = ohdc::put_events(&storage, &token, &[input]).expect("put_events");
    assert!(matches!(results[0], PutEventResult::Committed { .. }));
}

#[test]
fn smoke_token_kind_matrix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("auth.db");
    let storage = Storage::open(StorageConfig::new(&path)).expect("open");

    // Wrong prefix → unauthenticated.
    let res = storage.with_conn(|conn| auth::resolve_token(conn, "not-a-token"));
    assert!(matches!(res, Err(ohd_storage_core::Error::Unauthenticated)));

    // Random ohds_ token that doesn't exist → unauthenticated.
    let res = storage.with_conn(|conn| auth::resolve_token(conn, "ohds_doesnotexist"));
    assert!(matches!(res, Err(ohd_storage_core::Error::Unauthenticated)));
}
