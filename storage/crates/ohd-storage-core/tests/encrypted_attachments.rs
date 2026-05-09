//! Encrypted attachments at the filesystem level: per-blob DEK wrapped
//! under `K_envelope`, AAD-bound to the attachment's wire ULID + sha256.
//!
//! Covers:
//!   - encrypt + decrypt round-trip
//!   - AAD mismatch (sha256 tampering) → DecryptionFailed
//!   - large blob streaming (> 1 MiB) round-trips
//!   - lazy migration of a pre-existing plaintext attachment

use ohd_storage_core::attachments::{
    self, read_and_lazy_migrate_attachment, read_attachment_bytes,
};
use ohd_storage_core::storage::{Storage, StorageConfig};
use ohd_storage_core::ulid::Ulid;
use ohd_storage_core::Error;
use rusqlite::params;

fn open_storage(name: &str) -> (tempfile::TempDir, Storage) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    let storage =
        Storage::open(StorageConfig::new(&path).with_cipher_key(vec![0x42u8; 32])).expect("open");
    (dir, storage)
}

fn seed_event(storage: &Storage) -> (i64, Ulid) {
    use ohd_storage_core::auth::{issue_self_session_token, resolve_token};
    use ohd_storage_core::events::{ChannelScalar, ChannelValue, EventInput, PutEventResult};
    use ohd_storage_core::ohdc;

    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, storage.user_ulid(), Some("test"), None))
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
        PutEventResult::Committed { ulid, .. } => ulid.clone(),
        other => panic!("expected committed, got {other:?}"),
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

#[test]
fn encrypt_decrypt_round_trip_small() {
    let (_dir, storage) = open_storage("data.db");
    let (event_id, event_ulid) = seed_event(&storage);
    let envelope = storage.envelope_key().unwrap().clone();
    let root = attachments::sidecar_root_for(storage.path());

    let payload = b"hello encrypted world".to_vec();
    let mut writer =
        attachments::new_writer(&root, Some("text/plain".into()), Some("note.txt".into()))
            .unwrap()
            .with_envelope_key(envelope);
    writer.write_chunk(&payload).unwrap();
    let (path, row) = storage
        .with_conn(|conn| writer.finalize(conn, event_id, &event_ulid, None))
        .unwrap();

    // On-disk bytes must NOT equal the plaintext (encryption took effect).
    let on_disk = std::fs::read(&path).unwrap();
    assert_ne!(on_disk, payload, "ciphertext on disk");
    // Length: 19-byte stream nonce prefix + plaintext + 16-byte tag (one chunk).
    assert_eq!(on_disk.len(), payload.len() + 19 + 16);

    // Decrypt via read_attachment_bytes.
    let plaintext = storage
        .with_conn(|conn| read_attachment_bytes(conn, &root, &row.ulid, storage.envelope_key()))
        .unwrap();
    assert_eq!(plaintext, payload);
}

#[test]
fn aad_mismatch_fails_decryption() {
    let (_dir, storage) = open_storage("data.db");
    let (event_id, event_ulid) = seed_event(&storage);
    let envelope = storage.envelope_key().unwrap().clone();
    let root = attachments::sidecar_root_for(storage.path());

    let payload = b"sensitive medical note".to_vec();
    let mut writer = attachments::new_writer(&root, None, None)
        .unwrap()
        .with_envelope_key(envelope);
    writer.write_chunk(&payload).unwrap();
    let (orig_path, row) = storage
        .with_conn(|conn| writer.finalize(conn, event_id, &event_ulid, None))
        .unwrap();

    // To test AAD binding without losing the path lookup, we relocate the
    // ciphertext file to a tampered-sha path and update the row's sha to
    // match. The on-disk path now resolves, but the AEAD AAD uses the
    // tampered sha and decryption fails.
    let tampered_sha = [0xAAu8; 32];
    let hex = hex::encode(tampered_sha);
    let new_dir = root.join(&hex[..2]);
    std::fs::create_dir_all(&new_dir).unwrap();
    let new_path = new_dir.join(&hex);
    std::fs::rename(&orig_path, &new_path).unwrap();

    let rand_tail = ohd_storage_core::ulid::random_tail(&row.ulid);
    storage
        .with_conn(|conn| {
            conn.execute(
                "UPDATE attachments SET sha256 = ?1 WHERE ulid_random = ?2",
                params![tampered_sha.to_vec(), rand_tail.to_vec()],
            )
            .map_err(Error::from)
        })
        .unwrap();

    let result = storage
        .with_conn(|conn| read_attachment_bytes(conn, &root, &row.ulid, storage.envelope_key()));
    assert!(
        matches!(result, Err(Error::DecryptionFailed)),
        "expected DecryptionFailed, got {result:?}"
    );
}

