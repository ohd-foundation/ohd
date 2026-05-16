//! The OHD account store — email/password credentials over the shared
//! OHD SaaS SQLite database (`config.store.saas_db`).
//!
//! The IdP is a *consumer* of the SaaS account store, never a second
//! source of truth (see `SPEC.md` — "Account store: shared, not
//! duplicated"). An OHD account is a `profiles` row keyed by a stable
//! `profile_ulid`; this module adds the `email_credentials` table that
//! binds an email + argon2id password hash to that profile.
//!
//! Passwords are held only as argon2id PHC strings — never plaintext,
//! never reversible. The recovery code minted at sign-up is the
//! account-recovery path; its sha-256 hash lands in `profiles`, exactly
//! the shape the rest of OHD already uses.

use anyhow::{anyhow, Context, Result};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rand::Rng;
use rusqlite::params;
use sha2::{Digest, Sha256};

/// `CREATE TABLE IF NOT EXISTS` for the email/password table. Run on
/// startup so the IdP works whether or not the SaaS migration runner has
/// been here first — idempotent by construction. Kept byte-identical to
/// `saas/migrations/002_email_credentials.sql`.
const EMAIL_CREDENTIALS_DDL: &str = "\
CREATE TABLE IF NOT EXISTS email_credentials (
    email          TEXT PRIMARY KEY,
    profile_ulid   TEXT NOT NULL,
    password_hash  TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    FOREIGN KEY (profile_ulid) REFERENCES profiles(profile_ulid) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS email_credentials_profile_idx ON email_credentials(profile_ulid);
";

/// A `profiles` row's minimal shape — enough for the IdP to mint a profile
/// at sign-up. (`profiles` itself is owned by OHD SaaS.)
const PROFILES_DDL: &str = "\
CREATE TABLE IF NOT EXISTS profiles (
    profile_ulid       TEXT PRIMARY KEY,
    recovery_hash_hex  TEXT NOT NULL,
    plan               TEXT NOT NULL DEFAULT 'free',
    created_at         TEXT NOT NULL,
    last_seen_at       TEXT NOT NULL
);
";

/// An authenticated account, as the login flow consumes it.
#[derive(Debug, Clone)]
pub struct Account {
    /// The stable OHD identity — the `sub` of every `id_token`.
    pub profile_ulid: String,
    /// Normalized (lowercase) email.
    pub email: String,
    /// The stored argon2id PHC string.
    password_hash: String,
}

impl Account {
    /// Verify a presented plaintext password against the stored argon2id
    /// hash. Returns `false` on any mismatch or malformed hash — never an
    /// error a caller might leak.
    pub fn verify_password(&self, password: &str) -> bool {
        let parsed = match PasswordHash::new(&self.password_hash) {
            Ok(p) => p,
            Err(_) => return false,
        };
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    }
}

/// A handle on the shared SaaS account store.
#[derive(Clone)]
pub struct AccountStore {
    pool: Pool<SqliteConnectionManager>,
}

