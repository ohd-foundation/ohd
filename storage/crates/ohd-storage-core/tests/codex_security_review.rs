//! Regression tests for the Codex security review fixes (findings #1–#11).
//!
//! Each test pins down a specific finding from the review:
//!   #1  AES-GCM 96-bit nonce birthday bound under long-lived K_class.
//!   #2  Channel AAD too narrow.
//!   #3  Attachment AAD doesn't bind event_id/MIME/filename/size.
//!   #4  Class-key rotation drift.
//!   #5  Zeroize gaps.
//!   #6  Plaintext residency in attachment finalization.
//!   #7  Silent SHA corruption handling.
//!   #8  Low-order X25519 pubkey rejection.
//!   #9  ECDH wrap AAD doesn't bind issuer/grantee/grant_ulid/key_id.
//!   #10 Forbid non-finite floats in signed events.
//!   #11 Reject duplicate channel paths in signed events.

use ohd_storage_core::attachments;
use ohd_storage_core::channel_encryption::{self, EncryptedBlob};
use ohd_storage_core::encryption::{
    self, ClassKey, EnvelopeKey, FileKey, RecoveryKeypair, WrappedClassKey, XNONCE_LEN,
};
use ohd_storage_core::events::{ChannelScalar, ChannelValue, EventInput};
use ohd_storage_core::source_signing;
use ohd_storage_core::storage::{Storage, StorageConfig};
use ohd_storage_core::Error;
use rusqlite::params;
use zeroize::Zeroizing;

fn open_storage(seed: u8) -> (tempfile::TempDir, Storage) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("data.db");
    let storage = Storage::open(StorageConfig::new(&path).with_cipher_key(vec![seed; 32]))
        .expect("open storage");
    (dir, storage)
}

fn seed_event(storage: &Storage) -> (i64, ohd_storage_core::ulid::Ulid) {
    use ohd_storage_core::auth::{issue_self_session_token, resolve_token};
    use ohd_storage_core::ohdc;
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, storage.user_ulid(), Some("t"), None))
        .unwrap();
    let token = storage
        .with_conn(|conn| resolve_token(conn, &bearer))
        .unwrap();
    let ev = EventInput {
        event_type: "std.blood_glucose".into(),
        timestamp_ms: 1_700_000_000_000,
        channels: vec![ChannelValue {
            channel_path: "value".into(),
            value: ChannelScalar::Real { real_value: 5.5 },
        }],
        ..Default::default()
    };
    let outcomes = ohdc::put_events(storage, &token, std::slice::from_ref(&ev)).unwrap();
    let ulid_str = match &outcomes[0] {
        ohd_storage_core::events::PutEventResult::Committed { ulid, .. } => ulid.clone(),
        other => panic!("unexpected: {other:?}"),
    };
    let parsed = ohd_storage_core::ulid::parse_crockford(&ulid_str).unwrap();
    let event_id = storage
        .with_conn(|conn| {
            let rt = ohd_storage_core::ulid::random_tail(&parsed);
            conn.query_row(
                "SELECT id FROM events WHERE ulid_random = ?1",
                params![rt.to_vec()],
                |r| r.get::<_, i64>(0),
            )
            .map_err(Error::from)
        })
        .unwrap();
    (event_id, parsed)
}

// =============================================================================
// #1 — Nonce uniqueness under XChaCha20-Poly1305
// =============================================================================

#[test]
fn codex_1_v2_nonces_are_unique_across_writes() {
    use std::collections::HashSet;
    let key = ClassKey::from_bytes([0xAA; 32]);
    let event_ulid = [9u8; 16];
    let v = ChannelScalar::Int { int_value: 42 };

    // 1000 encrypts under the SAME K_class — every nonce distinct.
    let mut seen = HashSet::new();
    for _ in 0..1000 {
        let blob = channel_encryption::encrypt_channel_value("value", &v, &key, &event_ulid, 1)
            .expect("encrypt");
        // XChaCha20-Poly1305 — 192-bit nonce.
        assert_eq!(blob.nonce.len(), XNONCE_LEN);
        assert!(
            seen.insert(blob.nonce.clone()),
            "nonce collision under same K_class"
        );
    }
}

// =============================================================================
// #2 — Channel AAD binds event_ulid + key_id
// =============================================================================

