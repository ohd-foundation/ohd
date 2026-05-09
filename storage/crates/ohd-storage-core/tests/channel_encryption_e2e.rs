//! End-to-end tests for value-level channel encryption (P6 of the
//! channel-encryption pass).
//!
//! Each test opens a fresh per-user storage file with a 32-byte cipher key,
//! writes events that touch encrypted-class channels, and verifies the
//! on-disk and in-memory shapes.

use ohd_storage_core::auth;
use ohd_storage_core::channel_encryption;
use ohd_storage_core::encryption::{self, EnvelopeKey, NONCE_LEN};
use ohd_storage_core::events::{
    self, ChannelScalar, ChannelValue, EventFilter, EventInput, PutEventResult,
};
use ohd_storage_core::ohdc;
use ohd_storage_core::registry;
use ohd_storage_core::{Storage, StorageConfig};

fn open_test_storage() -> (tempfile::TempDir, Storage) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("e2e.db");
    let key: Vec<u8> = (0u8..32).collect();
    let storage = Storage::open(StorageConfig::new(&path).with_cipher_key(key)).expect("open");
    (dir, storage)
}

/// Seed `std.mood` is already present (sensitivity_class=mental_health). We
/// just confirm the seed and surface its channel path. The default seed has
/// `mood` and `energy` channels (no `value` channel).
fn mood_channel_path() -> &'static str {
    "mood"
}

#[test]
fn migration_creates_class_keys_and_history() {
    let (_dir, storage) = open_test_storage();
    storage
        .with_conn(|conn| {
            // class_keys table populated for every default encrypted class.
            for class in encryption::DEFAULT_ENCRYPTED_CLASSES {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM class_keys WHERE sensitivity_class = ?1",
                        rusqlite::params![class],
                        |r| r.get(0),
                    )
                    .expect("count");
                assert_eq!(count, 1, "class_keys row missing for {class}");
                let nonce_len: i64 = conn
                    .query_row(
                        "SELECT length(nonce) FROM class_keys WHERE sensitivity_class = ?1",
                        rusqlite::params![class],
                        |r| r.get(0),
                    )
                    .expect("nonce");
                assert_eq!(nonce_len as usize, NONCE_LEN);
            }
            // class_key_history mirrors them.
            let hist_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM class_key_history", [], |r| r.get(0))
                .expect("hist count");
            assert_eq!(
                hist_count as usize,
                encryption::DEFAULT_ENCRYPTED_CLASSES.len()
            );
            Ok::<_, ohd_storage_core::Error>(())
        })
        .unwrap();
}

#[test]
fn encrypted_channel_round_trip_via_ohdc() {
    let (_dir, storage) = open_test_storage();
    let _ = mood_channel_path();

    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| auth::issue_self_session_token(conn, user_ulid, Some("mh"), None))
        .unwrap();
    let token = storage
        .with_conn(|conn| auth::resolve_token(conn, &bearer))
        .unwrap();

    let input = EventInput {
        timestamp_ms: 1_700_000_000_000,
        event_type: "std.mood".into(),
        channels: vec![ChannelValue {
            channel_path: "mood".into(),
            value: ChannelScalar::EnumOrdinal { enum_ordinal: 2 },
        }],
        ..Default::default()
    };
    let results = ohdc::put_events(&storage, &token, &[input]).expect("put");
    let ulid_str = match &results[0] {
        PutEventResult::Committed { ulid, .. } => ulid.clone(),
        other => panic!("expected committed: {:?}", other),
    };

    // The on-disk row must have encrypted=1 + non-NULL value_blob; the
    // value_* plaintext columns must all be NULL.
    storage
        .with_conn(|conn| {
            let row: (
                i64,
                Option<Vec<u8>>,
                Option<i64>,
                Option<f64>,
                Option<i64>,
                Option<String>,
                Option<i32>,
            ) = conn
                .query_row(
                    "SELECT encrypted, value_blob, encryption_key_id,
                            value_real, value_int, value_text, value_enum
                       FROM event_channels
                      WHERE event_id = (SELECT id FROM events ORDER BY id DESC LIMIT 1)",
                    [],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                        ))
                    },
                )
                .expect("row");
            assert_eq!(row.0, 1, "encrypted=1");
            assert!(row.1.is_some(), "value_blob set");
            assert!(row.2.is_some(), "encryption_key_id set");
            assert!(row.3.is_none(), "value_real NULL");
            assert!(row.4.is_none(), "value_int NULL");
            assert!(row.5.is_none(), "value_text NULL");
            assert!(row.6.is_none(), "value_enum NULL");
            Ok::<_, ohd_storage_core::Error>(())
        })
        .unwrap();

    // The OHDC read path decrypts.
    let resp = ohdc::query_events(
        &storage,
        &token,
        &EventFilter {
            event_types_in: vec!["std.mood".into()],
            ..Default::default()
        },
    )
    .expect("query");
    assert_eq!(resp.events.len(), 1);
    assert_eq!(resp.events[0].ulid, ulid_str);
    assert_eq!(resp.events[0].channels.len(), 1);
    match &resp.events[0].channels[0].value {
        ChannelScalar::EnumOrdinal { enum_ordinal } => assert_eq!(*enum_ordinal, 2),
        other => panic!("expected enum, got {:?}", other),
    }
}