impl AccountStore {
    /// Open the shared SaaS SQLite DB at `path` and ensure the
    /// `email_credentials` table exists (idempotent `CREATE TABLE IF NOT
    /// EXISTS`). `profiles` is created too, only as a fallback for a DB the
    /// SaaS has not yet migrated — a no-op once SaaS has run.
    pub fn open(path: &str) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path);
        let pool = Pool::new(manager).context("opening SaaS account database")?;
        let store = Self { pool };
        store.ensure_schema()?;
        Ok(store)
    }

    /// An in-memory store with a fresh schema — for tests.
    pub fn in_memory() -> Result<Self> {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .context("building in-memory pool")?;
        let store = Self { pool };
        store.ensure_schema()?;
        Ok(store)
    }

    fn ensure_schema(&self) -> Result<()> {
        let conn = self.pool.get().context("checking out DB connection")?;
        conn.execute_batch(PROFILES_DDL)
            .context("ensuring profiles table")?;
        conn.execute_batch(EMAIL_CREDENTIALS_DDL)
            .context("ensuring email_credentials table")?;
        Ok(())
    }

    /// Look up an account by email. The email is normalized first, so a
    /// caller may pass whatever the user typed.
    pub fn find_by_email(&self, email: &str) -> Result<Option<Account>> {
        let normalized = normalize_email(email);
        let conn = self.pool.get().context("checking out DB connection")?;
        let row = conn
            .query_row(
                "SELECT email, profile_ulid, password_hash
                   FROM email_credentials WHERE email = ?1",
                params![normalized],
                |r| {
                    Ok(Account {
                        email: r.get(0)?,
                        profile_ulid: r.get(1)?,
                        password_hash: r.get(2)?,
                    })
                },
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("querying email_credentials")?;
        Ok(row)
    }

    /// Create a new email/password account.
    ///
    /// Normalizes + validates the email, mints a fresh `profile_ulid`
    /// (Crockford-base32 ULID — the shape the rest of OHD uses), generates
    /// a recovery code, and writes both the `profiles` row (with
    /// `recovery_hash_hex` = sha-256 of the code) and the
    /// `email_credentials` row in one transaction. Returns
    /// `(profile_ulid, recovery_code)` — the recovery code is shown to the
    /// user exactly once.
    ///
    /// A duplicate email is rejected.
    pub fn create_account(&self, email: &str, password: &str) -> Result<NewAccount> {
        let normalized = normalize_email(email);
        validate_email(&normalized)?;
        validate_password(password)?;

        let profile_ulid = mint_profile_ulid();
        let recovery_code = mint_recovery_code();
        let recovery_hash = sha256_hex(&canonical_recovery(&recovery_code));
        let password_hash = hash_password(password)?;
        let now = now_iso();

        let mut conn = self.pool.get().context("checking out DB connection")?;
        let tx = conn.transaction().context("opening transaction")?;

        // Reject a duplicate email before touching `profiles`.
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM email_credentials WHERE email = ?1",
                params![normalized],
                |_| Ok(()),
            )
            .map(|_| true)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(false),
                other => Err(other),
            })
            .context("checking for duplicate email")?;
        if exists {
            return Err(anyhow!("an account with that email already exists"));
        }

        tx.execute(
            "INSERT INTO profiles (profile_ulid, recovery_hash_hex, plan, created_at, last_seen_at)
             VALUES (?1, ?2, 'free', ?3, ?3)",
            params![profile_ulid, recovery_hash, now],
        )
        .context("inserting profiles row")?;
        tx.execute(
            "INSERT INTO email_credentials (email, profile_ulid, password_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![normalized, profile_ulid, password_hash, now],
        )
        .context("inserting email_credentials row")?;

        tx.commit().context("committing new account")?;

        Ok(NewAccount {
            profile_ulid,
            email: normalized,
            recovery_code,
        })
    }
}

/// The result of [`AccountStore::create_account`].
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub profile_ulid: String,
    pub email: String,
    /// The plaintext recovery code — shown to the user once, never stored.
    pub recovery_code: String,
}

/// A profile resolved from a recovery code. The profile may or may not
/// already have an email credential — recovery-code login resolves the
/// identity directly off `profiles`, independent of `email_credentials`.
#[derive(Debug, Clone)]
pub struct RecoveryAccount {
    /// The stable OHD identity.
    pub profile_ulid: String,
    /// The email of this profile's existing credential, if it has one.
    pub email: Option<String>,
}