#[test]
fn codex_2_event_blob_swap_between_events_fails_decryption() {
    // Write event A with channel_path X and value V; copy A's value_blob → event B.
    let (_dir, storage) = open_storage(0x11);

    use ohd_storage_core::auth::{issue_self_session_token, resolve_token};
    use ohd_storage_core::ohdc;
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, storage.user_ulid(), Some("c2"), None))
        .unwrap();
    let token = storage
        .with_conn(|conn| resolve_token(conn, &bearer))
        .unwrap();

    // Two events of std.mood (mental_health → encrypted-class).
    let mk = |ts: i64, ord: i32| EventInput {
        event_type: "std.mood".into(),
        timestamp_ms: ts,
        channels: vec![ChannelValue {
            channel_path: "mood".into(),
            value: ChannelScalar::EnumOrdinal { enum_ordinal: ord },
        }],
        ..Default::default()
    };
    let outcomes_a = ohdc::put_events(&storage, &token, &[mk(1_700_000_000_000, 1)]).unwrap();
    let outcomes_b = ohdc::put_events(&storage, &token, &[mk(1_700_000_001_000, 4)]).unwrap();
    let ulid_a = match &outcomes_a[0] {
        ohd_storage_core::events::PutEventResult::Committed { ulid, .. } => ulid.clone(),
        _ => panic!("expected committed A"),
    };
    let ulid_b = match &outcomes_b[0] {
        ohd_storage_core::events::PutEventResult::Committed { ulid, .. } => ulid.clone(),
        _ => panic!("expected committed B"),
    };

    // Copy A's value_blob → B's row at the SAME channel_id.
    storage
        .with_conn(|conn| {
            let parsed_a = ohd_storage_core::ulid::parse_crockford(&ulid_a).unwrap();
            let parsed_b = ohd_storage_core::ulid::parse_crockford(&ulid_b).unwrap();
            let rt_a = ohd_storage_core::ulid::random_tail(&parsed_a);
            let rt_b = ohd_storage_core::ulid::random_tail(&parsed_b);
            let event_a: i64 = conn
                .query_row(
                    "SELECT id FROM events WHERE ulid_random = ?1",
                    params![rt_a.to_vec()],
                    |r| r.get(0),
                )
                .unwrap();
            let event_b: i64 = conn
                .query_row(
                    "SELECT id FROM events WHERE ulid_random = ?1",
                    params![rt_b.to_vec()],
                    |r| r.get(0),
                )
                .unwrap();
            let (blob_a, key_id_a): (Vec<u8>, i64) = conn
                .query_row(
                    "SELECT value_blob, encryption_key_id
                       FROM event_channels WHERE event_id = ?1",
                    params![event_a],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            // Overwrite B's row with A's blob + key_id.
            conn.execute(
                "UPDATE event_channels
                    SET value_blob = ?1, encryption_key_id = ?2
                  WHERE event_id = ?3",
                params![blob_a, key_id_a, event_b],
            )
            .unwrap();
            Ok::<_, Error>(())
        })
        .unwrap();

    // Reading B should now surface the redacted marker (decrypt fails because
    // the V2 AAD bound A's event ULID — Codex review #2 + the read-side
    // graceful-degradation policy).
    let parsed_b = ohd_storage_core::ulid::parse_crockford(&ulid_b).unwrap();
    let event_b = storage
        .with_conn(|conn| {
            ohd_storage_core::events::get_event_by_ulid_with_key(
                conn,
                &parsed_b,
                storage.envelope_key(),
            )
        })
        .unwrap();
    let value = &event_b.channels[0].value;
    match value {
        ChannelScalar::Text { text_value } => {
            assert!(
                text_value.starts_with("<encrypted:"),
                "swapped blob should fail AEAD verify and surface redacted marker; got {text_value}"
            );
        }
        other => panic!("expected redacted-marker text scalar after blob swap, got {other:?}"),
    }
}

// =============================================================================
// #3 — Attachment AAD binds event_ulid + metadata
// =============================================================================

