//! The IdP's own state store — authorization codes and access tokens.
//!
//! This is the IdP-local SQLite database in `config.server.data_dir`
//! (distinct from the shared SaaS account store). It holds the short-lived
//! OIDC machinery state, never accounts:
//!
//! - **Authorization codes** — single-use, expire after `code_ttl_secs`,
//!   bound to `(client_id, profile_ulid, redirect_uri, nonce,
//!   code_challenge)`. Minted after a successful login, redeemed once at
//!   `POST /token`.
//! - **Access tokens** — opaque bearer strings, TTL'd, redeemed at
//!   `GET /userinfo`.
//!
//! Codes are stored hashed (sha-256), so a leaked database snapshot does
//! not yield a usable code. The PKCE `code_challenge` is the verifier's
//! S256 transform — the IdP checks the presented `code_verifier` against
//! it at the token endpoint.

use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rand::Rng;
use rusqlite::params;
use sha2::{Digest, Sha256};

/// Schema for the IdP-local store. Idempotent.
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS auth_codes (
    code_hash      TEXT PRIMARY KEY,
    client_id      TEXT NOT NULL,
    profile_ulid   TEXT NOT NULL,
    redirect_uri   TEXT NOT NULL,
    nonce          TEXT,
    code_challenge TEXT NOT NULL,
    scope          TEXT NOT NULL,
    email          TEXT NOT NULL,
    auth_time      INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL,
    used           INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS access_tokens (
    token_hash    TEXT PRIMARY KEY,
    profile_ulid  TEXT NOT NULL,
    email         TEXT NOT NULL,
    scope         TEXT NOT NULL,
    expires_at    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS continuations (
    token_hash     TEXT PRIMARY KEY,
    client_id      TEXT NOT NULL,
    profile_ulid   TEXT NOT NULL,
    email          TEXT NOT NULL,
    redirect_uri   TEXT NOT NULL,
    nonce          TEXT,
    code_challenge TEXT NOT NULL,
    scope          TEXT NOT NULL,
    state          TEXT NOT NULL,
    auth_time      INTEGER NOT NULL,
    expires_at     INTEGER NOT NULL
);
";

/// A pending authorization — what `/authorize` records for a login still
/// in flight, carried through the login form, then [`IdpStore::issue_code`]'d.
#[derive(Debug, Clone)]
pub struct PendingAuth {
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub state: String,
    pub nonce: Option<String>,
    pub code_challenge: String,
}

/// A redeemed authorization code — the binding `/token` verifies against.
#[derive(Debug, Clone)]
pub struct RedeemedCode {
    pub client_id: String,
    pub profile_ulid: String,
    pub redirect_uri: String,
    pub nonce: Option<String>,
    pub code_challenge: String,
    pub scope: String,
    pub email: String,
    pub auth_time: i64,
}

/// A validated access token's identity, for `/userinfo`.
#[derive(Debug, Clone)]
pub struct TokenIdentity {
    pub profile_ulid: String,
    pub email: String,
    pub scope: String,
}

/// Why a code redemption failed. Each maps to an OAuth `invalid_grant`,
/// but the variants keep the reason precise for tracing + tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeError {
    /// No such code — never minted, or already pruned.
    Unknown,
    /// The code's `expires_at` has passed.
    Expired,
    /// The code was already redeemed once.
    AlreadyUsed,
}

impl std::fmt::Display for CodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            CodeError::Unknown => "authorization code is unknown",
            CodeError::Expired => "authorization code has expired",
            CodeError::AlreadyUsed => "authorization code has already been used",
        };
        f.write_str(s)
    }
}

impl std::error::Error for CodeError {}

/// The IdP-local SQLite store.
#[derive(Clone)]
pub struct IdpStore {
    pool: Pool<SqliteConnectionManager>,
}