#[test]
fn large_blob_round_trip() {
    // > 1 MiB: 1.5 MiB random-ish data.
    let (_dir, storage) = open_storage("data.db");
    let (event_id, event_ulid) = seed_event(&storage);
    let envelope = storage.envelope_key().unwrap().clone();
    let root = attachments::sidecar_root_for(storage.path());

    let mut payload = vec![0u8; 1_500_000];
    for (i, b) in payload.iter_mut().enumerate() {
        *b = (i & 0xff) as u8;
    }

    let mut writer = attachments::new_writer(&root, None, None)
        .unwrap()
        .with_envelope_key(envelope);
    // Write in 64 KiB chunks to exercise the streaming path.
    for chunk in payload.chunks(64 * 1024) {
        writer.write_chunk(chunk).unwrap();
    }
    let (_path, row) = storage
        .with_conn(|conn| writer.finalize(conn, event_id, &event_ulid, None))
        .unwrap();

    let plaintext = storage
        .with_conn(|conn| read_attachment_bytes(conn, &root, &row.ulid, storage.envelope_key()))
        .unwrap();
    assert_eq!(plaintext.len(), payload.len());
    assert_eq!(plaintext, payload);
}

#[test]
fn lazy_migration_of_plaintext_attachment() {
    let (_dir, storage) = open_storage("data.db");
    let (event_id, event_ulid) = seed_event(&storage);
    let root = attachments::sidecar_root_for(storage.path());

    let payload = b"legacy plaintext attachment".to_vec();

    // Write WITHOUT an envelope key → plaintext on disk + wrapped_dek = NULL.
    let mut writer = attachments::new_writer(&root, None, None).unwrap();
    writer.write_chunk(&payload).unwrap();
    let (path, row) = storage
        .with_conn(|conn| writer.finalize(conn, event_id, &event_ulid, None))
        .unwrap();

    // Confirm initial state.
    let on_disk = std::fs::read(&path).unwrap();
    assert_eq!(on_disk, payload, "file is plaintext before migration");
    let wrapped_initial: Option<Vec<u8>> = storage
        .with_conn(|conn| {
            let rt = ohd_storage_core::ulid::random_tail(&row.ulid);
            conn.query_row(
                "SELECT wrapped_dek FROM attachments WHERE ulid_random = ?1",
                params![rt.to_vec()],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .map_err(Error::from)
        })
        .unwrap();
    assert!(wrapped_initial.is_none(), "wrapped_dek NULL pre-migration");

    // First read with the migration helper: returns plaintext, encrypts in
    // place, updates the row.
    let envelope = storage.envelope_key().unwrap().clone();
    let plaintext = storage
        .with_conn(|conn| read_and_lazy_migrate_attachment(conn, &root, &row.ulid, &envelope))
        .unwrap();
    assert_eq!(plaintext, payload);

    // After migration: file on disk is ciphertext, row carries wrapped_dek.
    let after = std::fs::read(&path).unwrap();
    assert_ne!(after, payload, "file ciphertext post-migration");
    let wrapped_after: Option<Vec<u8>> = storage
        .with_conn(|conn| {
            let rt = ohd_storage_core::ulid::random_tail(&row.ulid);
            conn.query_row(
                "SELECT wrapped_dek FROM attachments WHERE ulid_random = ?1",
                params![rt.to_vec()],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .map_err(Error::from)
        })
        .unwrap();
    assert!(
        wrapped_after.is_some(),
        "wrapped_dek populated post-migration"
    );

    // Subsequent read via read_attachment_bytes returns plaintext.
    let again = storage
        .with_conn(|conn| read_attachment_bytes(conn, &root, &row.ulid, storage.envelope_key()))
        .unwrap();
    assert_eq!(again, payload);
}

#[test]
fn read_without_envelope_key_on_encrypted_row_errors() {
    let (_dir, storage) = open_storage("data.db");
    let (event_id, event_ulid) = seed_event(&storage);
    let envelope = storage.envelope_key().unwrap().clone();
    let root = attachments::sidecar_root_for(storage.path());

    let payload = b"contents".to_vec();
    let mut writer = attachments::new_writer(&root, None, None)
        .unwrap()
        .with_envelope_key(envelope);
    writer.write_chunk(&payload).unwrap();
    let (_path, row) = storage
        .with_conn(|conn| writer.finalize(conn, event_id, &event_ulid, None))
        .unwrap();

    // Reading without an envelope key on an encrypted row must error rather
    // than silently return ciphertext.
    let result = storage.with_conn(|conn| read_attachment_bytes(conn, &root, &row.ulid, None));
    assert!(matches!(result, Err(Error::InvalidArgument(_))));
}

#[test]
fn legacy_plaintext_read_without_envelope_key_succeeds() {
    let (_dir, storage) = open_storage("data.db");
    let (event_id, event_ulid) = seed_event(&storage);
    let root = attachments::sidecar_root_for(storage.path());

    let payload = b"legacy".to_vec();
    let mut writer = attachments::new_writer(&root, None, None).unwrap();
    writer.write_chunk(&payload).unwrap();
    let (_path, row) = storage
        .with_conn(|conn| writer.finalize(conn, event_id, &event_ulid, None))
        .unwrap();

    let bytes = storage
        .with_conn(|conn| read_attachment_bytes(conn, &root, &row.ulid, None))
        .unwrap();
    assert_eq!(bytes, payload);
}