#[test]
fn codex_3_attachment_relocation_between_events_fails() {
    // Write attachment under event A, then point its `event_id` at event B.
    let (_dir, storage) = open_storage(0x22);
    let (event_a_id, event_a_ulid) = seed_event(&storage);

    // Seed a second event B (same shape).
    use ohd_storage_core::auth::{issue_self_session_token, resolve_token};
    use ohd_storage_core::ohdc;
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, storage.user_ulid(), Some("c3"), None))
        .unwrap();
    let token = storage
        .with_conn(|conn| resolve_token(conn, &bearer))
        .unwrap();
    let ev_b = EventInput {
        event_type: "std.blood_glucose".into(),
        timestamp_ms: 1_700_000_001_000,
        channels: vec![ChannelValue {
            channel_path: "value".into(),
            value: ChannelScalar::Real { real_value: 6.6 },
        }],
        ..Default::default()
    };
    let outcomes_b = ohdc::put_events(&storage, &token, &[ev_b]).unwrap();
    let ulid_b_str = match &outcomes_b[0] {
        ohd_storage_core::events::PutEventResult::Committed { ulid, .. } => ulid.clone(),
        _ => panic!("expected committed B"),
    };
    let event_b_ulid = ohd_storage_core::ulid::parse_crockford(&ulid_b_str).unwrap();
    let event_b_id: i64 = storage
        .with_conn(|conn| {
            let rt = ohd_storage_core::ulid::random_tail(&event_b_ulid);
            conn.query_row(
                "SELECT id FROM events WHERE ulid_random = ?1",
                params![rt.to_vec()],
                |r| r.get(0),
            )
            .map_err(Error::from)
        })
        .unwrap();

    // Attach a blob to event A.
    let _ = event_a_ulid;
    let payload = b"sensitive medical attachment".to_vec();
    let row = ohdc::attach_blob(
        &storage,
        &token,
        &event_a_ulid,
        Some("text/plain".into()),
        Some("note.txt".into()),
        &payload,
        None,
    )
    .expect("attach to A");
    let _ = event_a_id;

    // Verify it reads back under A.
    let (_meta_a, decrypted_a) =
        ohdc::read_attachment_bytes(&storage, &token, &row.ulid).expect("read under A");
    assert_eq!(decrypted_a, payload);

    // Now relocate the attachment row's event_id to B.
    let rt = ohd_storage_core::ulid::random_tail(&row.ulid);
    storage
        .with_conn(|conn| {
            conn.execute(
                "UPDATE attachments SET event_id = ?1 WHERE ulid_random = ?2",
                params![event_b_id, rt.to_vec()],
            )
            .map_err(Error::from)
        })
        .unwrap();

    // Reading should now fail AEAD verify (AAD bound A's event_ulid via V2).
    let result = storage.with_conn(|conn| {
        let root = attachments::sidecar_root_for(storage.path());
        attachments::read_attachment_bytes(conn, &root, &row.ulid, storage.envelope_key())
    });
    assert!(
        matches!(result, Err(Error::DecryptionFailed)),
        "relocation between events must fail AEAD verify, got {result:?}"
    );
}

#[test]
fn codex_3_attachment_metadata_tamper_fails() {
    // Mutating mime_type / filename / byte_size on the row breaks decryption.
    let (_dir, storage) = open_storage(0x23);
    let (_eid, event_ulid) = seed_event(&storage);

    use ohd_storage_core::auth::{issue_self_session_token, resolve_token};
    use ohd_storage_core::ohdc;
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, storage.user_ulid(), Some("c3b"), None))
        .unwrap();
    let token = storage
        .with_conn(|conn| resolve_token(conn, &bearer))
        .unwrap();
    let payload = b"PDF body bytes".to_vec();
    let row = ohdc::attach_blob(
        &storage,
        &token,
        &event_ulid,
        Some("application/pdf".into()),
        Some("scan.pdf".into()),
        &payload,
        None,
    )
    .expect("attach");

    // Tamper with mime_type.
    let rt = ohd_storage_core::ulid::random_tail(&row.ulid);
    storage
        .with_conn(|conn| {
            conn.execute(
                "UPDATE attachments SET mime_type = 'image/jpeg' WHERE ulid_random = ?1",
                params![rt.to_vec()],
            )
            .map_err(Error::from)
        })
        .unwrap();

    let result = storage.with_conn(|conn| {
        let root = attachments::sidecar_root_for(storage.path());
        attachments::read_attachment_bytes(conn, &root, &row.ulid, storage.envelope_key())
    });
    assert!(
        matches!(result, Err(Error::DecryptionFailed)),
        "mime_type tamper must fail AEAD verify, got {result:?}"
    );
}

