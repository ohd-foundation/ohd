//! Device tokens (Model 3 — in-app issuance for sensors / bridges).
//!
//! A device token is a write-only grant the user issues for a sensor or
//! integration the user themselves controls (Health Connect bridge, manual
//! logger, integration daemon). Token wire form: `ohdd_<base32>`.
//!
//! This module wraps the trio:
//! - [`issue_device_token`] — create a device-kind grant + bearer token, persist
//!   the device-shaped metadata (label, kind, allowed event types).
//! - [`list_device_tokens`] — return the issued device tokens for the user
//!   (introspection helper for `AuthService.ListDeviceTokens`).
//! - [`revoke_device_token`] — revoke both the grant row + the token row.
//!
//! Schema: `_device_token_grants` (migration 009) holds the device-specific
//! metadata; the grant row enforces the event-type allowlist via the existing
//! `grant_event_type_rules` machinery.

use rusqlite::{params, Connection, OptionalExtension};

use crate::auth::{issue_grant_token, TokenKind};
use crate::grants::{create_grant, NewGrant, RuleEffect};
use crate::ulid::Ulid;
use crate::{Error, Result};

/// One row from `ListDeviceTokens`.
#[derive(Debug, Clone)]
pub struct DeviceTokenInfo {
    /// Grant rowid (joinable into `grants`).
    pub grant_id: i64,
    /// Wire grant ULID.
    pub grant_ulid: Ulid,
    /// Device label.
    pub device_label: String,
    /// Device kind (e.g. `"health_connect_bridge"`, `"manual_logger"`).
    pub device_kind: String,
    /// CSV of allowed event types.
    pub event_types: Vec<String>,
    /// Issuance time.
    pub issued_at_ms: i64,
    /// Revocation time, if any.
    pub revoked_at_ms: Option<i64>,
}

/// Outcome of [`issue_device_token`].
#[derive(Debug, Clone)]
pub struct IssuedDeviceToken {
    /// Wire bearer token. Shown exactly once.
    pub bearer: String,
    /// Grant rowid.
    pub grant_id: i64,
    /// Wire grant ULID.
    pub grant_ulid: Ulid,
}

/// Mint a device token bound to a fresh `kind=device` grant.
///
/// The grant's write-side rules allowlist exactly the supplied
/// `event_types`. The default action is `deny` (closed-by-default per
/// `spec/privacy-access.md` "default deny"); since device tokens can only
/// `PutEvents`, the read path is blocked at the auth-kind matrix anyway.
pub fn issue_device_token(
    conn: &mut Connection,
    user_ulid: Ulid,
    device_label: &str,
    device_kind: &str,
    event_types: &[String],
    now_ms: i64,
) -> Result<IssuedDeviceToken> {
    if device_label.is_empty() {
        return Err(Error::InvalidArgument("device_label required".into()));
    }
    if device_kind.is_empty() {
        return Err(Error::InvalidArgument("device_kind required".into()));
    }
    let write_rules: Vec<(String, RuleEffect)> = event_types
        .iter()
        .map(|t| (t.clone(), RuleEffect::Allow))
        .collect();

    let new_grant = NewGrant {
        grantee_label: device_label.to_string(),
        grantee_kind: "device".to_string(),
        purpose: Some(format!("device token: {device_kind}")),
        default_action: RuleEffect::Deny,
        approval_mode: "never_required".to_string(),
        expires_at_ms: None,
        event_type_rules: vec![],
        channel_rules: vec![],
        sensitivity_rules: vec![],
        write_event_type_rules: write_rules,
        auto_approve_event_types: event_types.to_vec(),
        aggregation_only: false,
        strip_notes: false,
        notify_on_access: false,
        require_approval_per_query: false,
        max_queries_per_day: None,
        max_queries_per_hour: None,
        rolling_window_days: None,
        absolute_window: None,
        delegate_for_user_ulid: None,
        grantee_recovery_pubkey: None,
    };

    let (grant_id, grant_ulid) = create_grant(conn, &new_grant)?;

    let event_types_csv = event_types.join(",");
    conn.execute(
        "INSERT INTO _device_token_grants
            (grant_id, device_label, device_kind, event_types_csv, issued_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![grant_id, device_label, device_kind, event_types_csv, now_ms],
    )?;

    let bearer = issue_grant_token(conn, user_ulid, grant_id, TokenKind::Device, None)?;
    Ok(IssuedDeviceToken {
        bearer,
        grant_id,
        grant_ulid,
    })
}

/// List every device token ever issued for the user (active + revoked).
pub fn list_device_tokens(conn: &Connection, user_ulid: Ulid) -> Result<Vec<DeviceTokenInfo>> {
    let mut stmt = conn.prepare(
        "SELECT g.id, g.ulid_random, g.created_at_ms, g.revoked_at_ms,
                d.device_label, d.device_kind, d.event_types_csv, d.issued_at_ms
           FROM grants g
           JOIN _device_token_grants d ON d.grant_id = g.id
           JOIN _tokens t ON t.grant_id = g.id AND t.user_ulid = ?1
              AND t.token_prefix = 'ohdd'
          WHERE g.grantee_kind = 'device'
          GROUP BY g.id
          ORDER BY g.created_at_ms DESC",
    )?;
    let mut iter = stmt.query_map(params![user_ulid.to_vec()], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Vec<u8>>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, Option<i64>>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, i64>(7)?,
        ))
    })?;
    let mut out = Vec::new();
    while let Some(row) = iter.next() {
        let (id, rand_tail, _created_at, revoked, label, kind, etypes_csv, issued) = row?;
        let mut grant_ulid = [0u8; 16];
        if rand_tail.len() == 10 {
            grant_ulid[6..].copy_from_slice(&rand_tail);
        }
        let event_types = etypes_csv
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        out.push(DeviceTokenInfo {
            grant_id: id,
            grant_ulid,
            device_label: label,
            device_kind: kind,
            event_types,
            issued_at_ms: issued,
            revoked_at_ms: revoked,
        });
    }
    Ok(out)
}