impl AccountStore {
    /// Resolve a recovery-code hash to a profile. `recovery_hash` must be
    /// the lowercase-hex sha-256 of the *canonical* recovery code (see
    /// [`canonical_recovery`] + [`sha256_hex`]) — the exact encoding
    /// `profiles.recovery_hash_hex` stores. Returns the profile plus the
    /// email of its existing credential, if any.
    pub fn find_by_recovery_hash(&self, recovery_hash: &str) -> Result<Option<RecoveryAccount>> {
        let conn = self.pool.get().context("checking out DB connection")?;
        let profile_ulid: Option<String> = conn
            .query_row(
                "SELECT profile_ulid FROM profiles WHERE recovery_hash_hex = ?1",
                params![recovery_hash],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("querying profiles by recovery hash")?;

        let profile_ulid = match profile_ulid {
            Some(p) => p,
            None => return Ok(None),
        };

        // The profile may already have an email credential — resolve it so
        // a recovery-code login can carry the email into the `id_token`.
        let email: Option<String> = conn
            .query_row(
                "SELECT email FROM email_credentials WHERE profile_ulid = ?1",
                params![profile_ulid],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("querying email_credentials for profile")?;

        Ok(Some(RecoveryAccount { profile_ulid, email }))
    }

    /// Set (or replace) the password for a profile authenticated by a
    /// recovery code.
    ///
    /// If the profile already has an email credential its `password_hash`
    /// is updated in place. If it has none, `new_email` must be supplied —
    /// a fresh `email_credentials` row is created binding that email to the
    /// profile. The email is normalized + validated and must not already
    /// belong to a *different* profile. Returns the credential's email.
    pub fn reset_password(
        &self,
        profile_ulid: &str,
        new_password: &str,
        new_email: Option<&str>,
    ) -> Result<String> {
        validate_password(new_password)?;
        let password_hash = hash_password(new_password)?;
        let now = now_iso();

        let mut conn = self.pool.get().context("checking out DB connection")?;
        let tx = conn.transaction().context("opening transaction")?;

        // The profile must exist (the recovery code already resolved it,
        // but re-check inside the transaction).
        let profile_exists: bool = tx
            .query_row(
                "SELECT 1 FROM profiles WHERE profile_ulid = ?1",
                params![profile_ulid],
                |_| Ok(()),
            )
            .map(|_| true)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(false),
                other => Err(other),
            })
            .context("checking profile exists")?;
        if !profile_exists {
            return Err(anyhow!("no such profile"));
        }

        // Does this profile already have an email credential?
        let existing_email: Option<String> = tx
            .query_row(
                "SELECT email FROM email_credentials WHERE profile_ulid = ?1",
                params![profile_ulid],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("querying existing credential")?;

        let email = match existing_email {
            Some(email) => {
                // Update the password in place.
                tx.execute(
                    "UPDATE email_credentials
                       SET password_hash = ?1, updated_at = ?2
                     WHERE profile_ulid = ?3",
                    params![password_hash, now, profile_ulid],
                )
                .context("updating password hash")?;
                email
            }
            None => {
                // No credential yet — the user must supply an email to
                // create one.
                let raw = new_email.ok_or_else(|| {
                    anyhow!(
                        "this account has no email yet — enter one to set a password"
                    )
                })?;
                let normalized = normalize_email(raw);
                validate_email(&normalized)?;

                // The email must not already belong to another profile.
                let taken: Option<String> = tx
                    .query_row(
                        "SELECT profile_ulid FROM email_credentials WHERE email = ?1",
                        params![normalized],
                        |r| r.get(0),
                    )
                    .map(Some)
                    .or_else(|e| match e {
                        rusqlite::Error::QueryReturnedNoRows => Ok(None),
                        other => Err(other),
                    })
                    .context("checking email ownership")?;
                if let Some(owner) = taken {
                    if owner != profile_ulid {
                        return Err(anyhow!(
                            "that email is already in use by another account"
                        ));
                    }
                }

                tx.execute(
                    "INSERT INTO email_credentials
                       (email, profile_ulid, password_hash, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![normalized, profile_ulid, password_hash, now],
                )
                .context("inserting email_credentials row")?;
                normalized
            }
        };

        tx.commit().context("committing password reset")?;
        Ok(email)
    }
}

/// Normalize an email: trim + lowercase. The store key is the normalized
/// form, so lookups and inserts agree regardless of how the user typed it.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// A light email validity check — one `@`, non-empty local + domain,
/// a dot in the domain, no spaces. Not RFC 5322, deliberately: it rejects
/// obvious garbage without claiming to be a deliverability oracle.
pub fn validate_email(email: &str) -> Result<()> {
    if email.is_empty() || email.len() > 254 {
        return Err(anyhow!("email is empty or too long"));
    }
    if email.chars().any(|c| c.is_whitespace()) {
        return Err(anyhow!("email must not contain whitespace"));
    }
    let (local, domain) = email
        .split_once('@')
        .ok_or_else(|| anyhow!("email must contain exactly one @"))?;
    if domain.contains('@') {
        return Err(anyhow!("email must contain exactly one @"));
    }
    if local.is_empty() || domain.is_empty() {
        return Err(anyhow!("email is missing a local part or domain"));
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return Err(anyhow!("email domain is not valid"));
    }
    Ok(())
}

/// Minimum password length. Short enough not to nag, long enough to keep
/// the obvious junk out — argon2id carries the real weight.
const MIN_PASSWORD_LEN: usize = 8;

fn validate_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(anyhow!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        ));
    }
    if password.len() > 1024 {
        return Err(anyhow!("password is too long"));
    }
    Ok(())
}

