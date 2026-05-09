//! BIP39 K_recovery hierarchy: end-to-end create / close / reopen.
//!
//! Per `spec/encryption.md` "Key hierarchy" — files created via
//! `Storage::create_with_mnemonic` derive their SQLCipher key from a 24-word
//! BIP39 phrase + per-file salt. `Storage::open_with_mnemonic` re-derives
//! the same key from the phrase + sidecar salt and reopens the file.

use ohd_storage_core::format::DeploymentMode;
use ohd_storage_core::storage::{salt_sidecar_path, Storage};
use ohd_storage_core::Error;

#[test]
fn create_with_mnemonic_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");

    // Create — fresh mnemonic returned to caller.
    let (storage, mnemonic) =
        Storage::create_with_mnemonic(&path, None, DeploymentMode::Primary, None).unwrap();
    let user_ulid = storage.user_ulid();
    drop(storage);

    let phrase = mnemonic.phrase().to_string();
    assert_eq!(phrase.split_whitespace().count(), 24);

    // Salt sidecar exists.
    let salt_path = salt_sidecar_path(&path);
    assert!(salt_path.exists(), "salt sidecar written next to DB");
    let salt_hex = std::fs::read_to_string(&salt_path).unwrap();
    assert_eq!(salt_hex.trim().len(), 64, "salt is 32 bytes hex");

    // Reopen with the same phrase.
    let reopened = Storage::open_with_mnemonic(&path, &phrase).unwrap();
    assert_eq!(reopened.user_ulid(), user_ulid);
    drop(reopened);
}

#[test]
fn open_with_wrong_mnemonic_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let (storage, _phrase) =
        Storage::create_with_mnemonic(&path, None, DeploymentMode::Primary, None).unwrap();
    drop(storage);

    // Different phrase → SQLCipher refuses to decrypt → error surfaces.
    let other = ohd_storage_core::encryption::generate_mnemonic();
    let result = Storage::open_with_mnemonic(&path, other.phrase());
    assert!(
        result.is_err(),
        "open with a wrong mnemonic must fail (SQLCipher HMAC verify)"
    );
}

#[test]
fn open_with_malformed_phrase_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let (storage, _phrase) =
        Storage::create_with_mnemonic(&path, None, DeploymentMode::Primary, None).unwrap();
    drop(storage);

    let result = Storage::open_with_mnemonic(&path, "totally not a mnemonic");
    assert!(matches!(result, Err(Error::InvalidArgument(_))));
}

#[test]
fn create_with_existing_path_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    std::fs::write(&path, b"existing").unwrap();
    let result = Storage::create_with_mnemonic(&path, None, DeploymentMode::Primary, None);
    assert!(matches!(result, Err(Error::InvalidArgument(_))));
}

#[test]
fn create_with_supplied_phrase_uses_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");

    let supplied = ohd_storage_core::encryption::generate_mnemonic();
    let supplied_phrase = supplied.phrase().to_string();

    let (storage, returned) =
        Storage::create_with_mnemonic(&path, Some(&supplied_phrase), DeploymentMode::Primary, None)
            .unwrap();
    drop(storage);

    assert_eq!(returned.phrase(), supplied_phrase);

    // Confirm reopen with the supplied phrase works.
    let reopened = Storage::open_with_mnemonic(&path, &supplied_phrase).unwrap();
    drop(reopened);
}

#[test]
fn open_without_salt_sidecar_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let (storage, phrase) =
        Storage::create_with_mnemonic(&path, None, DeploymentMode::Primary, None).unwrap();
    let p = phrase.phrase().to_string();
    drop(storage);

    // Remove the sidecar — open must fail with a useful message.
    let salt_path = salt_sidecar_path(&path);
    std::fs::remove_file(&salt_path).unwrap();

    let result = Storage::open_with_mnemonic(&path, &p);
    assert!(matches!(result, Err(Error::InvalidArgument(_))));
}

#[test]
fn deterministic_path_still_works() {
    // `Storage::open` (the deterministic-key path) keeps working alongside
    // BIP39. Both modes coexist. This is the back-compat check.
    use ohd_storage_core::StorageConfig;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let key = vec![0x42u8; 32];
    let storage = Storage::open(StorageConfig::new(&path).with_cipher_key(key.clone())).unwrap();
    let ulid = storage.user_ulid();
    drop(storage);

    // Reopen with the same raw key.
    let reopened = Storage::open(StorageConfig::new(&path).with_cipher_key(key)).unwrap();
    assert_eq!(reopened.user_ulid(), ulid);
}