/// Revoke a device token: marks both the underlying grant row and any matching
/// `_tokens` row as revoked. Returns the revoke timestamp.
pub fn revoke_device_token(
    conn: &Connection,
    user_ulid: Ulid,
    grant_id: i64,
    now_ms: i64,
) -> Result<i64> {
    // Verify the grant belongs to a device token of this user.
    let row: Option<(String, Option<i64>)> = conn
        .query_row(
            "SELECT g.grantee_kind, g.revoked_at_ms
               FROM grants g
               JOIN _tokens t ON t.grant_id = g.id
              WHERE g.id = ?1 AND t.user_ulid = ?2 AND t.token_prefix = 'ohdd'",
            params![grant_id, user_ulid.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (kind, already) = row.ok_or(Error::NotFound)?;
    if kind != "device" {
        return Err(Error::InvalidArgument(
            "grant_id is not a device-kind grant".into(),
        ));
    }
    if let Some(prev) = already {
        return Ok(prev);
    }
    conn.execute(
        "UPDATE grants SET revoked_at_ms = ?1 WHERE id = ?2",
        params![now_ms, grant_id],
    )?;
    conn.execute(
        "UPDATE _tokens SET revoked_at_ms = ?1
          WHERE grant_id = ?2 AND token_prefix = 'ohdd' AND revoked_at_ms IS NULL",
        params![now_ms, grant_id],
    )?;
    Ok(now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{open_or_create, DeploymentMode, OpenParams};

    fn open_db() -> Connection {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("device.db");
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
    fn issue_then_list() {
        let mut conn = open_db();
        let u = user(1);
        let now = crate::format::now_ms();
        let issued = issue_device_token(
            &mut conn,
            u,
            "Garmin Bridge",
            "health_connect_bridge",
            &["std.heart_rate_resting".to_string()],
            now,
        )
        .unwrap();
        assert!(issued.bearer.starts_with("ohdd_"));
        let rows = list_device_tokens(&conn, u).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].device_label, "Garmin Bridge");
        assert_eq!(rows[0].event_types.len(), 1);
    }

    #[test]
    fn revoke_makes_token_unusable() {
        let mut conn = open_db();
        let u = user(2);
        let now = crate::format::now_ms();
        let issued = issue_device_token(&mut conn, u, "Sensor", "manual_logger", &[], now).unwrap();
        // Stamp revoke in the past so `rev <= now` fires immediately.
        let rev = revoke_device_token(&conn, u, issued.grant_id, 1).unwrap();
        assert_eq!(rev, 1);
        // Token should now fail to resolve due to grant revocation.
        let res = crate::auth::resolve_token(&conn, &issued.bearer);
        assert!(matches!(res, Err(Error::TokenRevoked)));
    }

    #[test]
    fn revoke_unknown_user_rejected() {
        let mut conn = open_db();
        let owner = user(3);
        let stranger = user(4);
        let now = crate::format::now_ms();
        let issued = issue_device_token(&mut conn, owner, "X", "manual_logger", &[], now).unwrap();
        let res = revoke_device_token(&conn, stranger, issued.grant_id, now + 1);
        assert!(matches!(res, Err(Error::NotFound)));
    }
}