impl IdpStore {
    /// Open (or create) the IdP-local DB at `path`.
    pub fn open(path: &str) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path);
        let pool = Pool::new(manager).context("opening IdP state database")?;
        let store = Self { pool };
        store.init()?;
        Ok(store)
    }

    /// An in-memory store — for tests.
    pub fn in_memory() -> Result<Self> {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .context("building in-memory pool")?;
        let store = Self { pool };
        store.init()?;
        Ok(store)
    }

    fn init(&self) -> Result<()> {
        let conn = self.pool.get().context("checking out DB connection")?;
        conn.execute_batch(SCHEMA).context("initialising IdP schema")?;
        Ok(())
    }

    /// Mint a single-use authorization code for an authenticated user.
    /// Returns the opaque code string handed to the browser; only its
    /// hash is stored. `ttl_secs` is `config.session.code_ttl_secs`.
    #[allow(clippy::too_many_arguments)]
    pub fn issue_code(
        &self,
        client_id: &str,
        profile_ulid: &str,
        email: &str,
        redirect_uri: &str,
        nonce: Option<&str>,
        code_challenge: &str,
        scope: &str,
        ttl_secs: i64,
    ) -> Result<String> {
        let code = random_token(48);
        let now = now_unix();
        let conn = self.pool.get().context("checking out DB connection")?;
        conn.execute(
            "INSERT INTO auth_codes
               (code_hash, client_id, profile_ulid, redirect_uri, nonce,
                code_challenge, scope, email, auth_time, expires_at, used)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0)",
            params![
                sha256_b64(&code),
                client_id,
                profile_ulid,
                redirect_uri,
                nonce,
                code_challenge,
                scope,
                email,
                now,
                now + ttl_secs,
            ],
        )
        .context("inserting authorization code")?;
        Ok(code)
    }

    /// Redeem an authorization code: it must exist, be unexpired, and
    /// unused. On success the code is marked used (single-use) and its
    /// binding returned. The caller still verifies `client_id`,
    /// `redirect_uri`, and the PKCE `code_verifier` against the returned
    /// binding.
    pub fn redeem_code(&self, code: &str) -> Result<std::result::Result<RedeemedCode, CodeError>> {
        let hash = sha256_b64(code);
        let now = now_unix();
        let mut conn = self.pool.get().context("checking out DB connection")?;
        let tx = conn.transaction().context("opening transaction")?;

        let row = tx
            .query_row(
                "SELECT client_id, profile_ulid, redirect_uri, nonce,
                        code_challenge, scope, email, auth_time, expires_at, used
                   FROM auth_codes WHERE code_hash = ?1",
                params![hash],
                |r| {
                    Ok((
                        RedeemedCode {
                            client_id: r.get(0)?,
                            profile_ulid: r.get(1)?,
                            redirect_uri: r.get(2)?,
                            nonce: r.get(3)?,
                            code_challenge: r.get(4)?,
                            scope: r.get(5)?,
                            email: r.get(6)?,
                            auth_time: r.get(7)?,
                        },
                        r.get::<_, i64>(8)?, // expires_at
                        r.get::<_, i64>(9)?, // used
                    ))
                },
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("querying authorization code")?;

        let (redeemed, expires_at, used) = match row {
            None => return Ok(Err(CodeError::Unknown)),
            Some(t) => t,
        };
        if used != 0 {
            return Ok(Err(CodeError::AlreadyUsed));
        }
        if expires_at <= now {
            return Ok(Err(CodeError::Expired));
        }
        tx.execute(
            "UPDATE auth_codes SET used = 1 WHERE code_hash = ?1",
            params![hash],
        )
        .context("marking authorization code used")?;
        tx.commit().context("committing code redemption")?;
        Ok(Ok(redeemed))
    }

    /// Mint and store an opaque access token. Returns the token string;
    /// only its hash is persisted.
    pub fn issue_access_token(
        &self,
        profile_ulid: &str,
        email: &str,
        scope: &str,
        ttl_secs: i64,
    ) -> Result<String> {
        let token = random_token(48);
        let conn = self.pool.get().context("checking out DB connection")?;
        conn.execute(
            "INSERT INTO access_tokens (token_hash, profile_ulid, email, scope, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                sha256_b64(&token),
                profile_ulid,
                email,
                scope,
                now_unix() + ttl_secs,
            ],
        )
        .context("inserting access token")?;
        Ok(token)
    }

    /// Resolve an access token to its identity, if it exists and is
    /// unexpired.
    pub fn lookup_access_token(&self, token: &str) -> Result<Option<TokenIdentity>> {
        let conn = self.pool.get().context("checking out DB connection")?;
        let row = conn
            .query_row(
                "SELECT profile_ulid, email, scope, expires_at
                   FROM access_tokens WHERE token_hash = ?1",
                params![sha256_b64(token)],
                |r| {
                    Ok((
                        TokenIdentity {
                            profile_ulid: r.get(0)?,
                            email: r.get(1)?,
                            scope: r.get(2)?,
                        },
                        r.get::<_, i64>(3)?,
                    ))
                },
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("querying access token")?;
        Ok(match row {
            Some((id, expires_at)) if expires_at > now_unix() => Some(id),
            _ => None,
        })
    }
}