// =============================================================================
// #4 — Class-key rotation linkage
// =============================================================================

#[test]
fn codex_4_rotation_keeps_current_history_id_consistent() {
    let (_dir, storage) = open_storage(0x44);
    // Bootstrap is run on open. Rotate three times in a row.
    for _ in 0..3 {
        storage
            .with_conn_mut(|conn| {
                let env = storage.envelope_key().cloned().unwrap();
                encryption::rotate_class_key(conn, &env, "mental_health")?;
                Ok::<_, Error>(())
            })
            .unwrap();
    }
    // After every rotation, the live row's `current_history_id` MUST match
    // the most recent non-rotated history row for the class.
    storage
        .with_conn(|conn| {
            let live_history_id: Option<i64> = conn
                .query_row(
                    "SELECT current_history_id FROM class_keys
                      WHERE sensitivity_class = 'mental_health'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let max_unrotated: Option<i64> = conn
                .query_row(
                    "SELECT MAX(id) FROM class_key_history
                      WHERE sensitivity_class = 'mental_health' AND rotated_at_ms IS NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                live_history_id, max_unrotated,
                "current_history_id must point at the live history row after rotation"
            );
            // And `load_active_class_key` agrees.
            let env = storage.envelope_key().unwrap();
            let active = encryption::load_active_class_key(conn, env, "mental_health").unwrap();
            assert_eq!(Some(active.key_id), live_history_id);
            Ok::<_, Error>(())
        })
        .unwrap();
}

// =============================================================================
// #5 — Zeroize gaps
// =============================================================================

#[test]
fn codex_5_filekey_to_hex_returns_zeroizing_string() {
    // Type-level assertion: `to_hex` returns `Zeroizing<String>` (not raw String).
    // This compiles only if the signature matches.
    let fk = FileKey::from_bytes([7u8; 32]);
    let hex: Zeroizing<String> = fk.to_hex();
    assert_eq!(hex.len(), 64); // 32 bytes hex = 64 chars
    assert_eq!(*hex, "07".repeat(32));
}

// =============================================================================
// #6 — Streaming attachment encryption (peak memory bounded)
// =============================================================================

#[test]
fn codex_6_streaming_encrypt_5mib_round_trip() {
    let (_dir, storage) = open_storage(0x66);
    let (_eid, event_ulid) = seed_event(&storage);

    use ohd_storage_core::auth::{issue_self_session_token, resolve_token};
    use ohd_storage_core::ohdc;
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, storage.user_ulid(), Some("c6"), None))
        .unwrap();
    let token = storage
        .with_conn(|conn| resolve_token(conn, &bearer))
        .unwrap();

    // 5 MiB blob — many 64 KiB chunks, exercises stream.
    let mut payload = vec![0u8; 5 * 1024 * 1024];
    for (i, b) in payload.iter_mut().enumerate() {
        *b = (i & 0xff) as u8;
    }
    let row = ohdc::attach_blob(
        &storage,
        &token,
        &event_ulid,
        Some("application/octet-stream".into()),
        Some("big.bin".into()),
        &payload,
        None,
    )
    .expect("attach");
    let (_meta, decrypted) =
        ohdc::read_attachment_bytes(&storage, &token, &row.ulid).expect("read");
    assert_eq!(decrypted.len(), payload.len());
    assert_eq!(decrypted, payload);

    // The on-disk file is plaintext + 19 (stream nonce prefix)
    // + ceil(pt_len / 64KiB) * 16 (per-chunk tags).
    let on_disk_path = {
        let root = attachments::sidecar_root_for(storage.path());
        let hex_sha = hex::encode(row.sha256);
        root.join(&hex_sha[..2]).join(&hex_sha)
    };
    let on_disk = std::fs::read(&on_disk_path).unwrap();
    let chunks = (payload.len() as u64 + (64 * 1024 - 1)) / (64 * 1024);
    let expected = 19 + payload.len() + (chunks as usize) * 16;
    assert_eq!(on_disk.len(), expected, "streaming layout mismatch");
}

