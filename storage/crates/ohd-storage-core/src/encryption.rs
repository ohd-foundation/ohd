//! Encryption-at-rest and key management.
//!
//! v0 wires SQLCipher's `PRAGMA key` directly with the caller-supplied bytes
//! via [`crate::format::open_or_create`]. On top of that whole-file
//! encryption, this module implements the value-level channel-encryption
//! pipeline from `spec/encryption.md` "End-to-end channel encryption".
//!
//! # Key hierarchy
//!
//! ```text
//! K_recovery (BIP39, user-held; not in this module — v1.x deliverable)
//!     └─> K_envelope (HKDF-SHA256 of K_recovery; daemon receives at unlock)
//!             └─> K_class[mental_health]   (AES-256 DEK)
//!             └─> K_class[sexual_health]   (AES-256 DEK)
//!             └─> K_class[substance_use]   (AES-256 DEK)
//!             └─> K_class[reproductive]    (AES-256 DEK)
//! ```
//!
//! v1 simplification: the BIP39 `K_recovery` and the full multi-device
//! key-handoff flow are still v1.x. `K_envelope` is derived deterministically
//! from `K_file` (the SQLCipher key) at first open via HKDF-SHA256 with a
//! fixed info string. The daemon never persists `K_envelope` cleartext — it
//! lives in `Zeroizing<[u8; 32]>` for the duration of a session and is
//! recomputed on each open. Replacing the deterministic derivation with the
//! real BIP39 hierarchy is a one-function swap (`derive_envelope_key`); the
//! pipeline below stays unchanged.
//!
//! # Channel encryption pipeline (write side)
//!
//! 1. Caller submits an event with channels.
//! 2. For each channel whose `sensitivity_class` is in the encrypted-classes
//!    set, [`crate::channel_encryption::encrypt_channel_value`] is called
//!    with the live `K_class` for that class.
//! 3. The function CBOR-encodes the [`crate::events::ChannelScalar`],
//!    AEAD-encrypts the bytes under AES-256-GCM with a fresh 12-byte nonce,
//!    and returns `(ciphertext_blob, key_id)`.
//! 4. `events::insert_channel_value` stores
//!    `(encrypted=1, value_blob=blob, encryption_key_id=key_id, value_*=NULL)`.
//!
//! # Read side
//!
//! 1. `query_events` reads the row.
//! 2. If `encrypted=1`, the daemon looks up the wrapped DEK in
//!    `class_key_history` by `encryption_key_id`, unwraps it under
//!    `K_envelope`, decrypts the blob, CBOR-decodes back to `ChannelScalar`.
//! 3. The unwrapped DEK lives in `Zeroizing<[u8; 32]>`; it's dropped at the
//!    end of the read transaction.
//!
//! # Grant-side wrap material
//!
//! When a grant scope includes one of the encrypted classes, the
//! `grants.class_key_wraps` BLOB carries a CBOR map
//! `{ sensitivity_class -> wrapped_K_class }`. The grantee (running their
//! own storage handle) holds these wraps and can unwrap each DEK under their
//! own `K_envelope` for decryption. v1 only handles the single-storage case
//! (the user's grants on their own storage); multi-storage grant scenarios
//! are documented as v0.x in STATUS.md.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use bip39::{Language, Mnemonic, MnemonicType};
use hkdf::Hkdf;
use hmac::Hmac;
use rand::rngs::OsRng;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Sha256, Sha512};
use zeroize::{Zeroize, Zeroizing};

use crate::{Error, Result};

/// Default set of sensitivity classes whose channel values are encrypted at
/// rest with a per-class DEK (see `class_keys` table).
///
/// This matches the v1 default per-`spec/encryption.md` "End-to-end channel
/// encryption". The set is intentionally narrow — broadening it has both a
/// CPU and a queryability cost (encrypted values can't be SQL-filtered with
/// `channel_predicates`). Operators can extend the list via
/// [`Storage::with_encrypted_classes`] (TBD; v1 ships the default set).
pub const DEFAULT_ENCRYPTED_CLASSES: &[&str] = &[
    "mental_health",
    "sexual_health",
    "substance_use",
    "reproductive",
];

/// Returns true when this sensitivity class triggers value-level encryption.
///
/// v1 hard-codes the default set. v1.x: per-storage configuration row.
pub fn is_encrypted_class(sensitivity_class: &str) -> bool {
    DEFAULT_ENCRYPTED_CLASSES
        .iter()
        .any(|c| *c == sensitivity_class)
}

/// KDF algorithm choice for unlock (placeholder enum used by future API for
/// the BIP39 / Argon2id replacement of the deterministic derivation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KdfAlgorithm {
    /// SQLCipher 4 default — PBKDF2-SHA512.
    Pbkdf2Sha512,
    /// SQLCipher 5 era — memory-hard.
    Argon2id,
}

/// KDF parameters stored in `_meta.cipher_kdf` (placeholder for v1.x).
#[derive(Debug, Clone)]
pub struct KdfParams {
    /// Algorithm in use.
    pub algorithm: KdfAlgorithm,
    /// Iteration / time cost.
    pub iterations: u32,
    /// Salt (>=128 bits).
    pub salt: Vec<u8>,
}

/// Trait surface a real keystore implementation will fulfil. v1 stubs in a
/// caller-supplied byte buffer; production implementations route to platform
/// keystores (Keychain on iOS, Keystore on Android, OS-specific secrets vault
/// on desktop).
pub trait KeyProvider {
    /// Return the SQLCipher key bytes for this user/file.
    fn unlock(&self) -> Vec<u8>;
}

/// Trivial in-memory provider used by tests.
pub struct StaticKeyProvider(pub Vec<u8>);

impl KeyProvider for StaticKeyProvider {
    fn unlock(&self) -> Vec<u8> {
        self.0.clone()
    }
}

/// Length of the per-class data-encryption key. AES-256 / XChaCha20 → 32 bytes.
pub const DEK_LEN: usize = 32;