/// An authenticated login awaiting the user's "I saved my recovery code"
/// confirmation — what [`IdpStore::stash_continuation`] records and
/// [`IdpStore::take_continuation`] resumes.
#[derive(Debug, Clone)]
pub struct Continuation {
    pub client_id: String,
    pub profile_ulid: String,
    pub email: String,
    pub redirect_uri: String,
    pub nonce: Option<String>,
    pub code_challenge: String,
    pub scope: String,
    pub state: String,
    pub auth_time: i64,
}

impl IdpStore {
    /// Stash an authenticated-but-not-yet-completed login so the sign-up
    /// recovery-code page can resume the authorize flow without re-posting
    /// the password. Returns an opaque continuation token; only its hash
    /// is stored. Short-lived (`ttl_secs`).
    #[allow(clippy::too_many_arguments)]
    pub fn stash_continuation(&self, c: &Continuation, ttl_secs: i64) -> Result<String> {
        let token = random_token(40);
        let conn = self.pool.get().context("checking out DB connection")?;
        conn.execute(
            "INSERT INTO continuations
               (token_hash, client_id, profile_ulid, email, redirect_uri,
                nonce, code_challenge, scope, state, auth_time, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                sha256_b64(&token),
                c.client_id,
                c.profile_ulid,
                c.email,
                c.redirect_uri,
                c.nonce,
                c.code_challenge,
                c.scope,
                c.state,
                c.auth_time,
                now_unix() + ttl_secs,
            ],
        )
        .context("inserting continuation")?;
        Ok(token)
    }