#[test]
fn encrypted_row_redacted_when_no_envelope_key() {
    let (_dir, storage) = open_test_storage();
    let _ = mood_channel_path();

    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| auth::issue_self_session_token(conn, user_ulid, None, None))
        .unwrap();
    let token = storage
        .with_conn(|conn| auth::resolve_token(conn, &bearer))
        .unwrap();
    let input = EventInput {
        timestamp_ms: 1_700_000_000_000,
        event_type: "std.mood".into(),
        channels: vec![ChannelValue {
            channel_path: "mood".into(),
            value: ChannelScalar::EnumOrdinal { enum_ordinal: 4 },
        }],
        ..Default::default()
    };
    ohdc::put_events(&storage, &token, &[input]).expect("put");

    // Bypass the OHDC layer; call the read path directly without an envelope
    // key. The encrypted channel surfaces as the redacted marker.
    let filter = EventFilter {
        include_superseded: true,
        ..Default::default()
    };
    let (events, _) = storage
        .with_conn(|conn| events::query_events(conn, &filter, None))
        .unwrap();
    assert_eq!(events.len(), 1);
    let value = &events[0].channels[0].value;
    match value {
        ChannelScalar::Text { text_value } => {
            assert!(
                text_value.starts_with("<encrypted:"),
                "expected redacted marker, got {text_value:?}"
            );
            assert!(text_value.contains("mental_health"));
        }
        other => panic!("expected redacted text marker, got {:?}", other),
    }
}

#[test]
fn rotate_class_key_old_blob_still_decrypts() {
    let (_dir, storage) = open_test_storage();
    let _ = mood_channel_path();
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| auth::issue_self_session_token(conn, user_ulid, None, None))
        .unwrap();
    let token = storage
        .with_conn(|conn| auth::resolve_token(conn, &bearer))
        .unwrap();

    // Write event #1 under K_class v1.
    ohdc::put_events(
        &storage,
        &token,
        &[EventInput {
            timestamp_ms: 1_700_000_000_000,
            event_type: "std.mood".into(),
            channels: vec![ChannelValue {
                channel_path: "mood".into(),
                value: ChannelScalar::EnumOrdinal { enum_ordinal: 1 },
            }],
            ..Default::default()
        }],
    )
    .unwrap();

    // Rotate.
    let env = storage.envelope_key().cloned().unwrap();
    storage
        .with_conn_mut(|conn| {
            encryption::rotate_class_key(conn, &env, "mental_health")?;
            Ok(())
        })
        .unwrap();

    // Write event #2 under K_class v2.
    ohdc::put_events(
        &storage,
        &token,
        &[EventInput {
            timestamp_ms: 1_700_000_001_000,
            event_type: "std.mood".into(),
            channels: vec![ChannelValue {
                channel_path: "mood".into(),
                value: ChannelScalar::EnumOrdinal { enum_ordinal: 3 },
            }],
            ..Default::default()
        }],
    )
    .unwrap();

    // Read both back — both must decrypt successfully.
    let resp = ohdc::query_events(
        &storage,
        &token,
        &EventFilter {
            event_types_in: vec!["std.mood".into()],
            ..Default::default()
        },
    )
    .expect("query after rotation");
    assert_eq!(resp.events.len(), 2);
    let mut ordinals: Vec<i32> = resp
        .events
        .iter()
        .map(|e| match &e.channels[0].value {
            ChannelScalar::EnumOrdinal { enum_ordinal } => *enum_ordinal,
            other => panic!("expected enum, got {:?}", other),
        })
        .collect();
    ordinals.sort();
    assert_eq!(ordinals, vec![1, 3]);

    // class_key_history has two non-rotated rows for mental_health (the old
    // is rotated_at_ms set, the new isn't).
    storage
        .with_conn(|conn| {
            let total: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM class_key_history WHERE sensitivity_class = 'mental_health'",
                    [],
                    |r| r.get(0),
                )
                .expect("hist");
            assert_eq!(total, 2);
            let active: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM class_key_history
                      WHERE sensitivity_class = 'mental_health' AND rotated_at_ms IS NULL",
                    [],
                    |r| r.get(0),
                )
                .expect("active");
            assert_eq!(active, 1);
            Ok::<_, ohd_storage_core::Error>(())
        })
        .unwrap();
}