/// Length of the AES-GCM nonce. 12 bytes is the AES-GCM standard. Also the
/// size for "wrap" sites (per-class DEK wrap, ECDH grant wrap, attachment
/// DEK wrap) — write volume is bounded so the 96-bit nonce is fine there.
pub const NONCE_LEN: usize = 12;

/// Length of the XChaCha20-Poly1305 nonce (192 bits = 24 bytes). Used for
/// the value-side AEAD (channel values + attachment payloads) to lift the
/// nonce-collision birthday bound from 2^32 messages to "any practical
/// volume". Codex review finding #1.
pub const XNONCE_LEN: usize = 24;

/// HKDF info string used to derive `K_envelope` from `K_file`. Changing this
/// rotates every wrapped class key — treated as a versioning point.
const ENVELOPE_INFO: &[u8] = b"ohd.v0.envelope_key";

/// HKDF info-string prefix used to derive a salt-equivalent for the per-class
/// DEK material. We don't actually derive the DEK from `K_envelope`; we wrap
/// a CSPRNG-generated DEK *under* `K_envelope`. This constant exists so a
/// future implementation that switches to deterministic class keys (e.g.
/// for stateless-client correlation) can do so without changing the on-disk
/// format.
#[allow(dead_code)]
const CLASS_INFO_PREFIX: &[u8] = b"ohd.v0.class_key.";

/// In-memory `K_envelope`. Auto-zeroed on drop.
#[derive(Clone)]
pub struct EnvelopeKey(Zeroizing<[u8; DEK_LEN]>);

impl EnvelopeKey {
    /// Construct from raw bytes. Caller is responsible for zeroing the input.
    pub fn from_bytes(bytes: [u8; DEK_LEN]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Borrow the raw bytes (for AES key construction).
    pub fn as_bytes(&self) -> &[u8; DEK_LEN] {
        &self.0
    }

    /// Derive the v1 deterministic envelope key from a SQLCipher file key.
    ///
    /// HKDF-SHA256(ikm = file_key, info = `b"ohd.v0.envelope_key"`).
    ///
    /// The salt is empty (HKDF's `salt=None` → zeroed PRK salt). For v1 this
    /// is acceptable because `K_file` is already a 32-byte high-entropy CSPRNG
    /// output (or the SQLCipher PBKDF2 of a passphrase). Replacing this with
    /// `K_recovery` + a per-file salt is the v1.x migration; the on-disk
    /// format stays the same since this function only fills the in-memory
    /// `K_envelope`.
    pub fn derive_from_file_key(file_key: &[u8]) -> Self {
        let hk = Hkdf::<Sha256>::new(None, file_key);
        let mut out = [0u8; DEK_LEN];
        hk.expand(ENVELOPE_INFO, &mut out)
            .expect("HKDF expand of 32 bytes never fails");
        Self::from_bytes(out)
    }
}

impl std::fmt::Debug for EnvelopeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("EnvelopeKey").field(&"<redacted>").finish()
    }
}

// =============================================================================
// BIP39 K_recovery hierarchy (P0 of the recovery / mnemonic deliverable).
// =============================================================================

/// Length of `_meta.k_recovery_salt` (HKDF salt for the BIP39 → file-key
/// derivation). 32 bytes = 256 bits, generated once at create time.
pub const K_RECOVERY_SALT_LEN: usize = 32;

/// HKDF info string used to derive `K_file` from a BIP39-derived seed +
/// per-file salt. Distinct from [`ENVELOPE_INFO`] so the file-key and
/// envelope-key namespaces never overlap. Changing this rotates every file
/// key (treated as a versioning point).
const FILE_KEY_INFO: &[u8] = b"ohd.v0.file_key";

/// On-disk SQLCipher file key. 32 bytes (AES-256). Auto-zeroed on drop.
///
/// Distinct from [`EnvelopeKey`]: the file key is what gets passed to
/// SQLCipher's `PRAGMA key`; the envelope key is HKDF-derived from it for
/// channel-encryption wraps. See `spec/encryption.md` "Key hierarchy".
pub struct FileKey(Zeroizing<[u8; DEK_LEN]>);

impl FileKey {
    /// Borrow the raw bytes (for SQLCipher key construction).
    pub fn as_bytes(&self) -> &[u8; DEK_LEN] {
        &self.0
    }

    /// Construct from raw bytes. Caller is responsible for zeroing the input.
    pub fn from_bytes(bytes: [u8; DEK_LEN]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Hex encoding for downstream consumers that need a string form (the
    /// `Storage::open` path consumes a hex-encoded SQLCipher key — see
    /// [`crate::storage::StorageConfig::cipher_key`]).
    ///
    /// Returned in a `Zeroizing<String>` wrapper so the heap allocation
    /// holding the hex-encoded key material is wiped on drop. Codex review
    /// finding #5: the previous `-> String` return type left raw key bytes
    /// hanging around in the heap allocator's free list until reused.
    pub fn to_hex(&self) -> Zeroizing<String> {
        Zeroizing::new(hex::encode(self.as_bytes()))
    }
}

impl std::fmt::Debug for FileKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("FileKey").field(&"<redacted>").finish()
    }
}

/// Generate a fresh random BIP39 mnemonic (24 words = 256 bits of entropy).
///
/// Returned to the caller verbatim; the caller is responsible for displaying
/// it to the user once and never persisting it server-side. A copy of the
/// derived `K_file` lives in `Storage`'s SQLCipher session; the mnemonic
/// itself is not retained.
pub fn generate_mnemonic() -> Mnemonic {
    Mnemonic::new(MnemonicType::Words24, Language::English)
}

/// Validate + parse a user-supplied phrase into a [`Mnemonic`].
///
/// Surfaces [`Error::InvalidArgument`] with a redacted error message
/// (the phrase itself is never logged or echoed back).
pub fn parse_mnemonic(phrase: &str) -> Result<Mnemonic> {
    Mnemonic::from_phrase(phrase.trim(), Language::English)
        .map_err(|_| Error::InvalidArgument("invalid BIP39 mnemonic phrase".into()))
}

