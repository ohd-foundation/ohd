//! Out-of-band invitations.
//!
//! Two flavours per `spec/auth.md` "Invitations":
//!
//! - **Self-invite** (Model 1: invite-only deployments) — operator issues a
//!   token an unregistered user redeems on first login to bind their fresh
//!   `user_ulid` to the inviter's "introduced by" lineage.
//! - **Read-share invite** — used by Connect's "share my data with X" flow.
//!   User issues a redeem token, the recipient (often a clinician without an
//!   OHD account yet) redeems it via `AcceptInvite`, the storage daemon mints
//!   a grant on the inviter's behalf with a default scope.
//!
//! v1 lands the row shape, the issue / list / revoke trio, and a stub
//! `accept_invite` that marks the invite redeemed but does *not* mint a grant
//! (the grant minting will land alongside the operator-side enrollment flow,
//! which lives in Connect).
//!
//! Schema in `migrations/009_auth_sessions_etc.sql`.

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::ulid::Ulid;
use crate::{Error, Result};

/// Bearer prefix on invite tokens. Wire form: `ohdv_<base32>`.
pub const INVITE_PREFIX: &str = "ohdv";

/// Default invite TTL: 14 days.
pub const DEFAULT_INVITE_TTL_MS: i64 = 14 * 24 * 3600 * 1000;

/// One materialized invite row.
#[derive(Debug, Clone)]
pub struct Invite {
    /// Wire ULID (synthesized from rowid; same shape as session ULID).
    pub ulid: Ulid,
    /// Optional email this invite is bound to (case-insensitive when matched).
    pub email_bound: Option<String>,
    /// Free-text note from the issuer.
    pub note: Option<String>,
    /// Issued time (UTC ms).
    pub issued_at_ms: i64,
    /// Expiry (UTC ms). None = never expires.
    pub expires_at_ms: Option<i64>,
    /// Set when redeemed.
    pub redeemed_at_ms: Option<i64>,
    /// Set when revoked.
    pub revoked_at_ms: Option<i64>,
}

/// Outcome of [`create_invite`].
#[derive(Debug, Clone)]
pub struct CreatedInvite {
    /// Invite row.
    pub invite: Invite,
    /// Cleartext bearer token. Shown exactly once.
    pub bearer: String,
}

/// SHA-256 hash a bearer body — same construction as `auth::hash_token` but
/// scoped here to keep the invite store independent of the session-token
/// table.
fn hash(bearer: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bearer.as_bytes());
    h.finalize().into()
}

fn synth_ulid_from_rowid(rowid: i64) -> Ulid {
    let mut o = [0u8; 16];
    o[8..].copy_from_slice(&rowid.to_be_bytes());
    o
}

fn rowid_from_ulid(ulid: Ulid) -> Result<i64> {
    if ulid[..8] != [0u8; 8] {
        return Err(Error::InvalidUlid);
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&ulid[8..]);
    Ok(i64::from_be_bytes(buf))
}

/// Create a new invite. Returns the row + the cleartext bearer (shown once).
pub fn create_invite(
    conn: &Connection,
    issuer_user_ulid: Ulid,
    email_bound: Option<&str>,
    expires_at_ms: Option<i64>,
    note: Option<&str>,
    now_ms: i64,
) -> Result<CreatedInvite> {
    let body = base32::encode(
        base32::Alphabet::Rfc4648 { padding: false },
        &crate::ulid::random_bytes(32),
    );
    let bearer = format!("{INVITE_PREFIX}_{body}");
    let token_hash = hash(&bearer);
    let rand_tail = crate::ulid::random_bytes(10);
    conn.execute(
        "INSERT INTO _pending_invites
            (ulid_random, invite_token_hash, issuer_user_ulid, email_bound,
             note, issued_at_ms, expires_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            rand_tail,
            token_hash.to_vec(),
            issuer_user_ulid.to_vec(),
            email_bound,
            note,
            now_ms,
            expires_at_ms,
        ],
    )?;
    let id = conn.last_insert_rowid();
    Ok(CreatedInvite {
        invite: Invite {
            ulid: synth_ulid_from_rowid(id),
            email_bound: email_bound.map(str::to_string),
            note: note.map(str::to_string),
            issued_at_ms: now_ms,
            expires_at_ms,
            redeemed_at_ms: None,
            revoked_at_ms: None,
        },
        bearer,
    })
}

/// List every invite ever issued by `issuer_user_ulid`, including redeemed
/// and revoked ones. Ordered by `issued_at_ms DESC`.
pub fn list_invites(conn: &Connection, issuer_user_ulid: Ulid) -> Result<Vec<Invite>> {
    let mut stmt = conn.prepare(
        "SELECT id, email_bound, note, issued_at_ms, expires_at_ms,
                redeemed_at_ms, revoked_at_ms
           FROM _pending_invites
          WHERE issuer_user_ulid = ?1
          ORDER BY issued_at_ms DESC",
    )?;
    let mut iter = stmt.query_map(params![issuer_user_ulid.to_vec()], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, Option<i64>>(5)?,
            r.get::<_, Option<i64>>(6)?,
        ))
    })?;
    let mut out = Vec::new();
    while let Some(row) = iter.next() {
        let (id, email, note, issued, expires, redeemed, revoked) = row?;
        out.push(Invite {
            ulid: synth_ulid_from_rowid(id),
            email_bound: email,
            note,
            issued_at_ms: issued,
            expires_at_ms: expires,
            redeemed_at_ms: redeemed,
            revoked_at_ms: revoked,
        });
    }
    Ok(out)
}

