//! Open-ended-schema flow: unknown event types auto-register under `custom.*`
//! and are transparently found via the same wire name on reads.

use ohd_storage_core::auth;
use ohd_storage_core::events::{ChannelScalar, ChannelValue, EventFilter, EventInput, PutEventResult};
use ohd_storage_core::ohdc;
use ohd_storage_core::{Storage, StorageConfig};

fn open() -> (tempfile::TempDir, Storage, auth::ResolvedToken) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("custom.db");
    let key: Vec<u8> = (0u8..32).collect();
    let storage = Storage::open(StorageConfig::new(&path).with_cipher_key(key)).expect("open");
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| auth::issue_self_session_token(conn, user_ulid, Some("custom-test"), None))
        .expect("issue token");
    let token = storage
        .with_conn(|conn| auth::resolve_token(conn, &bearer))
        .expect("resolve");
    (dir, storage, token)
}

#[test]
fn unknown_type_auto_registers_under_custom_and_is_queryable_by_original_name() {
    let (_dir, storage, token) = open();

    let input = EventInput {
        timestamp_ms: 1_700_000_000_000,
        event_type: "composition.allergen.gluten".into(),
        channels: vec![ChannelValue {
            channel_path: "correlation_id".into(),
            value: ChannelScalar::Text { text_value: "abc".into() },
        }],
        ..Default::default()
    };
    let results = ohdc::put_events(&storage, &token, &[input]).expect("put_events");
    assert!(matches!(results[0], PutEventResult::Committed { .. }));

    // Read using the original (non-custom) name — should transparently resolve
    // through the custom-prefix fallback.
    let resp = ohdc::query_events(
        &storage,
        &token,
        &EventFilter {
            event_types_in: vec!["composition.allergen.gluten".into()],
            ..Default::default()
        },
    )
    .expect("query_events");
    assert_eq!(resp.events.len(), 1, "should find one event under canonical name");
    assert_eq!(resp.events[0].event_type, "custom.composition.allergen.gluten");

    // And direct reads via the custom name itself also work.
    let resp = ohdc::query_events(
        &storage,
        &token,
        &EventFilter {
            event_types_in: vec!["custom.composition.allergen.gluten".into()],
            ..Default::default()
        },
    )
    .expect("query_events");
    assert_eq!(resp.events.len(), 1);
}

#[test]
fn custom_prefixed_input_does_not_double_prefix() {
    let (_dir, storage, token) = open();

    let input = EventInput {
        timestamp_ms: 1_700_000_000_000,
        event_type: "custom.experiment.foo".into(),
        channels: vec![ChannelValue {
            channel_path: "value".into(),
            value: ChannelScalar::Real { real_value: 1.0 },
        }],
        ..Default::default()
    };
    let results = ohdc::put_events(&storage, &token, &[input]).expect("put_events");
    assert!(matches!(results[0], PutEventResult::Committed { .. }));

    let resp = ohdc::query_events(
        &storage,
        &token,
        &EventFilter {
            event_types_in: vec!["custom.experiment.foo".into()],
            ..Default::default()
        },
    )
    .expect("query_events");
    assert_eq!(resp.events.len(), 1);
    assert_eq!(resp.events[0].event_type, "custom.experiment.foo");
}