/// Hash a password with argon2id, returning the PHC string to store.
fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("hashing password: {e}"))?;
    Ok(hash.to_string())
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Mint a fresh `profile_ulid`: a 26-char Crockford-base32 ULID, the same
/// display shape OHD storage's `ulid::mint` + `to_crockford` produce — a
/// 48-bit millisecond time prefix plus an 80-bit random tail.
pub fn mint_profile_ulid() -> String {
    let ts_ms: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
        & 0x0000_FFFF_FFFF_FFFF;
    let mut bytes = [0u8; 16];
    bytes[0] = ((ts_ms >> 40) & 0xff) as u8;
    bytes[1] = ((ts_ms >> 32) & 0xff) as u8;
    bytes[2] = ((ts_ms >> 24) & 0xff) as u8;
    bytes[3] = ((ts_ms >> 16) & 0xff) as u8;
    bytes[4] = ((ts_ms >> 8) & 0xff) as u8;
    bytes[5] = (ts_ms & 0xff) as u8;
    rand::thread_rng().fill(&mut bytes[6..16]);

    let mut acc: u128 = 0;
    for &b in &bytes {
        acc = (acc << 8) | b as u128;
    }
    let mut out = [0u8; 26];
    for (i, slot) in out.iter_mut().enumerate() {
        let shift = 5 * (25 - i);
        let idx = ((acc >> shift) & 0x1f) as usize;
        *slot = CROCKFORD[idx];
    }
    String::from_utf8(out.to_vec()).expect("crockford alphabet is ascii")
}

/// Generate a recovery code: 16 lines × 8 Crockford-base32 chars, the same
/// 16×8 grid OHD Connect mints (640 bits of entropy). Rendered with a
/// space between lines so it pastes back cleanly.
pub fn mint_recovery_code() -> String {
    let mut rng = rand::thread_rng();
    let mut lines = Vec::with_capacity(16);
    for _ in 0..16 {
        let line: String = (0..8)
            .map(|_| CROCKFORD[rng.gen_range(0..32)] as char)
            .collect();
        lines.push(line);
    }
    lines.join(" ")
}

