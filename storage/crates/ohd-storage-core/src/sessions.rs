//! Session management — list / revoke / logout-everywhere.
//!
//! "Sessions" in OHDC are self-session tokens (`ohds_…`) issued via the OIDC
//! flow. The wire shape lives in `proto/ohdc/v0/auth.proto` (`SessionInfo`).
//!
//! The underlying rows live in `_tokens` (created in migration 001); migration
//! 009 added `last_seen_ms`, `user_agent`, and `ip_origin` columns for
//! introspection. This module wraps the read/update side.
//!
//! # Session vs. token
//!
//! There is one logical session per `_tokens` row of `kind=ohds`. A user may
//! have many concurrent sessions (phone, web, desktop). Revoking a session
//! sets `_tokens.revoked_at_ms`; subsequent `auth::resolve_token` calls fail
//! with `Error::TokenRevoked`. `logout_everywhere` revokes *all* of the
//! user's `ohds_…` rows in one transaction.
//!
//! # Session ULIDs
//!
//! Sessions don't have a separate ULID column; we synthesize one from the
//! `_tokens.id` (rowid) for the wire surface. The synthesized ULID has a
//! zeroed time prefix and the rowid encoded into the random tail; this is
//! enough for the wire to address a session for revoke without exposing the
//! token hash. Round-trips through [`session_ulid_from_rowid`] /
//! [`session_rowid_from_ulid`].

use rusqlite::{params, Connection, OptionalExtension};

use crate::ulid::Ulid;
use crate::{Error, Result};

/// One row from `ListSessions`.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Synthesized session ULID (encodes the `_tokens.id` rowid).
    pub session_ulid: Ulid,
    /// `_tokens.issued_at_ms`.
    pub created_at_ms: i64,
    /// `_tokens.last_seen_ms` — populated by future session-touch path.
    pub last_seen_ms: Option<i64>,
    /// `_tokens.user_agent` — best effort, may be NULL.
    pub user_agent: Option<String>,
    /// `_tokens.ip_origin` — best effort, may be NULL.
    pub ip_origin: Option<String>,
    /// Display label provided at issuance.
    pub label: Option<String>,
}

/// Encode a `_tokens.id` rowid into a session ULID. The wire shape is just an
/// opaque 16-byte identifier — we put the rowid at byte 8.. (8 bytes BE) so
/// the inverse is a simple slice; the leading 8 bytes are zero.
pub fn session_ulid_from_rowid(rowid: i64) -> Ulid {
    let mut out = [0u8; 16];
    out[8..].copy_from_slice(&rowid.to_be_bytes());
    out
}

/// Inverse of [`session_ulid_from_rowid`]. Returns `Error::InvalidUlid` if the
/// ULID isn't shaped like a synthesized session ULID (non-zero leading bytes).
pub fn session_rowid_from_ulid(ulid: Ulid) -> Result<i64> {
    if ulid[..8] != [0u8; 8] {
        return Err(Error::InvalidUlid);
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&ulid[8..]);
    Ok(i64::from_be_bytes(buf))
}

/// List every active self-session token bound to the user.
///
/// Filters out revoked / expired rows. Ordered by `issued_at_ms DESC` (most
/// recently-issued first).
pub fn list_active_sessions(
    conn: &Connection,
    user_ulid: Ulid,
    now_ms: i64,
) -> Result<Vec<SessionInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, issued_at_ms, last_seen_ms, user_agent, ip_origin, label, expires_at_ms, revoked_at_ms
           FROM _tokens
          WHERE token_prefix = 'ohds' AND user_ulid = ?1
          ORDER BY issued_at_ms DESC",
    )?;
    let mut iter = stmt.query_map(params![user_ulid.to_vec()], |r| {
        Ok((
            r.get::<_, i64>(0)?,            // id
            r.get::<_, i64>(1)?,            // issued_at_ms
            r.get::<_, Option<i64>>(2)?,    // last_seen_ms
            r.get::<_, Option<String>>(3)?, // user_agent
            r.get::<_, Option<String>>(4)?, // ip_origin
            r.get::<_, Option<String>>(5)?, // label
            r.get::<_, Option<i64>>(6)?,    // expires_at_ms
            r.get::<_, Option<i64>>(7)?,    // revoked_at_ms
        ))
    })?;
    let mut out = Vec::new();
    while let Some(row) = iter.next() {
        let (id, issued, last_seen, ua, ip, label, exp, rev) = row?;
        if let Some(r) = rev {
            if r <= now_ms {
                continue;
            }
        }
        if let Some(e) = exp {
            if e <= now_ms {
                continue;
            }
        }
        out.push(SessionInfo {
            session_ulid: session_ulid_from_rowid(id),
            created_at_ms: issued,
            last_seen_ms: last_seen,
            user_agent: ua,
            ip_origin: ip,
            label,
        });
    }
    Ok(out)
}