/// BIP39 PBKDF2-HMAC-SHA512 iteration count, fixed by BIP39 spec (2048).
const BIP39_PBKDF2_ROUNDS: u32 = 2048;

/// Manually derive the BIP39 seed bytes from a mnemonic phrase + optional
/// passphrase, matching the BIP39 spec exactly:
///
/// - PBKDF2-HMAC-SHA512
/// - password = mnemonic phrase (UTF-8, NFKD-normalized; `tiny-bip39` already
///   normalizes the phrase at parse time)
/// - salt = `"mnemonic" || passphrase`
/// - 2048 rounds
/// - 64-byte output
///
/// Codex review #5: `bip39::Seed::new` allocates an unzeroized `[u8; 64]`
/// internally that lingers in memory until drop. By driving PBKDF2 ourselves
/// we control the buffer allocation and place the result directly into a
/// `Zeroizing<[u8; 64]>`. The `pbkdf2` crate fills a caller-owned buffer
/// (no transient internal copies of the output), so this is the strict
/// upper-bound on plaintext-seed residency in heap memory.
fn derive_bip39_seed(mnemonic: &Mnemonic, bip39_passphrase: &str) -> Zeroizing<[u8; 64]> {
    let mut seed_bytes = Zeroizing::new([0u8; 64]);
    let phrase = mnemonic.phrase();
    let mut salt = Zeroizing::new(Vec::with_capacity(
        b"mnemonic".len() + bip39_passphrase.len(),
    ));
    salt.extend_from_slice(b"mnemonic");
    salt.extend_from_slice(bip39_passphrase.as_bytes());
    pbkdf2::pbkdf2::<Hmac<Sha512>>(
        phrase.as_bytes(),
        salt.as_ref(),
        BIP39_PBKDF2_ROUNDS,
        seed_bytes.as_mut(),
    )
    .expect("PBKDF2 with 64-byte output and HMAC-SHA512 never fails");
    seed_bytes
}

/// Derive the SQLCipher file key from a BIP39 mnemonic + per-file salt.
///
/// Pipeline:
/// 1. PBKDF2-HMAC-SHA512(password = mnemonic phrase,
///    salt = `"mnemonic" || bip39_passphrase`, 2048 rounds) → 64 bytes
///    held in `Zeroizing<[u8; 64]>` (Codex review #5).
/// 2. HKDF-SHA256 with the seed bytes as IKM, the per-file `k_recovery_salt`
///    as salt, and [`FILE_KEY_INFO`] as info → 32 bytes of `K_file`.
///
/// The salt binds the derivation to a specific file: identical mnemonic +
/// different salt = different `K_file`. Two users with the same passphrase
/// (statistically improbable for 24-word phrases but defended in depth)
/// don't share file keys.
///
/// The optional `bip39_passphrase` is BIP39's standard 25th-word stretch.
/// v1 leaves it empty (no UI surface); future deployments can pass a
/// device-bound second factor here.
pub fn derive_file_key_from_mnemonic(
    mnemonic: &Mnemonic,
    salt: &[u8],
    bip39_passphrase: &str,
) -> FileKey {
    let seed_bytes = derive_bip39_seed(mnemonic, bip39_passphrase);
    let hk = Hkdf::<Sha256>::new(Some(salt), seed_bytes.as_ref());
    let mut out = [0u8; DEK_LEN];
    hk.expand(FILE_KEY_INFO, &mut out)
        .expect("HKDF expand of 32 bytes never fails");
    FileKey::from_bytes(out)
}

