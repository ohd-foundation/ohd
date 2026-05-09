//! Top-level [`Storage`] handle wrapping a SQLCipher-encrypted SQLite
//! connection plus deployment metadata.
//!
//! # Concurrency
//!
//! v1 keeps the connection inside a `parking_lot`-style mutex via `std::sync::Mutex`;
//! all writes serialize. Read concurrency under WAL is preserved at the SQLite
//! level. A future refactor will split into `Reader` / `Writer` pools per
//! `spec/storage-format.md` "Concurrency".

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::encryption::{self, EnvelopeKey, RecoveryKeypair, K_RECOVERY_SALT_LEN};
use crate::format::{self, DeploymentMode};
use crate::ulid::Ulid;
use crate::{Error, Result};

pub use bip39::Mnemonic;

/// Configuration for opening or creating a per-user storage file.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Filesystem path to `data.db`.
    pub path: PathBuf,
    /// SQLCipher key (32 bytes recommended). Empty → unencrypted (testing only).
    pub cipher_key: Vec<u8>,
    /// Create the file when missing.
    pub create_if_missing: bool,
    /// Deployment mode for newly created files.
    pub create_mode: DeploymentMode,
    /// User ULID to stamp into `_meta.user_ulid` on creation.
    pub create_user_ulid: Option<Ulid>,
}

impl StorageConfig {
    /// Convenience for the in-process smoke test.
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self {
            path: path.into(),
            cipher_key: vec![],
            create_if_missing: true,
            create_mode: DeploymentMode::Primary,
            create_user_ulid: None,
        }
    }

    /// Builder: set cipher key bytes.
    pub fn with_cipher_key(mut self, key: Vec<u8>) -> Self {
        self.cipher_key = key;
        self
    }

    /// Builder: set deployment mode.
    pub fn with_create_mode(mut self, mode: DeploymentMode) -> Self {
        self.create_mode = mode;
        self
    }

    /// Builder: set the user ULID baked into `_meta.user_ulid` on creation.
    pub fn with_user_ulid(mut self, ulid: Ulid) -> Self {
        self.create_user_ulid = Some(ulid);
        self
    }
}

/// Handle to an open per-user storage file.
///
/// Holds the SQLCipher connection plus the live `K_envelope` (the value-level
/// encryption envelope key). The envelope key is derived from the SQLCipher
/// file key at open time via HKDF-SHA256 — see
/// [`encryption::EnvelopeKey::derive_from_file_key`]. v1.x will replace the
/// deterministic derivation with the BIP39 / `K_recovery` hierarchy from
/// `spec/encryption.md`; the storage handle's API surface stays the same.
pub struct Storage {
    inner: Mutex<Connection>,
    path: PathBuf,
    user_ulid: Ulid,
    deployment_mode: DeploymentMode,
    /// Live envelope key for the channel-encryption pipeline. `None` only when
    /// no SQLCipher key is configured (testing-only path); production always
    /// carries an envelope key.
    envelope_key: Option<EnvelopeKey>,
    /// Live X25519 recovery keypair for multi-storage grant re-targeting.
    /// `None` mirrors `envelope_key`'s testing-only path. The pubkey is also
    /// published in `_meta.recovery_pubkey` (idempotent, deterministic from
    /// `K_file`).
    recovery_keypair: Option<RecoveryKeypair>,
}

impl Storage {
    /// Open or create a storage file under `cfg`. Migrations run on first open.
    ///
    /// Side-effects on first open:
    /// - SQLCipher key applied, WAL enabled, schema migrations run.
    /// - `K_envelope` derived from `cfg.cipher_key` via HKDF-SHA256.
    /// - `class_keys` rows for the default encrypted classes are bootstrapped
    ///   (idempotent — pre-existing rows are left alone).
    pub fn open(cfg: StorageConfig) -> Result<Self> {
        let (mut conn, path) = format::open_or_create(format::OpenParams {
            path: &cfg.path,
            cipher_key: &cfg.cipher_key,
            create_if_missing: cfg.create_if_missing,
            create_mode: cfg.create_mode,
            create_user_ulid: cfg.create_user_ulid,
        })?;
        let user_ulid = read_user_ulid(&conn)?;
        let deployment_mode = read_deployment_mode(&conn)?;
        let (envelope_key, recovery_keypair) = if cfg.cipher_key.is_empty() {
            // Testing-only path: derive a constant envelope key from the
            // empty input so the channel-encryption pipeline still functions
            // end-to-end. Production always provides a 32-byte cipher key.
            let fallback = b"ohd.test.fallback";
            (
                Some(EnvelopeKey::derive_from_file_key(fallback)),
                Some(RecoveryKeypair::derive_from_file_key(fallback)),
            )
        } else {
            (
                Some(EnvelopeKey::derive_from_file_key(&cfg.cipher_key)),
                Some(RecoveryKeypair::derive_from_file_key(&cfg.cipher_key)),
            )
        };
        // Bootstrap class_keys for the default encrypted classes.
        if let Some(env) = envelope_key.as_ref() {
            encryption::bootstrap_class_keys(&mut conn, env)?;
        }
        // Publish the recovery pubkey in `_meta.recovery_pubkey` so remote
        // grant issuers can ECDH against it without an out-of-band fetch.
        // Idempotent: same `K_file` always derives the same pubkey.
        if let Some(kp) = recovery_keypair.as_ref() {
            let hex_pub = hex::encode(kp.public_bytes());
            conn.execute(
                "INSERT INTO _meta (key, value) VALUES ('recovery_pubkey', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![hex_pub],
            )?;
        }
        Ok(Self {
            inner: Mutex::new(conn),
            path,
            user_ulid,
            deployment_mode,
            envelope_key,
            recovery_keypair,
        })
    }

