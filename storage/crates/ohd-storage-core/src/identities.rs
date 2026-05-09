//! Multi-identity OIDC account linking.
//!
//! A single `user_ulid` can be associated with multiple OIDC `(provider,
//! subject)` identities. This module owns:
//!
//! - the [`Identity`] row shape and CRUD over `_oidc_identities`,
//! - the link-flow state machine over `_pending_identity_links`
//!   ([`link_identity_start`] + [`complete_identity_link`]),
//! - the identity resolver used during sign-in
//!   ([`find_user_by_identity`]),
//! - the JWT (id_token) verification helpers — signature against a [`JwksResolver`],
//!   issuer / audience / time claims, and `(iss, sub)` extraction.
//!
//! See `spec/auth.md` "Multiple identities per user" and STATUS.md for the
//! design rationale. The schema lives in `migrations/007_multi_identity.sql`.
//!
//! # JWKS resolution
//!
//! Verifying a linked identity's `id_token` requires the OIDC issuer's
//! signing keys (the JWKS). Fetching the JWKS over HTTP is *not* this
//! crate's job — `ohd-storage-core` stays HTTP-client-agnostic so the
//! same code path works in the server (real network), bindings (FFI),
//! and tests (in-memory keys). The [`JwksResolver`] trait is the seam:
//! the server crate wires a hyper-based fetcher with a 1-hour TTL cache,
//! tests inject [`StaticJwksResolver`] with a precomputed JWK set.

use std::collections::HashMap;
use std::sync::Mutex;

use jsonwebtoken::jwk::{Jwk, JwkSet};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::ulid::Ulid;
use crate::{Error, Result};

/// One linked OIDC identity bound to a `user_ulid`.
#[derive(Debug, Clone)]
pub struct Identity {
    /// Storage rowid.
    pub id: i64,
    /// User this identity points at.
    pub user_ulid: Ulid,
    /// OIDC issuer (`iss` claim) or short provider name (`google`,
    /// `facebook`, `apple`, …).
    pub provider: String,
    /// Provider-issued opaque user id (`sub` claim).
    pub subject: String,
    /// User-facing label (e.g. "Personal Google", "Clinic SSO Acme"). Optional.
    pub display_label: Option<String>,
    /// One identity per user is the primary. Used as the canonical identity in
    /// invitation flows etc. The first-linked identity is auto-primary; users
    /// may promote any other via [`set_primary`].
    pub is_primary: bool,
    /// When this identity was linked (UTC ms).
    pub linked_at_ms: i64,
    /// Last successful sign-in via this identity. None = never used since linking.
    pub last_login_ms: Option<i64>,
}

/// Outcome of [`link_identity_start`]: a freshly-minted opaque link token the
/// caller hands the front-end app, plus the row's auto-expiry.
#[derive(Debug, Clone)]
pub struct LinkStartOutcome {
    /// Crockford-base32 encoded 32-byte nonce. Caller embeds this as the
    /// OAuth `state` parameter.
    pub link_token: String,
    /// When the pending row auto-expires (10 minutes after creation by default).
    pub expires_at_ms: i64,
}

/// TTL for `_pending_identity_links` rows.
pub const PENDING_LINK_TTL_MS: i64 = 10 * 60 * 1000;

/// Resolves OIDC `iss` → JWK set. Implementations decide caching strategy.
///
/// The server crate's implementation HTTP-GETs `<iss>/.well-known/jwks.json`
/// (or follows `<iss>/.well-known/openid-configuration`'s `jwks_uri`) with a
/// 1-hour TTL cache. Tests inject [`StaticJwksResolver`].
pub trait JwksResolver: Send + Sync {
    /// Return the JWKS for the given issuer. Must succeed within a few seconds
    /// or return an error; long-blocking fetches are not the storage core's
    /// problem.
    fn resolve(&self, issuer: &str) -> Result<JwkSet>;
}

/// In-memory `JwksResolver` keyed by `iss` string. Useful for tests and for
/// the in-process binding case where the host pre-loads keys.
pub struct StaticJwksResolver {
    by_issuer: Mutex<HashMap<String, JwkSet>>,
}

impl StaticJwksResolver {
    /// Empty resolver. Insert keys with [`insert`].
    pub fn new() -> Self {
        Self {
            by_issuer: Mutex::new(HashMap::new()),
        }
    }