/// Generate a fresh per-file `k_recovery_salt`. 32 bytes from CSPRNG.
pub fn generate_recovery_salt() -> [u8; K_RECOVERY_SALT_LEN] {
    use rand::RngCore;
    let mut bytes = [0u8; K_RECOVERY_SALT_LEN];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

// =============================================================================
// Per-storage X25519 recovery keypair (multi-storage grant re-targeting).
//
// Each storage publishes a long-lived X25519 pubkey derived from its
// `K_recovery` (BIP39 seed) — or, in the deterministic-key path, from
// `K_file` — via HKDF-SHA256. The pubkey is published in `_meta.recovery_pubkey`
// so a remote storage issuing a grant can ECDH against it without an
// out-of-band fetch. The seckey is held only when the storage is unlocked;
// it never persists.
//
// Re-targeting flow at grant create time (issuer-side):
//   1. Issuer's storage knows the grantee's recovery_pubkey (passed in via
//      CreateGrantRequest; for v1.x the grantee fetches their own pubkey
//      from `_meta.recovery_pubkey` and hands it to the issuer alongside
//      the grant request).
//   2. ECDH(K_recovery_seckey_issuer, recovery_pubkey_grantee) → 32-byte
//      shared secret.
//   3. HKDF-SHA256(salt = b"ohd.v0.grant_kek", info = sensitivity_class) →
//      32-byte wrap KEK.
//   4. AES-256-GCM-encrypt the K_class under the wrap KEK with AAD = the
//      sensitivity class name. Store the ciphertext + the issuer's pubkey
//      in the grant artifact's `class_key_wraps` map.
//
// Unwrap (grantee-side):
//   1. Grantee's storage has K_recovery_seckey (locally derived from its
//      own seed/file key).
//   2. ECDH(K_recovery_seckey_grantee, issuer_recovery_pubkey) → same shared
//      secret.
//   3. HKDF-SHA256 → wrap KEK; AES-GCM decrypt → K_class.
//
// Single-storage backwards-compat: when a grant is issued without a
// grantee_recovery_pubkey, the wrap is under the issuer's K_envelope (the
// pre-existing `wrap_class_key` path). On the wire the discriminator is
// the presence of `grantee_recovery_pubkey` on the grant row.
// =============================================================================

/// HKDF info string used to derive the per-storage X25519 recovery seckey.
/// Distinct from [`ENVELOPE_INFO`] / [`FILE_KEY_INFO`] so the recovery
/// keypair lives in its own namespace.
const RECOVERY_KEYPAIR_INFO: &[u8] = b"ohd.v0.recovery_pubkey";

/// HKDF salt for the ECDH-derived grant wrap KEK. Domain-separates the
/// KEK derivation from any other ECDH use (defended in depth).
const GRANT_KEK_HKDF_SALT: &[u8] = b"ohd.v0.grant_kek";

/// Length of an X25519 keypair component (pubkey or seckey). 32 bytes.
pub const RECOVERY_KEY_LEN: usize = 32;

/// Per-storage X25519 recovery keypair.
///
/// Derived deterministically from `K_file` (or `K_recovery` in the BIP39
/// path) via HKDF-SHA256 with `info = b"ohd.v0.recovery_pubkey"`. The
/// pubkey is published in `_meta.recovery_pubkey`; the seckey lives only
/// in process memory while the storage is unlocked.
#[derive(Clone)]
pub struct RecoveryKeypair {
    secret: x25519_dalek::StaticSecret,
    public: x25519_dalek::PublicKey,
}

impl RecoveryKeypair {
    /// Derive the recovery keypair from the storage's file key. The same
    /// `K_file` always yields the same keypair (deterministic), so an
    /// already-published pubkey stays valid across daemon restarts.
    ///
    /// HKDF-SHA256(ikm = file_key, info = `b"ohd.v0.recovery_pubkey"`) →
    /// 32 bytes → X25519 secret scalar (clamped by `StaticSecret::from`).
    pub fn derive_from_file_key(file_key: &[u8]) -> Self {
        let hk = Hkdf::<Sha256>::new(None, file_key);
        let mut seed = [0u8; RECOVERY_KEY_LEN];
        hk.expand(RECOVERY_KEYPAIR_INFO, &mut seed)
            .expect("HKDF expand of 32 bytes never fails");
        let secret = x25519_dalek::StaticSecret::from(seed);
        // The seed copy in `seed` is on the stack; zero it via Zeroizing.
        let _z = Zeroizing::new(seed);
        let public = x25519_dalek::PublicKey::from(&secret);
        Self { secret, public }
    }

    /// 32-byte public key, suitable for `_meta.recovery_pubkey` and for
    /// publication on grant artifacts.
    pub fn public_bytes(&self) -> [u8; RECOVERY_KEY_LEN] {
        *self.public.as_bytes()
    }

    /// Borrow the wire-shape pubkey.
    pub fn public(&self) -> &x25519_dalek::PublicKey {
        &self.public
    }

    /// Borrow the secret. Only used by ECDH; the secret itself is never
    /// exposed outside this crate (the type wraps it in `StaticSecret`
    /// which zeroes on drop).
    pub fn secret(&self) -> &x25519_dalek::StaticSecret {
        &self.secret
    }
}

impl std::fmt::Debug for RecoveryKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryKeypair")
            .field("public", &hex::encode(self.public_bytes()))
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// Derive the symmetric grant-wrap KEK from an X25519 ECDH shared secret.
///
/// Codex review #9: bind the issuer pubkey and grantee pubkey *into the
/// HKDF info string* (alongside the sensitivity class). Without this
/// binding, two grants between the same issuer and grantee for the same
/// class produce identical KEKs — a wrap stolen from grant A could
/// theoretically be replayed onto grant B's row. Binding the pubkeys
/// (concrete identities of the parties) ties the KEK to "this exact
/// issuer wrapping for this exact grantee for this class" so a wrap-row
/// swap is no longer transparent at the KEK layer.
///
/// HKDF-SHA256(salt = `b"ohd.v0.grant_kek"`, ikm = shared_secret,
/// info = `b"ohd.v0.grant_kek|" || class || "|iss:" || issuer_pubkey ||
/// "|grt:" || grantee_pubkey`) → 32-byte KEK.
fn derive_grant_kek(
    shared_secret: &[u8],
    sensitivity_class: &str,
    issuer_pubkey: &[u8; RECOVERY_KEY_LEN],
    grantee_pubkey: &[u8; RECOVERY_KEY_LEN],
) -> [u8; DEK_LEN] {
    let mut info =
        Vec::with_capacity(b"ohd.v0.grant_kek|".len() + sensitivity_class.len() + 5 + 32 + 5 + 32);
    info.extend_from_slice(b"ohd.v0.grant_kek|");
    info.extend_from_slice(sensitivity_class.as_bytes());
    info.extend_from_slice(b"|iss:");
    info.extend_from_slice(issuer_pubkey);
    info.extend_from_slice(b"|grt:");
    info.extend_from_slice(grantee_pubkey);
    let hk = Hkdf::<Sha256>::new(Some(GRANT_KEK_HKDF_SALT), shared_secret);
    let mut kek = [0u8; DEK_LEN];
    hk.expand(&info, &mut kek)
        .expect("HKDF expand of 32 bytes never fails");
    kek
}

/// Build the AEAD AAD for an ECDH-wrapped class key.
///
/// Codex review #9: the AAD binds the wrap to `(grant_ulid, sensitivity
/// class, class_key_history_id)`. Without `grant_ulid`, a wrap row is
/// replayable between grants that share the same (issuer, grantee, class)
/// — and now that the KEK derivation also binds the pubkeys (above), the
/// AEAD's AAD is the layer that prevents *reissue between two grants by
/// the same parties*. Including `key_id` further pins the wrap to a
/// specific generation in `class_key_history`, so a grant rotation that
/// replaces the live K_class can't have its wrap silently re-bound to an
/// older generation.
fn ecdh_grant_wrap_aad(
    grant_ulid: &[u8; 16],
    sensitivity_class: &str,
    class_key_history_id: i64,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(64 + sensitivity_class.len());
    aad.extend_from_slice(b"ohd.v0.grantwrap:");
    aad.extend_from_slice(grant_ulid);
    aad.extend_from_slice(b"|class:");
    aad.extend_from_slice(sensitivity_class.as_bytes());
    aad.extend_from_slice(b"|key_id:");
    aad.extend_from_slice(class_key_history_id.to_le_bytes().as_ref());
    aad
}

/// Codex review finding #8: reject low-order / all-zero X25519 shared
/// secrets. A grantee that publishes one of the small-order points on
/// Curve25519 (the all-zero pubkey is the trivial case) coerces the
/// shared secret to a fixed value regardless of the issuer's seckey. We
/// detect that and refuse rather than deriving a KEK from it.
fn check_x25519_shared_secret(shared: &x25519_dalek::SharedSecret) -> Result<()> {
    if shared.as_bytes() == &[0u8; 32] {
        return Err(Error::InvalidArgument(
            "low-order or invalid X25519 pubkey".into(),
        ));
    }
    Ok(())
}

/// Wrap a `ClassKey` for delivery to a remote grantee.
///
/// Codex review #9 hardening: the HKDF info now binds `(class, issuer_pk,
/// grantee_pk)`, and the AEAD AAD binds `(grant_ulid, class,
/// class_key_history_id)`. Together this prevents three classes of replay:
///
/// - operator moving a wrap row between grants with the same (issuer,
///   grantee, class) tuple — caught by the AAD `grant_ulid` bind;
/// - operator pointing a wrap at an older `class_key_history` row after a
///   rotation — caught by the AAD `key_id` bind;
/// - operator reusing a wrap row for a different (issuer, grantee) pair —
///   caught by the HKDF info bind on the pubkeys.
///
/// Codex review #8: reject the all-zero / low-order shared secret (an
/// attacker-controlled grantee pubkey could otherwise coerce a known KEK).
pub fn wrap_class_key_for_grantee(
    issuer: &RecoveryKeypair,
    grantee_pubkey: &[u8; RECOVERY_KEY_LEN],
    sensitivity_class: &str,
    class_key: &ClassKey,
    grant_ulid: &[u8; 16],
    class_key_history_id: i64,
) -> Result<WrappedClassKey> {
    let grantee_pk = x25519_dalek::PublicKey::from(*grantee_pubkey);
    let shared = issuer.secret().diffie_hellman(&grantee_pk);
    check_x25519_shared_secret(&shared)?;
    let issuer_pub = issuer.public_bytes();
    let kek = Zeroizing::new(derive_grant_kek(
        shared.as_bytes(),
        sensitivity_class,
        &issuer_pub,
        grantee_pubkey,
    ));
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(kek.as_ref()));
    let nonce_arr = Aes256Gcm::generate_nonce(&mut OsRng);
    let aad = ecdh_grant_wrap_aad(grant_ulid, sensitivity_class, class_key_history_id);
    let ciphertext = cipher
        .encrypt(
            &nonce_arr,
            Payload {
                msg: class_key.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|_| Error::Internal(anyhow::anyhow!("AES-GCM ECDH-grant wrap failed")))?;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(nonce_arr.as_slice());
    Ok(WrappedClassKey { nonce, ciphertext })
}

/// Unwrap a `WrappedClassKey` issued under the X25519/HKDF/AES-GCM scheme of
/// [`wrap_class_key_for_grantee`].
///
/// Mirrors the issuer-side bindings from [`wrap_class_key_for_grantee`]: the
/// caller passes the `grant_ulid` + `class_key_history_id` from the grant
/// row carrying the wrap, and the AEAD verify will fail if any of those
/// values has been tampered with at row level.
pub fn unwrap_class_key_from_issuer(
    grantee: &RecoveryKeypair,
    issuer_pubkey: &[u8; RECOVERY_KEY_LEN],
    sensitivity_class: &str,
    wrapped: &WrappedClassKey,
    grant_ulid: &[u8; 16],
    class_key_history_id: i64,
) -> Result<ClassKey> {
    let issuer_pk = x25519_dalek::PublicKey::from(*issuer_pubkey);
    let shared = grantee.secret().diffie_hellman(&issuer_pk);
    check_x25519_shared_secret(&shared)?;
    let grantee_pub = grantee.public_bytes();
    let kek = Zeroizing::new(derive_grant_kek(
        shared.as_bytes(),
        sensitivity_class,
        issuer_pubkey,
        &grantee_pub,
    ));
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(kek.as_ref()));
    let nonce: &Nonce<<Aes256Gcm as AeadCore>::NonceSize> = Nonce::from_slice(&wrapped.nonce);
    let aad = ecdh_grant_wrap_aad(grant_ulid, sensitivity_class, class_key_history_id);
    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &wrapped.ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| Error::DecryptionFailed)?;
    if plaintext.len() != DEK_LEN {
        let mut p = plaintext;
        p.zeroize();
        return Err(Error::DecryptionFailed);
    }
    let mut bytes = [0u8; DEK_LEN];
    bytes.copy_from_slice(&plaintext);
    let mut p = plaintext;
    p.zeroize();
    Ok(ClassKey::from_bytes(bytes))
}