/// Revoke one session by its synthesized ULID. Returns the revoke timestamp.
///
/// Errors with `Error::NotFound` when no matching `_tokens` row exists for the
/// user. Idempotent: if the row is already revoked, returns the existing
/// `revoked_at_ms`.
pub fn revoke_session(
    conn: &Connection,
    user_ulid: Ulid,
    session_ulid: Ulid,
    now_ms: i64,
) -> Result<i64> {
    let rowid = session_rowid_from_ulid(session_ulid)?;
    let row: Option<(Vec<u8>, Option<i64>)> = conn
        .query_row(
            "SELECT user_ulid, revoked_at_ms FROM _tokens
              WHERE id = ?1 AND token_prefix = 'ohds'",
            params![rowid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (owner_blob, already) = row.ok_or(Error::NotFound)?;
    if owner_blob != user_ulid.to_vec() {
        return Err(Error::NotFound);
    }
    if let Some(prev) = already {
        return Ok(prev);
    }
    conn.execute(
        "UPDATE _tokens SET revoked_at_ms = ?1 WHERE id = ?2",
        params![now_ms, rowid],
    )?;
    Ok(now_ms)
}

/// Revoke every active session for the user. Returns the count of newly
/// revoked rows.
///
/// Used by both `Logout` (which is just "revoke the caller's own session") and
/// `LogoutEverywhere` (revoke all of the user's sessions). The handler picks
/// which surface to expose by passing the right `user_ulid`.
pub fn revoke_all_sessions(conn: &Connection, user_ulid: Ulid, now_ms: i64) -> Result<i64> {
    let n = conn.execute(
        "UPDATE _tokens
            SET revoked_at_ms = ?1
          WHERE token_prefix = 'ohds'
            AND user_ulid = ?2
            AND revoked_at_ms IS NULL",
        params![now_ms, user_ulid.to_vec()],
    )?;
    Ok(n as i64)
}

/// Touch a session's `last_seen_ms`. Best-effort; ignores rows that don't
/// exist or are already revoked.
#[allow(dead_code)]
pub fn touch_session(conn: &Connection, session_token_id: i64, now_ms: i64) -> Result<()> {
    conn.execute(
        "UPDATE _tokens SET last_seen_ms = ?1
          WHERE id = ?2 AND token_prefix = 'ohds' AND revoked_at_ms IS NULL",
        params![now_ms, session_token_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{issue_self_session_token, resolve_token};
    use crate::format::{open_or_create, DeploymentMode, OpenParams};

    fn open_db() -> Connection {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sessions.db");
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
    fn session_ulid_round_trip() {
        let r = 12345i64;
        let u = session_ulid_from_rowid(r);
        assert_eq!(session_rowid_from_ulid(u).unwrap(), r);
    }

    #[test]
    fn list_returns_active_sessions() {
        let conn = open_db();
        let user = make_user_ulid(1);
        issue_self_session_token(&conn, user, Some("phone"), None).unwrap();
        issue_self_session_token(&conn, user, Some("web"), None).unwrap();
        let now = crate::format::now_ms();
        let rows = list_active_sessions(&conn, user, now).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].label.is_some());
    }

    #[test]
    fn revoke_session_one() {
        let conn = open_db();
        let user = make_user_ulid(2);
        issue_self_session_token(&conn, user, Some("phone"), None).unwrap();
        let now = crate::format::now_ms();
        let rows = list_active_sessions(&conn, user, now).unwrap();
        assert_eq!(rows.len(), 1);
        revoke_session(&conn, user, rows[0].session_ulid, now + 1).unwrap();
        let rows2 = list_active_sessions(&conn, user, now + 2).unwrap();
        assert!(rows2.is_empty());
    }

    #[test]
    fn revoke_all_sessions_drains() {
        let conn = open_db();
        let user = make_user_ulid(3);
        let t1 = issue_self_session_token(&conn, user, Some("a"), None).unwrap();
        let t2 = issue_self_session_token(&conn, user, Some("b"), None).unwrap();
        // Stamp the revoke `at_ms` in the past so resolve_token's `rev <= now`
        // check fires regardless of clock resolution noise.
        let n = revoke_all_sessions(&conn, user, 1).unwrap();
        assert_eq!(n, 2);
        // Both tokens now fail to resolve.
        assert!(resolve_token(&conn, &t1).is_err());
        assert!(resolve_token(&conn, &t2).is_err());
    }

    #[test]
    fn revoke_session_unknown_user_rejected() {
        let conn = open_db();
        let user = make_user_ulid(4);
        let stranger = make_user_ulid(5);
        issue_self_session_token(&conn, user, None, None).unwrap();
        let rows = list_active_sessions(&conn, user, crate::format::now_ms()).unwrap();
        let res = revoke_session(
            &conn,
            stranger,
            rows[0].session_ulid,
            crate::format::now_ms(),
        );
        assert!(matches!(res, Err(Error::NotFound)));
    }
}