/// Revoke an invite by its wire ULID.
pub fn revoke_invite(
    conn: &Connection,
    issuer_user_ulid: Ulid,
    invite_ulid: Ulid,
    now_ms: i64,
) -> Result<i64> {
    let rowid = rowid_from_ulid(invite_ulid)?;
    let row: Option<(Vec<u8>, Option<i64>)> = conn
        .query_row(
            "SELECT issuer_user_ulid, revoked_at_ms FROM _pending_invites WHERE id = ?1",
            params![rowid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (owner_blob, already) = row.ok_or(Error::NotFound)?;
    if owner_blob != issuer_user_ulid.to_vec() {
        return Err(Error::NotFound);
    }
    if let Some(prev) = already {
        return Ok(prev);
    }
    conn.execute(
        "UPDATE _pending_invites SET revoked_at_ms = ?1 WHERE id = ?2",
        params![now_ms, rowid],
    )?;
    Ok(now_ms)
}

/// Outcome of redemption: the invite is marked redeemed and the redeeming
/// user_ulid is recorded. Grant minting (the "share data with X" path) is a
/// follow-up step the caller performs separately.
#[derive(Debug, Clone)]
pub struct AcceptOutcome {
    /// `id` of the redeemed `_pending_invites` row (callers can use this to
    /// chain a follow-up grant insert).
    pub invite_id: i64,
    /// `issuer_user_ulid` from the invite row (the data-owner being shared
    /// FROM — useful for a downstream grant insert).
    pub issuer_user_ulid: Ulid,
    /// `redeemed_at_ms` stamp.
    pub redeemed_at_ms: i64,
}

/// Redeem an invite. Validates it isn't already redeemed / revoked / expired.
pub fn accept_invite(
    conn: &Connection,
    invite_bearer: &str,
    redeemer_user_ulid: Ulid,
    now_ms: i64,
) -> Result<AcceptOutcome> {
    let prefix = invite_bearer
        .split_once('_')
        .map(|(p, _)| p)
        .unwrap_or_default();
    if prefix != INVITE_PREFIX {
        return Err(Error::InvalidArgument("invite token: bad prefix".into()));
    }
    let token_hash = hash(invite_bearer);
    let row: Option<(i64, Vec<u8>, Option<i64>, Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT id, issuer_user_ulid, expires_at_ms, redeemed_at_ms, revoked_at_ms
               FROM _pending_invites
              WHERE invite_token_hash = ?1",
            params![token_hash.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let (id, issuer_blob, expires, redeemed, revoked) =
        row.ok_or_else(|| Error::InvalidArgument("invite token: not found".into()))?;
    if revoked.is_some() {
        return Err(Error::TokenRevoked);
    }
    if redeemed.is_some() {
        return Err(Error::InvalidArgument(
            "invite token: already redeemed".into(),
        ));
    }
    if let Some(e) = expires {
        if e <= now_ms {
            return Err(Error::TokenExpired);
        }
    }
    let issuer_user_ulid: Ulid = issuer_blob
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidUlid)?;
    conn.execute(
        "UPDATE _pending_invites
            SET redeemed_at_ms = ?1, redeemed_by_user_ulid = ?2
          WHERE id = ?3",
        params![now_ms, redeemer_user_ulid.to_vec(), id],
    )?;
    Ok(AcceptOutcome {
        invite_id: id,
        issuer_user_ulid,
        redeemed_at_ms: now_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{open_or_create, DeploymentMode, OpenParams};

    fn open_db() -> Connection {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("invites.db");
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

    fn user(byte: u8) -> Ulid {
        let mut u = [0u8; 16];
        u[15] = byte;
        u
    }

    #[test]
    fn create_then_list() {
        let conn = open_db();
        let issuer = user(1);
        let now = crate::format::now_ms();
        let c = create_invite(
            &conn,
            issuer,
            Some("alice@example.com"),
            Some(now + 1_000_000),
            Some("for alice"),
            now,
        )
        .unwrap();
        assert!(c.bearer.starts_with("ohdv_"));
        let rows = list_invites(&conn, issuer).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].email_bound.as_deref(), Some("alice@example.com"));
    }

    #[test]
    fn accept_then_re_accept_fails() {
        let conn = open_db();
        let issuer = user(2);
        let redeemer = user(3);
        let now = crate::format::now_ms();
        let c = create_invite(&conn, issuer, None, None, None, now).unwrap();
        let outcome = accept_invite(&conn, &c.bearer, redeemer, now + 1).unwrap();
        assert_eq!(outcome.issuer_user_ulid, issuer);
        let res = accept_invite(&conn, &c.bearer, redeemer, now + 2);
        assert!(matches!(res, Err(Error::InvalidArgument(_))));
    }

    #[test]
    fn revoke_then_accept_fails() {
        let conn = open_db();
        let issuer = user(4);
        let redeemer = user(5);
        let now = crate::format::now_ms();
        let c = create_invite(&conn, issuer, None, None, None, now).unwrap();
        revoke_invite(&conn, issuer, c.invite.ulid, now + 1).unwrap();
        let res = accept_invite(&conn, &c.bearer, redeemer, now + 2);
        assert!(matches!(res, Err(Error::TokenRevoked)));
    }

    #[test]
    fn expired_invite_rejected() {
        let conn = open_db();
        let issuer = user(6);
        let redeemer = user(7);
        let now = crate::format::now_ms();
        let c = create_invite(&conn, issuer, None, Some(now - 1000), None, now).unwrap();
        let res = accept_invite(&conn, &c.bearer, redeemer, now);
        assert!(matches!(res, Err(Error::TokenExpired)));
    }
}