/// In-memory `K_class` (a per-sensitivity-class DEK). Auto-zeroed on drop.
///
/// The daemon constructs one of these from a `class_keys` row by unwrapping
/// the row's `wrapped_key` under [`EnvelopeKey`]. After the read/write
/// transaction completes, the value is zeroed.
#[derive(Clone)]
pub struct ClassKey(Zeroizing<[u8; DEK_LEN]>);

impl ClassKey {
    /// Construct from raw bytes.
    pub fn from_bytes(bytes: [u8; DEK_LEN]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Generate a fresh DEK from CSPRNG.
    pub fn generate() -> Self {
        let mut bytes = [0u8; DEK_LEN];
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut bytes);
        Self::from_bytes(bytes)
    }

    /// Borrow the raw bytes.
    pub fn as_bytes(&self) -> &[u8; DEK_LEN] {
        &self.0
    }
}

impl std::fmt::Debug for ClassKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ClassKey").field(&"<redacted>").finish()
    }
}

/// On-disk wrap of a `K_class` (the bytes stored in
/// `class_keys.wrapped_key` and `class_key_history.wrapped_key`).
///
/// Format: `nonce (12 bytes) || ciphertext_with_tag (32 + 16 = 48 bytes)`
/// — but on disk the nonce is stored separately in the `nonce` column to
/// keep schema-side audit (e.g. nonce-collision checks) ergonomic.
#[derive(Debug, Clone)]
pub struct WrappedClassKey {
    /// 12-byte AES-GCM nonce (the `nonce` column).
    pub nonce: [u8; NONCE_LEN],
    /// AES-GCM ciphertext + 16-byte tag (32 + 16 = 48 bytes for a 32-byte DEK).
    pub ciphertext: Vec<u8>,
}