#[test]
fn wrong_key_fails_to_decrypt() {
    let envelope = EnvelopeKey::from_bytes([7u8; 32]);
    let dek = encryption::ClassKey::generate();
    let event_ulid = [3u8; 16];
    let blob = channel_encryption::encrypt_channel_value(
        "value",
        &ChannelScalar::Real { real_value: 1.5 },
        &dek,
        &event_ulid,
        1,
    )
    .expect("encrypt");

    // Use a different key on decrypt.
    let wrong = encryption::ClassKey::from_bytes([99u8; 32]);
    let result = channel_encryption::decrypt_channel_value("value", &blob, &wrong, &event_ulid, 1);
    assert!(matches!(
        result,
        Err(ohd_storage_core::Error::DecryptionFailed)
    ));
    let _ = envelope; // silence unused
}

#[test]
fn grant_with_encrypted_class_carries_wrap_material() {
    use ohd_storage_core::grants::{self, NewGrant, RuleEffect};
    let (_dir, storage) = open_test_storage();
    let _ = mood_channel_path();

    let env = storage.envelope_key().cloned().unwrap();
    let new_grant = NewGrant {
        grantee_label: "Dr Smith".into(),
        grantee_kind: "human".into(),
        approval_mode: "never_required".into(),
        default_action: RuleEffect::Allow,
        sensitivity_rules: vec![("mental_health".to_string(), RuleEffect::Allow)],
        ..Default::default()
    };
    let (grant_id, _grant_ulid) = storage
        .with_conn_mut(|conn| grants::create_grant_with_envelope(conn, &new_grant, &env, None))
        .expect("create grant");
    let row = storage
        .with_conn(|conn| grants::read_grant(conn, grant_id))
        .expect("read grant");
    let wraps = &row.class_key_wraps;
    assert!(
        wraps.contains_key("mental_health"),
        "wrap for mental_health present"
    );
    let wrap = wraps.get("mental_health").unwrap();
    assert_eq!(wrap.nonce.len(), NONCE_LEN);
    // 32 byte DEK + 16 byte AEAD tag = 48 bytes.
    assert_eq!(wrap.ciphertext.len(), 48);
    assert!(wrap.key_id > 0);
}

#[test]
fn grant_denying_class_omits_wrap() {
    use ohd_storage_core::grants::{self, NewGrant, RuleEffect};
    let (_dir, storage) = open_test_storage();
    let _ = mood_channel_path();
    let env = storage.envelope_key().cloned().unwrap();
    let new_grant = NewGrant {
        grantee_label: "Dr Trial".into(),
        grantee_kind: "human".into(),
        approval_mode: "never_required".into(),
        default_action: RuleEffect::Allow,
        // Explicitly deny mental_health.
        sensitivity_rules: vec![("mental_health".to_string(), RuleEffect::Deny)],
        ..Default::default()
    };
    let (grant_id, _) = storage
        .with_conn_mut(|conn| grants::create_grant_with_envelope(conn, &new_grant, &env, None))
        .expect("create");
    let row = storage
        .with_conn(|conn| grants::read_grant(conn, grant_id))
        .expect("read");
    assert!(
        !row.class_key_wraps.contains_key("mental_health"),
        "wrap for denied class must be omitted"
    );
}

#[test]
fn non_encrypted_class_takes_plaintext_path() {
    // std.blood_glucose is sensitivity_class='general' by default — should
    // never go through the encryption pipeline.
    let (_dir, storage) = open_test_storage();
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| auth::issue_self_session_token(conn, user_ulid, None, None))
        .unwrap();
    let token = storage
        .with_conn(|conn| auth::resolve_token(conn, &bearer))
        .unwrap();
    let input = EventInput {
        timestamp_ms: 1_700_000_000_000,
        event_type: "std.blood_glucose".into(),
        channels: vec![ChannelValue {
            channel_path: "value".into(),
            value: ChannelScalar::Real { real_value: 6.4 },
        }],
        ..Default::default()
    };
    ohdc::put_events(&storage, &token, &[input]).expect("put");
    storage
        .with_conn(|conn| {
            let row: (i64, Option<f64>) = conn
                .query_row(
                    "SELECT encrypted, value_real FROM event_channels
                      WHERE event_id = (SELECT id FROM events ORDER BY id DESC LIMIT 1)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .expect("row");
            assert_eq!(row.0, 0, "encrypted=0 for general class");
            assert_eq!(row.1, Some(6.4));
            Ok::<_, ohd_storage_core::Error>(())
        })
        .unwrap();
    let _ = registry::EventTypeName::parse("std.blood_glucose").unwrap();
}