// =============================================================================
// #7 — Reject malformed sha256 length
// =============================================================================

#[test]
fn codex_7_malformed_sha256_rejected_loudly() {
    let (_dir, storage) = open_storage(0x77);
    let (_eid, event_ulid) = seed_event(&storage);

    use ohd_storage_core::auth::{issue_self_session_token, resolve_token};
    use ohd_storage_core::ohdc;
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, storage.user_ulid(), Some("c7"), None))
        .unwrap();
    let token = storage
        .with_conn(|conn| resolve_token(conn, &bearer))
        .unwrap();
    let payload = b"some bytes".to_vec();
    let row = ohdc::attach_blob(&storage, &token, &event_ulid, None, None, &payload, None)
        .expect("attach");

    // Inject a malformed sha (15 bytes instead of 32).
    let rt = ohd_storage_core::ulid::random_tail(&row.ulid);
    storage
        .with_conn(|conn| {
            conn.execute(
                "UPDATE attachments SET sha256 = ?1 WHERE ulid_random = ?2",
                params![vec![0xAAu8; 15], rt.to_vec()],
            )
            .map_err(Error::from)
        })
        .unwrap();

    let result = storage.with_conn(|conn| {
        let root = attachments::sidecar_root_for(storage.path());
        attachments::read_attachment_bytes(conn, &root, &row.ulid, storage.envelope_key())
    });
    // Codex review #7: should be a CorruptStorage / Internal error rather
    // than silently routing to a wrong path.
    assert!(
        matches!(result, Err(Error::Internal(_))),
        "malformed sha must be rejected loudly, got {result:?}"
    );
}

// =============================================================================
// #8 — Low-order X25519 pubkey rejection
// =============================================================================

#[test]
fn codex_8_all_zero_grantee_pubkey_rejected() {
    let issuer = RecoveryKeypair::derive_from_file_key(&[0x42u8; 32]);
    let dek = ClassKey::from_bytes([0xCC; 32]);
    let zero_pk = [0u8; 32];
    let result = encryption::wrap_class_key_for_grantee(
        &issuer,
        &zero_pk,
        "mental_health",
        &dek,
        &[1u8; 16],
        1,
    );
    assert!(
        matches!(result, Err(Error::InvalidArgument(ref s)) if s.contains("low-order")),
        "all-zero pubkey must be rejected, got {result:?}"
    );
}

// =============================================================================
// #9 — ECDH wrap AAD binds grant_ulid + key_id; HKDF info binds pubkeys
// =============================================================================

#[test]
fn codex_9_grant_wrap_replay_between_grants_fails() {
    // Same (issuer, grantee, class, key_id), two different grant_ulids:
    // a wrap from grant A is NOT decryptable as grant B's wrap.
    let issuer = RecoveryKeypair::derive_from_file_key(&[0x91u8; 32]);
    let grantee = RecoveryKeypair::derive_from_file_key(&[0x92u8; 32]);
    let dek = ClassKey::from_bytes([0xDD; 32]);
    let grant_a = [1u8; 16];
    let grant_b = [2u8; 16];

    let wrapped = encryption::wrap_class_key_for_grantee(
        &issuer,
        &grantee.public_bytes(),
        "mental_health",
        &dek,
        &grant_a,
        7,
    )
    .expect("wrap for grant A");

    // Try to unwrap as if it were grant B (other inputs unchanged).
    let result = encryption::unwrap_class_key_from_issuer(
        &grantee,
        &issuer.public_bytes(),
        "mental_health",
        &wrapped,
        &grant_b,
        7,
    );
    assert!(
        matches!(result, Err(Error::DecryptionFailed)),
        "wrap from grant A must NOT unwrap as grant B (replay defense), got {result:?}"
    );

    // Sanity: round-trip through grant A succeeds.
    let ok = encryption::unwrap_class_key_from_issuer(
        &grantee,
        &issuer.public_bytes(),
        "mental_health",
        &wrapped,
        &grant_a,
        7,
    )
    .expect("round trip");
    assert_eq!(ok.as_bytes(), dek.as_bytes());
}