/// Wrap a [`ClassKey`] under a [`EnvelopeKey`] for storage in `class_keys`.
///
/// Generates a fresh 12-byte nonce via OsRng. AES-256-GCM authenticated
/// encryption — tampering with the wrapped bytes fails decryption.
///
/// The `aad` is the sensitivity class name; this binds the wrap to its class
/// so a misbehaving operator can't move the wrap row from one class to another
/// without breaking the AEAD tag.
pub fn wrap_class_key(
    envelope: &EnvelopeKey,
    sensitivity_class: &str,
    class_key: &ClassKey,
) -> Result<WrappedClassKey> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(envelope.as_bytes()));
    let nonce_arr = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce_arr,
            Payload {
                msg: class_key.as_bytes(),
                aad: sensitivity_class.as_bytes(),
            },
        )
        .map_err(|_| Error::Internal(anyhow::anyhow!("AES-GCM wrap failed")))?;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(nonce_arr.as_slice());
    Ok(WrappedClassKey { nonce, ciphertext })
}

/// Unwrap a `WrappedClassKey` under a [`EnvelopeKey`]. Returns the cleartext
/// DEK in a `Zeroizing` wrapper (so it auto-clears at scope end).
pub fn unwrap_class_key(
    envelope: &EnvelopeKey,
    sensitivity_class: &str,
    wrapped: &WrappedClassKey,
) -> Result<ClassKey> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(envelope.as_bytes()));
    let nonce: &Nonce<<Aes256Gcm as AeadCore>::NonceSize> = Nonce::from_slice(&wrapped.nonce);
    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &wrapped.ciphertext,
                aad: sensitivity_class.as_bytes(),
            },
        )
        .map_err(|_| Error::DecryptionFailed)?;
    if plaintext.len() != DEK_LEN {
        let mut p = plaintext;
        p.zeroize();
        return Err(Error::DecryptionFailed);
    }
    let mut bytes = [0u8; DEK_LEN];
    bytes.copy_from_slice(&plaintext);
    let mut p = plaintext;
    p.zeroize();
    Ok(ClassKey::from_bytes(bytes))
}

/// Resolved live-class-key handle returned by [`load_active_class_key`].
///
/// Bundles the unwrapped DEK with the `key_id` (the rowid in
/// `class_key_history`) the caller should stamp onto the encrypted blob's
/// `encryption_key_id` column.
#[derive(Debug)]
pub struct ActiveClassKey {
    /// Unwrapped DEK.
    pub key: ClassKey,
    /// `class_key_history.id` to stamp onto the row.
    pub key_id: i64,
}

/// Idempotently bootstrap the encrypted-classes hierarchy.
///
/// For each class in [`DEFAULT_ENCRYPTED_CLASSES`]:
/// - If a row exists in `class_keys`, leave it untouched.
/// - Otherwise: generate a fresh `K_class`, wrap it under `K_envelope`,
///   insert into both `class_keys` (current) and `class_key_history`
///   (immutable record).
///
/// Safe to call on every open. Returns the number of new keys minted.
pub fn bootstrap_class_keys(conn: &mut Connection, envelope: &EnvelopeKey) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut minted = 0usize;
    for class in DEFAULT_ENCRYPTED_CLASSES {
        let exists: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM class_keys WHERE sensitivity_class = ?1",
                params![*class],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            continue;
        }
        let dek = ClassKey::generate();
        let wrapped = wrap_class_key(envelope, class, &dek)?;
        let now = crate::format::now_ms();
        // Codex review #4: bootstrap stamps the history row first, then
        // points `class_keys.current_history_id` at it inside the same
        // transaction. Reads use `current_history_id` as the single source
        // of truth, so a concurrent rotation can't observe an inconsistent
        // (live row, history id) pair.
        tx.execute(
            "INSERT INTO class_key_history
                (sensitivity_class, wrapped_key, nonce, created_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![*class, wrapped.ciphertext, wrapped.nonce.to_vec(), now],
        )?;
        let history_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO class_keys
                (sensitivity_class, wrapped_key, wrap_alg, nonce, created_at_ms,
                 current_history_id)
             VALUES (?1, ?2, 'aes-256-gcm', ?3, ?4, ?5)",
            params![
                *class,
                wrapped.ciphertext,
                wrapped.nonce.to_vec(),
                now,
                history_id
            ],
        )?;
        minted += 1;
    }
    tx.commit()?;
    Ok(minted)
}