    /// Register a JWK set under an issuer string.
    pub fn insert(&self, issuer: impl Into<String>, jwks: JwkSet) {
        self.by_issuer
            .lock()
            .expect("StaticJwksResolver mutex")
            .insert(issuer.into(), jwks);
    }
}

impl Default for StaticJwksResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl JwksResolver for StaticJwksResolver {
    fn resolve(&self, issuer: &str) -> Result<JwkSet> {
        self.by_issuer
            .lock()
            .expect("StaticJwksResolver mutex")
            .get(issuer)
            .cloned()
            .ok_or_else(|| Error::InvalidArgument(format!("no JWKS for issuer {issuer:?}")))
    }
}

/// Issuer-specific verification config. The caller supplies what audience and
/// algorithms are acceptable — different OIDC providers use different `aud`
/// shapes and a few still ship RS256 only.
#[derive(Debug, Clone)]
pub struct IssuerVerification {
    /// Expected `iss` claim. Must match exactly.
    pub issuer: String,
    /// Acceptable `aud` claim values. At least one must match the token's
    /// `aud`. (Some providers issue space-separated multi-audience tokens —
    /// jsonwebtoken handles both shapes.)
    pub audiences: Vec<String>,
    /// Acceptable signature algorithms. Defaults to `[RS256, ES256]`.
    pub algorithms: Vec<Algorithm>,
}

impl IssuerVerification {
    /// Construct with default `algorithms = [RS256, ES256]`.
    pub fn new(issuer: impl Into<String>, audiences: Vec<String>) -> Self {
        Self {
            issuer: issuer.into(),
            audiences,
            algorithms: vec![Algorithm::RS256, Algorithm::ES256],
        }
    }
}

/// Verified id_token claims this module cares about.
#[derive(Debug, Clone)]
pub struct VerifiedIdToken {
    /// Issuer (`iss`).
    pub issuer: String,
    /// Subject (`sub`).
    pub subject: String,
    /// Email if present (some providers emit it on the id_token, others gate
    /// it behind userinfo).
    pub email: Option<String>,
}

/// Verify an OIDC id_token against a [`JwksResolver`] and an
/// [`IssuerVerification`] config. On success, returns the `(iss, sub, email?)`
/// triple. On failure, returns an [`Error::InvalidArgument`] with a human
/// reason — we deliberately fold every JWT/JWKS error to the same code rather
/// than leak the upstream library's error structure into the OHDC error
/// catalog.
pub fn verify_id_token(
    id_token: &str,
    cfg: &IssuerVerification,
    jwks_resolver: &dyn JwksResolver,
) -> Result<VerifiedIdToken> {
    let header = decode_header(id_token)
        .map_err(|e| Error::InvalidArgument(format!("id_token header: {e}")))?;

    let kid = header
        .kid
        .ok_or_else(|| Error::InvalidArgument("id_token header missing kid".into()))?;

    let jwks = jwks_resolver.resolve(&cfg.issuer)?;
    let jwk: &Jwk = jwks
        .keys
        .iter()
        .find(|k| k.common.key_id.as_deref() == Some(kid.as_str()))
        .ok_or_else(|| {
            Error::InvalidArgument(format!(
                "no JWK with kid={kid:?} in issuer {:?}'s JWKS",
                cfg.issuer
            ))
        })?;

    let key = DecodingKey::from_jwk(jwk)
        .map_err(|e| Error::InvalidArgument(format!("decode JWK: {e}")))?;

    if !cfg.algorithms.is_empty() && !cfg.algorithms.contains(&header.alg) {
        return Err(Error::InvalidArgument(format!(
            "id_token alg {:?} not in accepted set {:?}",
            header.alg, cfg.algorithms
        )));
    }
    // Validate against the header's alg explicitly. jsonwebtoken treats
    // `validation.algorithms` as the *only* set of acceptable signing
    // algorithms; setting it to a multi-element list while the key was
    // decoded for one specific alg has tripped its `InvalidAlgorithm` check
    // in some versions. Pin it to the JWT header's alg (which we've already
    // confirmed is in the configured allow-set above).
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[cfg.issuer.as_str()]);
    if !cfg.audiences.is_empty() {
        let auds: Vec<&str> = cfg.audiences.iter().map(String::as_str).collect();
        validation.set_audience(&auds);
    } else {
        // No audiences configured: skip aud validation. Useful in tests; in
        // production every issuer should pin an audience.
        validation.validate_aud = false;
    }
    validation.validate_exp = true;
    validation.validate_nbf = true;

    let token = decode::<IdTokenClaims>(id_token, &key, &validation)
        .map_err(|e| Error::InvalidArgument(format!("id_token verification failed: {e}")))?;

    Ok(VerifiedIdToken {
        issuer: token.claims.iss,
        subject: token.claims.sub,
        email: token.claims.email,
    })
}