    /// Consume a continuation token (single-use). Returns `None` if it is
    /// unknown or expired.
    pub fn take_continuation(&self, token: &str) -> Result<Option<Continuation>> {
        let hash = sha256_b64(token);
        let mut conn = self.pool.get().context("checking out DB connection")?;
        let tx = conn.transaction().context("opening transaction")?;
        let row = tx
            .query_row(
                "SELECT client_id, profile_ulid, email, redirect_uri, nonce,
                        code_challenge, scope, state, auth_time, expires_at
                   FROM continuations WHERE token_hash = ?1",
                params![hash],
                |r| {
                    Ok((
                        Continuation {
                            client_id: r.get(0)?,
                            profile_ulid: r.get(1)?,
                            email: r.get(2)?,
                            redirect_uri: r.get(3)?,
                            nonce: r.get(4)?,
                            code_challenge: r.get(5)?,
                            scope: r.get(6)?,
                            state: r.get(7)?,
                            auth_time: r.get(8)?,
                        },
                        r.get::<_, i64>(9)?,
                    ))
                },
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("querying continuation")?;
        let cont = match row {
            None => return Ok(None),
            Some((c, expires_at)) => {
                tx.execute(
                    "DELETE FROM continuations WHERE token_hash = ?1",
                    params![hash],
                )
                .context("deleting continuation")?;
                if expires_at <= now_unix() {
                    tx.commit().ok();
                    return Ok(None);
                }
                c
            }
        };
        tx.commit().context("committing continuation take")?;
        Ok(Some(cont))
    }
}

/// Verify a PKCE `code_verifier` against a stored S256 `code_challenge`.
/// `code_challenge == base64url(sha256(code_verifier))`.
pub fn verify_pkce_s256(code_verifier: &str, code_challenge: &str) -> bool {
    let digest = Sha256::digest(code_verifier.as_bytes());
    let computed = URL_SAFE_NO_PAD.encode(digest);
    // Constant-time-ish: the inputs are short and non-secret post-redeem,
    // but a length-then-equality compare is plenty here.
    computed == code_challenge
}

/// A URL-safe random token from the unreserved alphabet.
pub fn random_token(len: usize) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| ALPHA[rng.gen_range(0..ALPHA.len())] as char)
        .collect()
}

/// base64url(sha256(input)) — how codes + tokens are stored at rest.
fn sha256_b64(input: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(input.as_bytes()))
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(store: &IdpStore, ttl: i64) -> String {
        store
            .issue_code(
                "cord-web",
                "01ABC",
                "u@e.com",
                "https://cord.ohd.dev/cb",
                Some("nonce-xyz"),
                "challenge-abc",
                "openid email",
                ttl,
            )
            .unwrap()
    }

    #[test]
    fn code_redeems_once_then_is_used() {
        let store = IdpStore::in_memory().unwrap();
        let code = issue(&store, 120);

        let first = store.redeem_code(&code).unwrap();
        let redeemed = first.expect("first redemption succeeds");
        assert_eq!(redeemed.client_id, "cord-web");
        assert_eq!(redeemed.profile_ulid, "01ABC");
        assert_eq!(redeemed.nonce.as_deref(), Some("nonce-xyz"));

        // Second redemption fails — single-use.
        let second = store.redeem_code(&code).unwrap();
        assert_eq!(second.unwrap_err(), CodeError::AlreadyUsed);
    }

    #[test]
    fn expired_code_is_rejected() {
        let store = IdpStore::in_memory().unwrap();
        // Negative TTL → already expired.
        let code = issue(&store, -1);
        let result = store.redeem_code(&code).unwrap();
        assert_eq!(result.unwrap_err(), CodeError::Expired);
    }

    #[test]
    fn unknown_code_is_rejected() {
        let store = IdpStore::in_memory().unwrap();
        let result = store.redeem_code("never-minted").unwrap();
        assert_eq!(result.unwrap_err(), CodeError::Unknown);
    }

    #[test]
    fn pkce_s256_verifies_a_matching_verifier() {
        // RFC 7636 appendix B test vector.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(verify_pkce_s256(verifier, challenge));
        assert!(!verify_pkce_s256("wrong-verifier", challenge));
    }

    #[test]
    fn access_token_round_trips() {
        let store = IdpStore::in_memory().unwrap();
        let token = store
            .issue_access_token("01XYZ", "u@e.com", "openid email", 3600)
            .unwrap();
        let id = store.lookup_access_token(&token).unwrap().unwrap();
        assert_eq!(id.profile_ulid, "01XYZ");
        assert_eq!(id.email, "u@e.com");

        assert!(store.lookup_access_token("bogus-token").unwrap().is_none());
    }

    #[test]
    fn expired_access_token_is_not_returned() {
        let store = IdpStore::in_memory().unwrap();
        let token = store
            .issue_access_token("01XYZ", "u@e.com", "openid", -1)
            .unwrap();
        assert!(store.lookup_access_token(&token).unwrap().is_none());
    }
}