/// Load the active `K_class` for a sensitivity class, returning the unwrapped
/// DEK and the `class_key_history.id` callers stamp onto the encrypted blob.
///
/// The caller's storage handle holds `K_envelope`; this function does the
/// unwrap. The DEK is owned by the returned `ActiveClassKey` and zeroed when
/// it drops.
///
/// Errors with [`Error::NotFound`] when the class isn't in `class_keys` —
/// callers should call [`bootstrap_class_keys`] first.
pub fn load_active_class_key(
    conn: &Connection,
    envelope: &EnvelopeKey,
    sensitivity_class: &str,
) -> Result<ActiveClassKey> {
    // Codex review #4: read `current_history_id` from the same row as the
    // wrapped key. Single SELECT, atomic snapshot — no race with a
    // concurrent rotation that updates one of the two and not the other.
    let row: Option<(Vec<u8>, Vec<u8>, Option<i64>)> = conn
        .query_row(
            "SELECT wrapped_key, nonce, current_history_id FROM class_keys
              WHERE sensitivity_class = ?1 AND rotated_at_ms IS NULL",
            params![sensitivity_class],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (ciphertext, nonce_bytes, history_id) = row.ok_or(Error::NotFound)?;
    if nonce_bytes.len() != NONCE_LEN {
        return Err(Error::DecryptionFailed);
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&nonce_bytes);
    let wrapped = WrappedClassKey { nonce, ciphertext };
    let key = unwrap_class_key(envelope, sensitivity_class, &wrapped)?;
    let key_id = history_id.ok_or_else(|| {
        Error::Internal(anyhow::anyhow!(
            "class_keys.current_history_id NULL for class {sensitivity_class:?} \
             — migration 014 may not have run"
        ))
    })?;
    Ok(ActiveClassKey { key, key_id })
}

/// Load a `K_class` by its history rowid (for decrypting a blob that was
/// written under a previously-active key after a rotation).
pub fn load_class_key_by_id(
    conn: &Connection,
    envelope: &EnvelopeKey,
    sensitivity_class: &str,
    key_id: i64,
) -> Result<ClassKey> {
    let row: Option<(Vec<u8>, Vec<u8>)> = conn
        .query_row(
            "SELECT wrapped_key, nonce FROM class_key_history
              WHERE id = ?1 AND sensitivity_class = ?2",
            params![key_id, sensitivity_class],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (ciphertext, nonce_bytes) = row.ok_or(Error::NotFound)?;
    if nonce_bytes.len() != NONCE_LEN {
        return Err(Error::DecryptionFailed);
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&nonce_bytes);
    unwrap_class_key(
        envelope,
        sensitivity_class,
        &WrappedClassKey { nonce, ciphertext },
    )
}

/// Rotate the active DEK for a sensitivity class.
///
/// Marks the previous `class_key_history` row as rotated, mints a fresh DEK
/// (wrapped under the same `K_envelope`), inserts a new `class_key_history`
/// row, and **replaces** the singleton `class_keys` row (PRIMARY KEY on
/// `sensitivity_class`). Existing encrypted blobs reference `class_key_history.id`
/// so they keep decrypting under the old DEK.
///
/// Returns the new `class_key_history.id` for the freshly minted key.
pub fn rotate_class_key(
    conn: &mut Connection,
    envelope: &EnvelopeKey,
    sensitivity_class: &str,
) -> Result<i64> {
    let tx = conn.transaction()?;
    let now = crate::format::now_ms();
    // Mark the previous history row as rotated. The class_keys row is
    // replaced in place below — its `rotated_at_ms` column tracks the last
    // rotation time but the PRIMARY KEY constraint forbids a second row, so
    // we use UPDATE…SET wrapped_key = …, rotated_at_ms = NULL.
    tx.execute(
        "UPDATE class_key_history SET rotated_at_ms = ?1
          WHERE sensitivity_class = ?2 AND rotated_at_ms IS NULL",
        params![now, sensitivity_class],
    )?;

    // Mint and wrap a new DEK.
    let dek = ClassKey::generate();
    let wrapped = wrap_class_key(envelope, sensitivity_class, &dek)?;

    // Insert the new history row first so the live row's logical mapping is
    // unambiguous (the new history.id is what new writes will stamp).
    tx.execute(
        "INSERT INTO class_key_history
            (sensitivity_class, wrapped_key, nonce, created_at_ms)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            sensitivity_class,
            wrapped.ciphertext,
            wrapped.nonce.to_vec(),
            now
        ],
    )?;
    let new_history_id = tx.last_insert_rowid();

    // Replace the singleton live row in-place (PRIMARY KEY on
    // sensitivity_class). `rotated_at_ms` is cleared because this row IS the
    // new live key. Codex review #4: also stamp `current_history_id` so the
    // (live wrapped DEK, history pointer) pair is always consistent.
    tx.execute(
        "UPDATE class_keys
            SET wrapped_key = ?1, wrap_alg = 'aes-256-gcm', nonce = ?2,
                created_at_ms = ?3, rotated_at_ms = NULL,
                current_history_id = ?4
          WHERE sensitivity_class = ?5",
        params![
            wrapped.ciphertext,
            wrapped.nonce.to_vec(),
            now,
            new_history_id,
            sensitivity_class
        ],
    )?;
    tx.commit()?;
    Ok(new_history_id)
}

/// Convenience helper used by transactional write paths.
///
/// Same as [`load_active_class_key`] but operates inside an open transaction.
pub fn load_active_class_key_tx(
    tx: &Transaction<'_>,
    envelope: &EnvelopeKey,
    sensitivity_class: &str,
) -> Result<ActiveClassKey> {
    // Codex review #4: same atomic-snapshot invariant as
    // [`load_active_class_key`]; both code paths consult `current_history_id`.
    let row: Option<(Vec<u8>, Vec<u8>, Option<i64>)> = tx
        .query_row(
            "SELECT wrapped_key, nonce, current_history_id FROM class_keys
              WHERE sensitivity_class = ?1 AND rotated_at_ms IS NULL",
            params![sensitivity_class],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (ciphertext, nonce_bytes, history_id) = row.ok_or(Error::NotFound)?;
    if nonce_bytes.len() != NONCE_LEN {
        return Err(Error::DecryptionFailed);
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&nonce_bytes);
    let wrapped = WrappedClassKey { nonce, ciphertext };
    let key = unwrap_class_key(envelope, sensitivity_class, &wrapped)?;
    let key_id = history_id.ok_or_else(|| {
        Error::Internal(anyhow::anyhow!(
            "class_keys.current_history_id NULL for class {sensitivity_class:?} \
             — migration 014 may not have run"
        ))
    })?;
    Ok(ActiveClassKey { key, key_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_derivation_is_deterministic() {
        let file_key = [7u8; 32];
        let a = EnvelopeKey::derive_from_file_key(&file_key);
        let b = EnvelopeKey::derive_from_file_key(&file_key);
        assert_eq!(a.as_bytes(), b.as_bytes());
        // Different file key → different envelope key.
        let c = EnvelopeKey::derive_from_file_key(&[8u8; 32]);
        assert_ne!(a.as_bytes(), c.as_bytes());
    }

    #[test]
    fn wrap_unwrap_round_trip() {
        let envelope = EnvelopeKey::from_bytes([1u8; 32]);
        let dek = ClassKey::from_bytes([42u8; 32]);
        let wrapped = wrap_class_key(&envelope, "mental_health", &dek).unwrap();
        let unwrapped = unwrap_class_key(&envelope, "mental_health", &wrapped).unwrap();
        assert_eq!(unwrapped.as_bytes(), dek.as_bytes());
    }

    #[test]
    fn wrong_envelope_fails() {
        let envelope = EnvelopeKey::from_bytes([1u8; 32]);
        let wrong = EnvelopeKey::from_bytes([2u8; 32]);
        let dek = ClassKey::from_bytes([42u8; 32]);
        let wrapped = wrap_class_key(&envelope, "mental_health", &dek).unwrap();
        let result = unwrap_class_key(&wrong, "mental_health", &wrapped);
        assert!(matches!(result, Err(Error::DecryptionFailed)));
    }

    #[test]
    fn aad_class_binding() {
        // Wrapping under one class but unwrapping with a different class AAD
        // fails — operator can't move wrap material between classes.
        let envelope = EnvelopeKey::from_bytes([1u8; 32]);
        let dek = ClassKey::from_bytes([42u8; 32]);
        let wrapped = wrap_class_key(&envelope, "mental_health", &dek).unwrap();
        let result = unwrap_class_key(&envelope, "sexual_health", &wrapped);
        assert!(matches!(result, Err(Error::DecryptionFailed)));
    }

    #[test]
    fn is_encrypted_class_matrix() {
        assert!(is_encrypted_class("mental_health"));
        assert!(is_encrypted_class("sexual_health"));
        assert!(is_encrypted_class("substance_use"));
        assert!(is_encrypted_class("reproductive"));
        assert!(!is_encrypted_class("general"));
        assert!(!is_encrypted_class("identifying"));
        assert!(!is_encrypted_class(""));
    }

    // -----------------------------------------------------------------------
    // BIP39 K_recovery hierarchy
    // -----------------------------------------------------------------------

    #[test]
    fn generate_mnemonic_is_24_words() {
        let m = generate_mnemonic();
        let words: Vec<&str> = m.phrase().split_whitespace().collect();
        assert_eq!(
            words.len(),
            24,
            "24-word phrase encodes 256 bits of entropy"
        );
    }

    #[test]
    fn derive_file_key_is_deterministic_in_mnemonic_and_salt() {
        let m = generate_mnemonic();
        let salt = [9u8; K_RECOVERY_SALT_LEN];
        let a = derive_file_key_from_mnemonic(&m, &salt, "");
        let b = derive_file_key_from_mnemonic(&m, &salt, "");
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn different_salt_yields_different_key() {
        let m = generate_mnemonic();
        let salt_a = [1u8; K_RECOVERY_SALT_LEN];
        let salt_b = [2u8; K_RECOVERY_SALT_LEN];
        let ka = derive_file_key_from_mnemonic(&m, &salt_a, "");
        let kb = derive_file_key_from_mnemonic(&m, &salt_b, "");
        assert_ne!(ka.as_bytes(), kb.as_bytes());
    }

    #[test]
    fn different_mnemonic_yields_different_key() {
        let m_a = generate_mnemonic();
        let m_b = generate_mnemonic();
        let salt = [3u8; K_RECOVERY_SALT_LEN];
        let ka = derive_file_key_from_mnemonic(&m_a, &salt, "");
        let kb = derive_file_key_from_mnemonic(&m_b, &salt, "");
        assert_ne!(ka.as_bytes(), kb.as_bytes());
    }

    #[test]
    fn parse_mnemonic_round_trip() {
        let m = generate_mnemonic();
        let phrase = m.phrase().to_string();
        let parsed = parse_mnemonic(&phrase).unwrap();
        assert_eq!(parsed.phrase(), m.phrase());
    }

    #[test]
    fn parse_mnemonic_rejects_garbage() {
        let result = parse_mnemonic("not a real mnemonic phrase here");
        assert!(matches!(result, Err(Error::InvalidArgument(_))));
    }

    #[test]
    fn parse_mnemonic_trims_whitespace() {
        let m = generate_mnemonic();
        let padded = format!("   {}\n", m.phrase());
        let parsed = parse_mnemonic(&padded).unwrap();
        assert_eq!(parsed.phrase(), m.phrase());
    }

    #[test]
    fn generate_recovery_salt_is_random() {
        let a = generate_recovery_salt();
        let b = generate_recovery_salt();
        assert_ne!(a, b);
        assert_eq!(a.len(), K_RECOVERY_SALT_LEN);
    }

    #[test]
    fn manual_bip39_seed_matches_upstream() {
        // Codex review #5: assert the manual PBKDF2 path produces byte-identical
        // output to `bip39::Seed::new`. If they ever diverge, every BIP39-derived
        // K_file changes — caught here loudly rather than at a cross-version
        // recovery flow.
        let m = generate_mnemonic();
        let upstream = bip39::Seed::new(&m, "");
        let manual = derive_bip39_seed(&m, "");
        assert_eq!(
            manual.as_ref() as &[u8],
            upstream.as_bytes(),
            "manual BIP39 seed must match upstream byte-for-byte"
        );

        // Same with a non-empty passphrase.
        let upstream_pp = bip39::Seed::new(&m, "second factor");
        let manual_pp = derive_bip39_seed(&m, "second factor");
        assert_eq!(
            manual_pp.as_ref() as &[u8],
            upstream_pp.as_bytes(),
            "manual BIP39 seed with passphrase must match upstream"
        );
    }
}