#[test]
fn codex_9_grant_wrap_key_id_tamper_fails() {
    let issuer = RecoveryKeypair::derive_from_file_key(&[0x93u8; 32]);
    let grantee = RecoveryKeypair::derive_from_file_key(&[0x94u8; 32]);
    let dek = ClassKey::from_bytes([0xEE; 32]);
    let grant_ulid = [3u8; 16];

    let wrapped = encryption::wrap_class_key_for_grantee(
        &issuer,
        &grantee.public_bytes(),
        "mental_health",
        &dek,
        &grant_ulid,
        10,
    )
    .expect("wrap");
    // Try to unwrap with a different key_id (pretend the wrap is for an
    // older / newer history generation).
    let result = encryption::unwrap_class_key_from_issuer(
        &grantee,
        &issuer.public_bytes(),
        "mental_health",
        &wrapped,
        &grant_ulid,
        11,
    );
    assert!(matches!(result, Err(Error::DecryptionFailed)));
}

// =============================================================================
// #10 — Reject non-finite floats in signed events (canonical CBOR)
// =============================================================================

#[test]
fn codex_10_canonical_cbor_rejects_nan() {
    let event = EventInput {
        event_type: "std.blood_glucose".into(),
        timestamp_ms: 1_700_000_000_000,
        channels: vec![ChannelValue {
            channel_path: "value".into(),
            value: ChannelScalar::Real {
                real_value: f64::NAN,
            },
        }],
        ..Default::default()
    };
    let ulid = [0u8; 16];
    let result = source_signing::canonical_event_bytes(&event, &ulid);
    assert!(
        matches!(result, Err(Error::InvalidArgument(ref s)) if s.contains("non-finite")),
        "NaN must be rejected, got {result:?}"
    );
}

#[test]
fn codex_10_canonical_cbor_rejects_inf() {
    let event = EventInput {
        event_type: "std.blood_glucose".into(),
        timestamp_ms: 1_700_000_000_000,
        channels: vec![ChannelValue {
            channel_path: "value".into(),
            value: ChannelScalar::Real {
                real_value: f64::INFINITY,
            },
        }],
        ..Default::default()
    };
    let ulid = [0u8; 16];
    let result = source_signing::canonical_event_bytes(&event, &ulid);
    assert!(
        matches!(result, Err(Error::InvalidArgument(ref s)) if s.contains("non-finite")),
        "Inf must be rejected, got {result:?}"
    );
}

#[test]
fn codex_10_canonical_cbor_rejects_neg_inf() {
    let event = EventInput {
        event_type: "std.blood_glucose".into(),
        timestamp_ms: 1_700_000_000_000,
        channels: vec![ChannelValue {
            channel_path: "value".into(),
            value: ChannelScalar::Real {
                real_value: f64::NEG_INFINITY,
            },
        }],
        ..Default::default()
    };
    let ulid = [0u8; 16];
    let result = source_signing::canonical_event_bytes(&event, &ulid);
    assert!(matches!(result, Err(Error::InvalidArgument(_))));
}

// =============================================================================
// #11 — Reject duplicate channel paths in signed events
// =============================================================================

#[test]
fn codex_11_canonical_cbor_rejects_duplicate_paths() {
    let event = EventInput {
        event_type: "std.blood_glucose".into(),
        timestamp_ms: 1_700_000_000_000,
        channels: vec![
            ChannelValue {
                channel_path: "value".into(),
                value: ChannelScalar::Real { real_value: 5.5 },
            },
            ChannelValue {
                channel_path: "value".into(),
                value: ChannelScalar::Real { real_value: 6.6 },
            },
        ],
        ..Default::default()
    };
    let ulid = [0u8; 16];
    let result = source_signing::canonical_event_bytes(&event, &ulid);
    assert!(
        matches!(result, Err(Error::InvalidArgument(ref s)) if s.contains("duplicate")),
        "duplicate channel path must be rejected, got {result:?}"
    );
}

// Silence the unused-import lint for items used only conditionally.
#[allow(dead_code)]
fn _suppress_unused() {
    let _ = WrappedClassKey {
        nonce: [0u8; 12],
        ciphertext: vec![],
    };
    let _ = EnvelopeKey::from_bytes([0u8; 32]);
    let _ = EncryptedBlob::from_bytes(&[]);
}