#[derive(Debug, serde::Deserialize)]
struct IdTokenClaims {
    iss: String,
    sub: String,
    #[serde(default)]
    email: Option<String>,
}

// ============================================================================
// Pending-link state machine
// ============================================================================

/// Begin a link flow. Caller is a self-session-authenticated user; we mint a
/// random 32-byte `link_token`, persist a `_pending_identity_links` row with a
/// 10-minute TTL, and hand the token back. The caller embeds it as the
/// OAuth `state` parameter when redirecting to the new provider.
pub fn link_identity_start(
    conn: &Connection,
    user_ulid: Ulid,
    session_token_id: Option<i64>,
    provider_hint: Option<&str>,
) -> Result<LinkStartOutcome> {
    let now = crate::format::now_ms();
    let expires_at_ms = now + PENDING_LINK_TTL_MS;
    let nonce = crate::ulid::random_bytes(32);
    let link_token = base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &nonce);
    conn.execute(
        "INSERT INTO _pending_identity_links
            (link_token, requesting_user_ulid, requesting_session_id,
             provider_hint, created_at_ms, expires_at_ms, completed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
        params![
            nonce,
            user_ulid.to_vec(),
            session_token_id,
            provider_hint,
            now,
            expires_at_ms,
        ],
    )?;
    Ok(LinkStartOutcome {
        link_token,
        expires_at_ms,
    })
}

/// Complete a link flow. Validates the supplied `link_token` is still pending,
/// verifies the `id_token` (signature, issuer, audience, expiry) via
/// [`verify_id_token`], inserts a new `_oidc_identities` row pointing at the
/// requesting user, and marks the pending row completed.
///
/// Idempotency: if the verified `(iss, sub)` is *already* linked to the same
/// user, the existing row is returned and the pending row is still marked
/// completed (no error). If `(iss, sub)` is linked to a *different* user, the
/// call fails with [`Error::IdempotencyConflict`] — the linker would otherwise
/// silently take ownership of someone else's account.
pub fn complete_identity_link(
    conn: &mut Connection,
    link_token: &str,
    id_token: &str,
    cfg: &IssuerVerification,
    jwks_resolver: &dyn JwksResolver,
    display_label: Option<&str>,
) -> Result<Identity> {
    let nonce = base32::decode(base32::Alphabet::Rfc4648 { padding: false }, link_token)
        .ok_or_else(|| Error::InvalidArgument("link_token: not base32".into()))?;

    // Verify the id_token (signature + claim checks) BEFORE we touch the DB.
    // Verification can fail without DB side effects.
    let verified = verify_id_token(id_token, cfg, jwks_resolver)?;

    let now = crate::format::now_ms();

    let tx = conn.transaction()?;

    // Look up the pending row.
    let row: Option<(i64, Vec<u8>, Option<i64>, i64, i64)> = tx
        .query_row(
            "SELECT id, requesting_user_ulid, requesting_session_id,
                    expires_at_ms, completed
               FROM _pending_identity_links
              WHERE link_token = ?1",
            params![nonce],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let (pending_id, user_blob, session_id, expires_at_ms, completed) =
        row.ok_or_else(|| Error::InvalidArgument("link_token: not found or already used".into()))?;
    if completed != 0 {
        return Err(Error::InvalidArgument(
            "link_token: already completed".into(),
        ));
    }
    if expires_at_ms <= now {
        return Err(Error::InvalidArgument("link_token: expired".into()));
    }
    let user_ulid: Ulid = user_blob
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidUlid)?;

    // Idempotency / conflict: is this (iss, sub) already linked?
    let existing: Option<(i64, Vec<u8>)> = tx
        .query_row(
            "SELECT id, user_ulid FROM _oidc_identities
              WHERE provider = ?1 AND subject = ?2",
            params![&verified.issuer, &verified.subject],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;

    let identity_id = if let Some((eid, existing_user_blob)) = existing {
        let existing_user: Ulid = existing_user_blob
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidUlid)?;
        if existing_user != user_ulid {
            return Err(Error::IdempotencyConflict);
        }
        // Already linked to the same user — idempotent re-link. Update the
        // last_login_ms so it's discoverable that the user re-presented.
        tx.execute(
            "UPDATE _oidc_identities SET last_login_ms = ?1 WHERE id = ?2",
            params![now, eid],
        )?;
        eid
    } else {
        // Determine `is_primary`: first-linked identity for the user is auto-primary.
        let any_identity: Option<i64> = tx
            .query_row(
                "SELECT id FROM _oidc_identities WHERE user_ulid = ?1 LIMIT 1",
                params![user_ulid.to_vec()],
                |r| r.get(0),
            )
            .optional()?;
        let is_primary = any_identity.is_none();

        let email_hash = verified
            .email
            .as_deref()
            .map(|e| Sha256::digest(e.as_bytes()).to_vec());

        tx.execute(
            "INSERT INTO _oidc_identities
                (user_ulid, provider, subject, email_hash, display_label,
                 is_primary, linked_at_ms, linked_via_actor_id, last_login_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                user_ulid.to_vec(),
                verified.issuer,
                verified.subject,
                email_hash,
                display_label,
                if is_primary { 1 } else { 0 },
                now,
                session_id,
                now,
            ],
        )?;
        tx.last_insert_rowid()
    };

    tx.execute(
        "UPDATE _pending_identity_links
           SET completed = 1, completed_at_ms = ?1
         WHERE id = ?2",
        params![now, pending_id],
    )?;

    let row: Identity = read_identity(&tx, identity_id)?;

    tx.commit()?;
    Ok(row)
}