    /// Borrow the live envelope key. Returns `None` only in the testing-only
    /// no-cipher-key path; production handles always have one.
    pub fn envelope_key(&self) -> Option<&EnvelopeKey> {
        self.envelope_key.as_ref()
    }

    /// Borrow the live X25519 recovery keypair. `None` mirrors the
    /// testing-only no-cipher-key path. The pubkey is durable across
    /// daemon restarts (deterministic from `K_file`); the seckey lives only
    /// while the storage handle is open.
    pub fn recovery_keypair(&self) -> Option<&RecoveryKeypair> {
        self.recovery_keypair.as_ref()
    }

    /// Path to `data.db`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// User ULID stamped into `_meta.user_ulid`.
    pub fn user_ulid(&self) -> Ulid {
        self.user_ulid
    }

    /// Deployment mode (primary / cache / mirror).
    pub fn deployment_mode(&self) -> DeploymentMode {
        self.deployment_mode
    }

    /// Run a closure with a borrowed connection. v1 serializes via a mutex;
    /// future refactor will split read/write pools.
    pub fn with_conn<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let g = self.inner.lock().expect("storage mutex poisoned");
        f(&g)
    }

    /// Run a closure with a mutable connection (for transactions).
    pub fn with_conn_mut<R>(&self, f: impl FnOnce(&mut Connection) -> Result<R>) -> Result<R> {
        let mut g = self.inner.lock().expect("storage mutex poisoned");
        f(&mut g)
    }

    /// Create a new per-user file whose SQLCipher key is derived from a BIP39
    /// 24-word recovery phrase per `spec/encryption.md` "Key hierarchy".
    ///
    /// Pipeline:
    /// 1. Generate (or accept caller-supplied) `Mnemonic`.
    /// 2. Mint a fresh per-file `k_recovery_salt` (32 bytes CSPRNG).
    /// 3. Run BIP39 standard PBKDF2-HMAC-SHA512 (2048 rounds, empty
    ///    passphrase) to get a 64-byte seed; HKDF-SHA256 expand against the
    ///    salt with `info = b"ohd.v0.file_key"` for the 32-byte `K_file`.
    /// 4. Open SQLCipher with `K_file`, run migrations.
    /// 5. Persist `_meta.k_recovery_salt = <hex>` and `_meta.kdf_mode = 'bip39'`
    ///    so subsequent unlocks can re-derive the same key.
    /// 6. Return `(Storage, Mnemonic)`. **The mnemonic is the user's only
    ///    backup** — UI is responsible for displaying it once and ensuring
    ///    the user copies it.
    ///
    /// If the file already exists, returns [`Error::InvalidArgument`]; this
    /// API is create-only. Use [`Self::open_with_mnemonic`] to reopen.
    pub fn create_with_mnemonic(
        path: &Path,
        mnemonic: Option<&str>,
        mode: DeploymentMode,
        user_ulid: Option<Ulid>,
    ) -> Result<(Self, Mnemonic)> {
        if path.exists() {
            return Err(Error::InvalidArgument(format!(
                "create_with_mnemonic: file already exists at {}",
                path.display()
            )));
        }
        let mnemonic_obj = match mnemonic {
            Some(p) => encryption::parse_mnemonic(p)?,
            None => encryption::generate_mnemonic(),
        };
        let salt = encryption::generate_recovery_salt();
        let file_key = encryption::derive_file_key_from_mnemonic(&mnemonic_obj, &salt, "");
        let cfg = StorageConfig {
            path: path.to_path_buf(),
            cipher_key: file_key.as_bytes().to_vec(),
            create_if_missing: true,
            create_mode: mode,
            create_user_ulid: user_ulid,
        };
        let storage = Self::open(cfg)?;
        // Stamp the BIP39 mode + salt into `_meta` so post-unlock paths can
        // see what mode the file is in.
        storage.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO _meta (key, value) VALUES ('kdf_mode', 'bip39')",
                [],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO _meta (key, value) VALUES ('k_recovery_salt', ?1)",
                params![hex::encode(salt)],
            )?;
            Ok(())
        })?;
        // And write the salt sidecar plaintext next to the DB. The salt isn't
        // secret (the mnemonic carries all the entropy); SQLCipher would
        // refuse to surface anything from the encrypted DB before unlock, so
        // this sidecar is the bootstrap input for `open_with_mnemonic`.
        std::fs::write(salt_sidecar_path(path), hex::encode(salt))?;
        Ok((storage, mnemonic_obj))
    }

    /// Open an existing per-user file whose SQLCipher key is derived from a
    /// BIP39 phrase. Counterpart to [`Self::create_with_mnemonic`].
    ///
    /// Reads the per-file `k_recovery_salt` from `_meta` (a separate
    /// SQLCipher-less peek pass would defeat the encryption — we open the
    /// file with the trial key and let SQLCipher fail loudly if the phrase is
    /// wrong; that's how SQLCipher signals authentication mismatch).
    ///
    /// Errors with [`Error::InvalidArgument`] when the file isn't configured
    /// for BIP39 (`kdf_mode != 'bip39'` after open). For deterministic-key
    /// files use [`Self::open`] with the original `cipher_key`.
    pub fn open_with_mnemonic(path: &Path, mnemonic_phrase: &str) -> Result<Self> {
        if !path.exists() {
            return Err(Error::NotFound);
        }
        let mnemonic_obj = encryption::parse_mnemonic(mnemonic_phrase)?;

        // We don't know the salt without opening the file, but the file's
        // encrypted under K_file derived FROM the salt. Catch-22 resolved by
        // SQLCipher's plaintext header (`_meta` page is encrypted, but the
        // SQLite page-1 KDF salt is in the file header; we use a side-channel
        // tiny SQLite peek with no key to read `_meta.k_recovery_salt` —
        // **wrong**: SQLCipher encrypts page 1 too.
        //
        // The pragmatic approach: keep a sidecar plaintext file
        // `<data.db>.salt` next to the encrypted DB carrying just the
        // `k_recovery_salt`. The salt is non-secret (BIP39 mnemonic provides
        // all the entropy); leaking it doesn't help an attacker. This is
        // explicitly OK per `spec/encryption.md` ("S_kdf … stored in
        // `_meta.cipher_kdf`" — for BIP39 mode we surface it externally so
        // it's reachable before unlock).
        let salt_path = salt_sidecar_path(path);
        let salt_hex = std::fs::read_to_string(&salt_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::InvalidArgument(format!(
                    "open_with_mnemonic: salt sidecar not found at {} \
                     (was the file created with create_with_mnemonic?)",
                    salt_path.display()
                ))
            } else {
                Error::Io(e)
            }
        })?;
        let salt_bytes = hex::decode(salt_hex.trim()).map_err(|_| {
            Error::InvalidArgument("open_with_mnemonic: salt sidecar malformed".into())
        })?;
        if salt_bytes.len() != K_RECOVERY_SALT_LEN {
            return Err(Error::InvalidArgument(
                "open_with_mnemonic: salt sidecar has wrong length".into(),
            ));
        }

        let file_key = encryption::derive_file_key_from_mnemonic(&mnemonic_obj, &salt_bytes, "");
        let cfg = StorageConfig {
            path: path.to_path_buf(),
            cipher_key: file_key.as_bytes().to_vec(),
            create_if_missing: false,
            create_mode: DeploymentMode::Primary, // ignored on open
            create_user_ulid: None,
        };
        let storage = Self::open(cfg)?;
        // Sanity-check kdf_mode. A wrong mnemonic would have failed
        // SQLCipher's HMAC verification by now (cleartext-pragma read would
        // error), but defense in depth: refuse to keep a handle to a non-
        // BIP39 file even if the salt sidecar happens to exist.
        let mode: Option<String> = storage.with_conn(|conn| {
            conn.query_row("SELECT value FROM _meta WHERE key = 'kdf_mode'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(Error::from)
        })?;
        if mode.as_deref() != Some("bip39") {
            return Err(Error::InvalidArgument(
                "open_with_mnemonic: file is not BIP39-keyed (kdf_mode != 'bip39')".into(),
            ));
        }
        Ok(storage)
    }
}

/// Path to the sidecar file holding the (non-secret) `k_recovery_salt` next
/// to a BIP39-keyed `data.db`. Public so callers can include / exclude it in
/// backups deterministically — it's needed alongside the DB to unlock with a
/// mnemonic, but lacks all the entropy on its own.
pub fn salt_sidecar_path(db_path: &Path) -> PathBuf {
    let mut s = db_path.as_os_str().to_owned();
    s.push(".salt");
    PathBuf::from(s)
}

fn read_user_ulid(conn: &Connection) -> Result<Ulid> {
    let hexed: String = conn
        .query_row("SELECT value FROM _meta WHERE key='user_ulid'", [], |r| {
            r.get(0)
        })
        .unwrap_or_default();
    if hexed.is_empty() {
        return Ok([0u8; 16]);
    }
    let bytes = hex::decode(hexed).unwrap_or_default();
    let mut out = [0u8; 16];
    if bytes.len() == 16 {
        out.copy_from_slice(&bytes);
    }
    Ok(out)
}

fn read_deployment_mode(conn: &Connection) -> Result<DeploymentMode> {
    let s: String = conn
        .query_row(
            "SELECT value FROM _meta WHERE key='deployment_mode'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "primary".to_string());
    DeploymentMode::parse(&s)
}
