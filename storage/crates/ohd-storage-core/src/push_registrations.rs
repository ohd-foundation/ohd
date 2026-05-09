//! Push registrations — FCM / APNs / web-push tokens for push-wake.
//!
//! `RegisterPushToken` stores a device's push provider token alongside the
//! user. The relay's push-wake path consumes these (out-of-band: a separate
//! "send the user a wakeup" channel keyed off `_push_registrations`). Storage
//! itself never sends pushes; it just keeps the tokens, exposes an
//! introspection surface (`ListPushDevices`), and revocation
//! (`UnregisterPushDevice`).
//!
//! Schema: `_push_registrations` (migration 009).

use rusqlite::{params, Connection, OptionalExtension};

use crate::ulid::Ulid;
use crate::{Error, Result};

/// One push registration row.
#[derive(Debug, Clone)]
pub struct PushRegistration {
    /// Synthesized wire ULID (encodes the rowid).
    pub ulid: Ulid,
    /// Platform: `"fcm"` / `"apns"` / `"web"` / `"email"`.
    pub platform: String,
    /// The push provider token. Plaintext — protected only by SQLCipher
    /// whole-file encryption.
    pub push_token: String,
    /// Registration time.
    pub registered_at_ms: i64,
    /// Last time the relay touched this row (for staleness reaping).
    pub last_seen_ms: Option<i64>,
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

/// Register or refresh a push token. Idempotent on `(platform, push_token)`:
/// re-registering the same token bumps `registered_at_ms` and clears
/// `revoked_at_ms`. Returns the (synthesized) wire ULID + the timestamp.
pub fn register_push(
    conn: &Connection,
    user_ulid: Ulid,
    platform: &str,
    push_token: &str,
    now_ms: i64,
) -> Result<(Ulid, i64)> {
    if platform.is_empty() || push_token.is_empty() {
        return Err(Error::InvalidArgument(
            "platform and push_token required".into(),
        ));
    }
    // Try to find an existing row first.
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM _push_registrations
              WHERE platform = ?1 AND push_token = ?2",
            params![platform, push_token],
            |r| r.get(0),
        )
        .optional()?;
    let id = if let Some(id) = existing {
        conn.execute(
            "UPDATE _push_registrations
                SET user_ulid = ?1, registered_at_ms = ?2, revoked_at_ms = NULL
              WHERE id = ?3",
            params![user_ulid.to_vec(), now_ms, id],
        )?;
        id
    } else {
        let rand_tail = crate::ulid::random_bytes(10);
        conn.execute(
            "INSERT INTO _push_registrations
                (ulid_random, user_ulid, platform, push_token, registered_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![rand_tail, user_ulid.to_vec(), platform, push_token, now_ms,],
        )?;
        conn.last_insert_rowid()
    };
    Ok((synth_ulid_from_rowid(id), now_ms))
}

/// Unregister a push token (by wire ULID).
pub fn unregister_push(
    conn: &Connection,
    user_ulid: Ulid,
    push_ulid: Ulid,
    now_ms: i64,
) -> Result<i64> {
    let rowid = rowid_from_ulid(push_ulid)?;
    let row: Option<Vec<u8>> = conn
        .query_row(
            "SELECT user_ulid FROM _push_registrations WHERE id = ?1",
            params![rowid],
            |r| r.get(0),
        )
        .optional()?;
    let owner_blob = row.ok_or(Error::NotFound)?;
    if owner_blob != user_ulid.to_vec() {
        return Err(Error::NotFound);
    }
    conn.execute(
        "UPDATE _push_registrations SET revoked_at_ms = ?1 WHERE id = ?2",
        params![now_ms, rowid],
    )?;
    Ok(now_ms)
}

/// List active push registrations for the user.
pub fn list_push(conn: &Connection, user_ulid: Ulid) -> Result<Vec<PushRegistration>> {
    let mut stmt = conn.prepare(
        "SELECT id, platform, push_token, registered_at_ms, last_seen_ms
           FROM _push_registrations
          WHERE user_ulid = ?1 AND revoked_at_ms IS NULL
          ORDER BY registered_at_ms DESC",
    )?;
    let mut iter = stmt.query_map(params![user_ulid.to_vec()], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<i64>>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    while let Some(row) = iter.next() {
        let (id, platform, token, registered, last_seen) = row?;
        out.push(PushRegistration {
            ulid: synth_ulid_from_rowid(id),
            platform,
            push_token: token,
            registered_at_ms: registered,
            last_seen_ms: last_seen,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{open_or_create, DeploymentMode, OpenParams};

    fn open_db() -> Connection {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("push.db");
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
    fn register_then_list() {
        let conn = open_db();
        let u = user(1);
        let now = crate::format::now_ms();
        let (_ulid, _) = register_push(&conn, u, "fcm", "fcm-tok-abc123", now).unwrap();
        let rows = list_push(&conn, u).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].platform, "fcm");
    }

    #[test]
    fn re_register_idempotent() {
        let conn = open_db();
        let u = user(2);
        let now = crate::format::now_ms();
        let (a, _) = register_push(&conn, u, "fcm", "tok-x", now).unwrap();
        let (b, _) = register_push(&conn, u, "fcm", "tok-x", now + 1).unwrap();
        assert_eq!(a, b, "same (platform, token) returns same ULID");
        let rows = list_push(&conn, u).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn unregister_drops_from_list() {
        let conn = open_db();
        let u = user(3);
        let now = crate::format::now_ms();
        let (ulid, _) = register_push(&conn, u, "apns", "apns-tok", now).unwrap();
        unregister_push(&conn, u, ulid, now + 1).unwrap();
        let rows = list_push(&conn, u).unwrap();
        assert!(rows.is_empty());
    }
}