fn read_identity(conn: &Connection, id: i64) -> Result<Identity> {
    conn.query_row(
        "SELECT id, user_ulid, provider, subject, display_label,
                is_primary, linked_at_ms, last_login_ms
           FROM _oidc_identities WHERE id = ?1",
        params![id],
        identity_from_row,
    )
    .map_err(Error::from)
}

fn identity_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Identity> {
    let id: i64 = r.get(0)?;
    let user_blob: Vec<u8> = r.get(1)?;
    let mut user_ulid = [0u8; 16];
    if user_blob.len() == 16 {
        user_ulid.copy_from_slice(&user_blob);
    }
    Ok(Identity {
        id,
        user_ulid,
        provider: r.get(2)?,
        subject: r.get(3)?,
        display_label: r.get(4)?,
        is_primary: r.get::<_, i64>(5)? != 0,
        linked_at_ms: r.get(6)?,
        last_login_ms: r.get(7)?,
    })
}

/// List every identity bound to `user_ulid`, ordered with the primary first
/// then by `linked_at_ms` ASC (oldest first — gives a stable display order).
pub fn list_identities(conn: &Connection, user_ulid: Ulid) -> Result<Vec<Identity>> {
    let mut stmt = conn.prepare(
        "SELECT id, user_ulid, provider, subject, display_label,
                is_primary, linked_at_ms, last_login_ms
           FROM _oidc_identities
          WHERE user_ulid = ?1
          ORDER BY is_primary DESC, linked_at_ms ASC",
    )?;
    let mut iter = stmt.query_map(params![user_ulid.to_vec()], identity_from_row)?;
    let mut out = Vec::new();
    while let Some(row) = iter.next() {
        out.push(row?);
    }
    Ok(out)
}