/// Canonicalize a recovery code before hashing — strip whitespace,
/// hyphens, underscores; uppercase. Byte-for-byte the same canonical form
/// `ohd-saas`'s `hash_recovery` uses, so a code minted here verifies there.
pub fn canonical_recovery(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Lowercase hex sha-256 — the `recovery_hash_hex` encoding `profiles` uses.
pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

/// Current time as an RFC 3339 / ISO-8601 UTC string.
fn now_iso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_normalization_trims_and_lowercases() {
        assert_eq!(normalize_email("  User@Example.COM "), "user@example.com");
    }

    #[test]
    fn email_validation_accepts_and_rejects() {
        assert!(validate_email("a@b.com").is_ok());
        assert!(validate_email("no-at-sign").is_err());
        assert!(validate_email("two@@b.com").is_err());
        assert!(validate_email("a@nodot").is_err());
        assert!(validate_email("a b@c.com").is_err());
        assert!(validate_email("@b.com").is_err());
    }

    #[test]
    fn profile_ulid_is_26_crockford_chars() {
        let u = mint_profile_ulid();
        assert_eq!(u.len(), 26);
        assert!(u.bytes().all(|b| CROCKFORD.contains(&b)));
        // Two mints differ in the random tail.
        assert_ne!(mint_profile_ulid(), mint_profile_ulid());
    }

    #[test]
    fn recovery_code_is_16x8_grid() {
        let code = mint_recovery_code();
        let lines: Vec<&str> = code.split(' ').collect();
        assert_eq!(lines.len(), 16);
        assert!(lines.iter().all(|l| l.len() == 8));
        // Canonicalization strips the spaces — 128 chars remain.
        assert_eq!(canonical_recovery(&code).len(), 128);
    }

    #[test]
    fn create_account_then_find_and_verify() {
        let store = AccountStore::in_memory().unwrap();
        let created = store
            .create_account("Alice@Example.com", "correct horse battery")
            .unwrap();
        assert_eq!(created.email, "alice@example.com");
        assert_eq!(created.profile_ulid.len(), 26);
        assert!(!created.recovery_code.is_empty());

        let found = store
            .find_by_email("  ALICE@example.com ")
            .unwrap()
            .expect("account found by normalized email");
        assert_eq!(found.profile_ulid, created.profile_ulid);
        assert!(found.verify_password("correct horse battery"));
        assert!(!found.verify_password("wrong password"));
    }

    #[test]
    fn duplicate_email_is_rejected() {
        let store = AccountStore::in_memory().unwrap();
        store.create_account("bob@example.com", "password123").unwrap();
        // Same email, different casing — still a duplicate after normalize.
        let err = store
            .create_account("BOB@example.com", "another-password")
            .unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn short_password_is_rejected() {
        let store = AccountStore::in_memory().unwrap();
        assert!(store.create_account("c@example.com", "short").is_err());
    }

    #[test]
    fn find_unknown_email_is_none() {
        let store = AccountStore::in_memory().unwrap();
        assert!(store.find_by_email("nobody@example.com").unwrap().is_none());
    }

    #[test]
    fn argon2id_hash_round_trips() {
        let hash = hash_password("a-decent-password").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        let acct = Account {
            profile_ulid: "x".into(),
            email: "x@y.z".into(),
            password_hash: hash,
        };
        assert!(acct.verify_password("a-decent-password"));
        assert!(!acct.verify_password("a-decent-passwerd"));
    }

    #[test]
    fn find_by_recovery_hash_resolves_the_profile() {
        let store = AccountStore::in_memory().unwrap();
        let created = store
            .create_account("rec@example.com", "a-good-password")
            .unwrap();
        // The hash a recovery code lands in `profiles` as.
        let hash = sha256_hex(&canonical_recovery(&created.recovery_code));

        let found = store
            .find_by_recovery_hash(&hash)
            .unwrap()
            .expect("profile resolved from recovery hash");
        assert_eq!(found.profile_ulid, created.profile_ulid);
        assert_eq!(found.email.as_deref(), Some("rec@example.com"));

        // An unknown hash resolves to nothing.
        assert!(store
            .find_by_recovery_hash(&sha256_hex("NOTAREALCODE"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn reset_password_updates_an_existing_credential() {
        let store = AccountStore::in_memory().unwrap();
        let created = store
            .create_account("reset@example.com", "old-password")
            .unwrap();

        let email = store
            .reset_password(&created.profile_ulid, "brand-new-password", None)
            .unwrap();
        assert_eq!(email, "reset@example.com");

        let acct = store
            .find_by_email("reset@example.com")
            .unwrap()
            .expect("account still found");
        assert!(acct.verify_password("brand-new-password"));
        assert!(!acct.verify_password("old-password"));
    }

    #[test]
    fn reset_password_creates_a_credential_for_an_emailless_profile() {
        let store = AccountStore::in_memory().unwrap();
        // A profile with a recovery hash but no email credential — the
        // shape a profile minted outside the IdP can have.
        let recovery = "AAAA BBBB CCCC DDDD";
        let hash = sha256_hex(&canonical_recovery(recovery));
        {
            let conn = store.pool.get().unwrap();
            conn.execute(
                "INSERT INTO profiles (profile_ulid, recovery_hash_hex, plan, created_at, last_seen_at)
                 VALUES ('01EMAILLESS', ?1, 'free', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                params![hash],
            )
            .unwrap();
        }

        // Without an email it cannot create a credential.
        assert!(store
            .reset_password("01EMAILLESS", "a-new-password", None)
            .is_err());

        // With an email it creates one.
        let email = store
            .reset_password("01EMAILLESS", "a-new-password", Some("fresh@example.com"))
            .unwrap();
        assert_eq!(email, "fresh@example.com");
        let acct = store
            .find_by_email("fresh@example.com")
            .unwrap()
            .expect("credential created");
        assert_eq!(acct.profile_ulid, "01EMAILLESS");
        assert!(acct.verify_password("a-new-password"));
    }

    #[test]
    fn reset_password_rejects_an_email_owned_by_another_profile() {
        let store = AccountStore::in_memory().unwrap();
        store
            .create_account("owner@example.com", "owner-password")
            .unwrap();
        // A second, email-less profile.
        let hash = sha256_hex(&canonical_recovery("ZZZZ YYYY"));
        {
            let conn = store.pool.get().unwrap();
            conn.execute(
                "INSERT INTO profiles (profile_ulid, recovery_hash_hex, plan, created_at, last_seen_at)
                 VALUES ('01OTHER', ?1, 'free', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                params![hash],
            )
            .unwrap();
        }
        let err = store
            .reset_password("01OTHER", "a-new-password", Some("owner@example.com"))
            .unwrap_err();
        assert!(err.to_string().contains("already in use"));
    }

    #[test]
    fn recovery_hash_matches_saas_canonical_form() {
        // The hash a recovery code lands in `profiles` as must be the
        // sha-256 of the SaaS canonical form (uppercase, no separators).
        let code = "abcd efgh\nijkl-mnop";
        assert_eq!(canonical_recovery(code), "ABCDEFGHIJKLMNOP");
        let expected =
            sha256_hex("ABCDEFGHIJKLMNOP");
        assert_eq!(sha256_hex(&canonical_recovery(code)), expected);
    }
}