/// Refuses to remove the *last* identity bound to a user (would orphan the
/// account). If the removed identity was primary, promotes the oldest
/// remaining identity to primary so there's always exactly one.
pub fn unlink_identity(
    conn: &mut Connection,
    user_ulid: Ulid,
    provider: &str,
    subject: &str,
) -> Result<()> {
    let tx = conn.transaction()?;
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM _oidc_identities WHERE user_ulid = ?1",
        params![user_ulid.to_vec()],
        |r| r.get(0),
    )?;
    if count <= 1 {
        return Err(Error::OutOfScope);
    }
    let removed: Option<(i64, i64)> = tx
        .query_row(
            "SELECT id, is_primary FROM _oidc_identities
              WHERE user_ulid = ?1 AND provider = ?2 AND subject = ?3",
            params![user_ulid.to_vec(), provider, subject],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (id, was_primary) = removed.ok_or(Error::NotFound)?;
    tx.execute("DELETE FROM _oidc_identities WHERE id = ?1", params![id])?;
    if was_primary != 0 {
        // Promote the oldest remaining identity.
        let next: Option<i64> = tx
            .query_row(
                "SELECT id FROM _oidc_identities
                   WHERE user_ulid = ?1
                   ORDER BY linked_at_ms ASC LIMIT 1",
                params![user_ulid.to_vec()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(next_id) = next {
            tx.execute(
                "UPDATE _oidc_identities SET is_primary = 1 WHERE id = ?1",
                params![next_id],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Sign-in resolver: given a presented OIDC `(iss, sub)`, return the bound
/// user. Returns `None` when no identity is linked (the caller decides whether
/// to mint a fresh user_ulid or refuse per the deployment's account-join mode).
pub fn find_user_by_identity(
    conn: &Connection,
    provider: &str,
    subject: &str,
) -> Result<Option<Ulid>> {
    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT user_ulid FROM _oidc_identities
              WHERE provider = ?1 AND subject = ?2",
            params![provider, subject],
            |r| r.get(0),
        )
        .optional()?;
    match blob {
        None => Ok(None),
        Some(b) if b.len() == 16 => {
            let mut o = [0u8; 16];
            o.copy_from_slice(&b);
            Ok(Some(o))
        }
        Some(_) => Err(Error::InvalidUlid),
    }
}

/// Mark `(provider, subject)` as the user's primary identity. The user's
/// previous primary (if any) is demoted. No-op if the identity is already primary.
pub fn set_primary(
    conn: &mut Connection,
    user_ulid: Ulid,
    provider: &str,
    subject: &str,
) -> Result<()> {
    let tx = conn.transaction()?;
    let target: Option<i64> = tx
        .query_row(
            "SELECT id FROM _oidc_identities
              WHERE user_ulid = ?1 AND provider = ?2 AND subject = ?3",
            params![user_ulid.to_vec(), provider, subject],
            |r| r.get(0),
        )
        .optional()?;
    let id = target.ok_or(Error::NotFound)?;
    tx.execute(
        "UPDATE _oidc_identities SET is_primary = 0 WHERE user_ulid = ?1",
        params![user_ulid.to_vec()],
    )?;
    tx.execute(
        "UPDATE _oidc_identities SET is_primary = 1 WHERE id = ?1",
        params![id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Record a successful sign-in via this identity (updates `last_login_ms`).
pub fn touch_last_login(conn: &Connection, provider: &str, subject: &str) -> Result<()> {
    let now = crate::format::now_ms();
    conn.execute(
        "UPDATE _oidc_identities SET last_login_ms = ?1
          WHERE provider = ?2 AND subject = ?3",
        params![now, provider, subject],
    )?;
    Ok(())
}

/// Bootstrap an `(provider, subject)` identity for a freshly-minted user.
/// Used by the OIDC sign-in flow when no existing identity matches: the
/// account-join mode mints a new `user_ulid`, then this function records the
/// initial identity (auto-primary). Subsequent identities are added via the
/// link flow.
pub fn bootstrap_first_identity(
    conn: &Connection,
    user_ulid: Ulid,
    provider: &str,
    subject: &str,
    email: Option<&str>,
    display_label: Option<&str>,
) -> Result<Identity> {
    let now = crate::format::now_ms();
    let email_hash = email.map(|e| Sha256::digest(e.as_bytes()).to_vec());
    conn.execute(
        "INSERT INTO _oidc_identities
            (user_ulid, provider, subject, email_hash, display_label,
             is_primary, linked_at_ms, last_login_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)",
        params![
            user_ulid.to_vec(),
            provider,
            subject,
            email_hash,
            display_label,
            now,
        ],
    )?;
    let id = conn.last_insert_rowid();
    read_identity(conn, id)
}

/// Periodic cleanup: hard-delete completed or expired pending-link rows older
/// than `max_age_ms`. Returns count removed.
pub fn sweep_pending_links(conn: &Connection, now_ms: i64) -> Result<u64> {
    let n = conn.execute(
        "DELETE FROM _pending_identity_links
          WHERE (completed = 1 AND completed_at_ms < ?1)
             OR (completed = 0 AND expires_at_ms < ?2)",
        // Drop completed rows older than 24h; expired rows: now.
        params![now_ms - 24 * 3600 * 1000, now_ms],
    )?;
    Ok(n as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{open_or_create, DeploymentMode, OpenParams};

    fn open_db() -> Connection {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("identities.db");
        // Leak the tempdir so the file lives until the test process exits;
        // tests are tiny and short-lived.
        Box::leak(Box::new(dir));
        let (conn, _) = open_or_create(OpenParams {
            path: &path,
            cipher_key: &[],
            create_if_missing: true,
            create_mode: DeploymentMode::Primary,
            create_user_ulid: None,
        })
        .expect("open");
        conn
    }

    fn make_user_ulid(byte: u8) -> Ulid {
        let mut u = [0u8; 16];
        u[15] = byte;
        u
    }

    #[test]
    fn link_start_creates_pending_row() {
        let conn = open_db();
        let user = make_user_ulid(1);
        let outcome = link_identity_start(&conn, user, None, Some("google")).expect("link_start");
        assert!(!outcome.link_token.is_empty());
        assert!(outcome.expires_at_ms > crate::format::now_ms());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM _pending_identity_links", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn list_returns_empty_for_unknown_user() {
        let conn = open_db();
        let user = make_user_ulid(2);
        let rows = list_identities(&conn, user).expect("list");
        assert_eq!(rows.len(), 0);
    }

    #[test]
    fn bootstrap_then_list() {
        let conn = open_db();
        let user = make_user_ulid(3);
        let id = bootstrap_first_identity(
            &conn,
            user,
            "google",
            "subject-abc",
            Some("alice@example.com"),
            Some("Personal Google"),
        )
        .expect("bootstrap");
        assert!(id.is_primary);
        assert_eq!(id.provider, "google");
        let rows = list_identities(&conn, user).expect("list");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_primary);
    }

    #[test]
    fn unlink_last_identity_refused() {
        let mut conn = open_db();
        let user = make_user_ulid(4);
        bootstrap_first_identity(&conn, user, "google", "sub-only", None, None).expect("bootstrap");
        let res = unlink_identity(&mut conn, user, "google", "sub-only");
        assert!(matches!(res, Err(Error::OutOfScope)));
    }

    #[test]
    fn find_user_by_identity_resolves() {
        let conn = open_db();
        let user = make_user_ulid(5);
        bootstrap_first_identity(&conn, user, "facebook", "fb-sub", None, None).expect("bootstrap");
        let resolved = find_user_by_identity(&conn, "facebook", "fb-sub")
            .expect("find")
            .unwrap();
        assert_eq!(resolved, user);
        let none = find_user_by_identity(&conn, "facebook", "nope").expect("find");
        assert!(none.is_none());
    }

    #[test]
    fn set_primary_promotes_and_demotes() {
        let mut conn = open_db();
        let user = make_user_ulid(6);
        bootstrap_first_identity(&conn, user, "google", "sub-g", None, None).expect("g");
        // Insert a second identity manually via a successful link path simulation:
        let now = crate::format::now_ms();
        conn.execute(
            "INSERT INTO _oidc_identities
                (user_ulid, provider, subject, is_primary, linked_at_ms)
             VALUES (?1, 'facebook', 'sub-fb', 0, ?2)",
            params![user.to_vec(), now],
        )
        .unwrap();
        set_primary(&mut conn, user, "facebook", "sub-fb").expect("set_primary");
        let rows = list_identities(&conn, user).expect("list");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].provider, "facebook"); // primary first
        assert!(rows[0].is_primary);
        assert!(!rows[1].is_primary);
    }

    #[test]
    fn unlink_primary_promotes_next_oldest() {
        let mut conn = open_db();
        let user = make_user_ulid(7);
        bootstrap_first_identity(&conn, user, "google", "sub-g", None, None).expect("g");
        // Add a non-primary second identity. Use a different timestamp so
        // ordering is deterministic.
        let now = crate::format::now_ms();
        conn.execute(
            "INSERT INTO _oidc_identities
                (user_ulid, provider, subject, is_primary, linked_at_ms)
             VALUES (?1, 'facebook', 'sub-fb', 0, ?2)",
            params![user.to_vec(), now + 1000],
        )
        .unwrap();
        unlink_identity(&mut conn, user, "google", "sub-g").expect("unlink");
        let rows = list_identities(&conn, user).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, "facebook");
        assert!(rows[0].is_primary, "next-oldest promoted to primary");
    }
}
