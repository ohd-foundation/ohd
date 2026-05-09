//! Notification preferences — quiet hours + (future) per-event-type opt-in.
//!
//! Per `spec/auth.md` (`UpdateNotificationPreferences`) and
//! `spec/notifications.md`. v1 surface: quiet-hours-on-flag + start hour +
//! end hour + IANA timezone. The relay's notification dispatcher reads from
//! this table when deciding whether to deliver an event-driven notification.
//!
//! Schema: `_notification_config` (migration 009). Singleton per user.

use rusqlite::{params, Connection, OptionalExtension};

use crate::ulid::Ulid;
use crate::{Error, Result};

/// Materialized notification config row.
#[derive(Debug, Clone, Default)]
pub struct NotificationConfig {
    /// Whether quiet hours are observed.
    pub quiet_hours_enabled: bool,
    /// Quiet hours start (0..23, local hour).
    pub quiet_hours_start: Option<i32>,
    /// Quiet hours end (0..23, local hour).
    pub quiet_hours_end: Option<i32>,
    /// IANA timezone, e.g. `"Europe/Prague"`.
    pub quiet_hours_tz: Option<String>,
    /// Last update time.
    pub updated_at_ms: i64,
}

/// Fetch the notification config for the user. Returns the default config
/// (quiet hours off) if no row exists.
pub fn get_notification_config(conn: &Connection, user_ulid: Ulid) -> Result<NotificationConfig> {
    let row: Option<(i64, Option<i32>, Option<i32>, Option<String>, i64)> = conn
        .query_row(
            "SELECT quiet_hours_enabled, quiet_hours_start, quiet_hours_end,
                    quiet_hours_tz, updated_at_ms
               FROM _notification_config
              WHERE user_ulid = ?1",
            params![user_ulid.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    Ok(match row {
        Some((qe, qs, qend, qtz, updated)) => NotificationConfig {
            quiet_hours_enabled: qe != 0,
            quiet_hours_start: qs,
            quiet_hours_end: qend,
            quiet_hours_tz: qtz,
            updated_at_ms: updated,
        },
        None => NotificationConfig::default(),
    })
}

/// Sparse update payload. `None` fields are left unchanged.
#[derive(Debug, Clone, Default)]
pub struct NotificationConfigUpdate {
    /// Set `quiet_hours_enabled`.
    pub quiet_hours_enabled: Option<bool>,
    /// Set `quiet_hours_start` (0..23).
    pub quiet_hours_start: Option<i32>,
    /// Set `quiet_hours_end` (0..23).
    pub quiet_hours_end: Option<i32>,
    /// Set `quiet_hours_tz` (IANA zone).
    pub quiet_hours_tz: Option<String>,
}

fn validate_hour(h: Option<i32>) -> Result<()> {
    if let Some(h) = h {
        if !(0..=23).contains(&h) {
            return Err(Error::InvalidArgument(format!(
                "quiet hour out of range: {h}"
            )));
        }
    }
    Ok(())
}

/// Update the user's notification config. Upserts the row.
pub fn update_notification_config(
    conn: &Connection,
    user_ulid: Ulid,
    update: &NotificationConfigUpdate,
    now_ms: i64,
) -> Result<NotificationConfig> {
    validate_hour(update.quiet_hours_start)?;
    validate_hour(update.quiet_hours_end)?;
    let current = get_notification_config(conn, user_ulid)?;
    let new = NotificationConfig {
        quiet_hours_enabled: update
            .quiet_hours_enabled
            .unwrap_or(current.quiet_hours_enabled),
        quiet_hours_start: update.quiet_hours_start.or(current.quiet_hours_start),
        quiet_hours_end: update.quiet_hours_end.or(current.quiet_hours_end),
        quiet_hours_tz: update
            .quiet_hours_tz
            .clone()
            .or(current.quiet_hours_tz.clone()),
        updated_at_ms: now_ms,
    };
    conn.execute(
        "INSERT INTO _notification_config
            (user_ulid, quiet_hours_enabled, quiet_hours_start, quiet_hours_end,
             quiet_hours_tz, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(user_ulid) DO UPDATE SET
            quiet_hours_enabled = excluded.quiet_hours_enabled,
            quiet_hours_start   = excluded.quiet_hours_start,
            quiet_hours_end     = excluded.quiet_hours_end,
            quiet_hours_tz      = excluded.quiet_hours_tz,
            updated_at_ms       = excluded.updated_at_ms",
        params![
            user_ulid.to_vec(),
            if new.quiet_hours_enabled { 1 } else { 0 },
            new.quiet_hours_start,
            new.quiet_hours_end,
            new.quiet_hours_tz,
            new.updated_at_ms,
        ],
    )?;
    Ok(new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{open_or_create, DeploymentMode, OpenParams};

    fn open_db() -> Connection {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("notif.db");
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
    fn default_then_update() {
        let conn = open_db();
        let u = user(1);
        let cfg = get_notification_config(&conn, u).unwrap();
        assert!(!cfg.quiet_hours_enabled);
        let update = NotificationConfigUpdate {
            quiet_hours_enabled: Some(true),
            quiet_hours_start: Some(22),
            quiet_hours_end: Some(7),
            quiet_hours_tz: Some("Europe/Prague".into()),
        };
        let now = crate::format::now_ms();
        let updated = update_notification_config(&conn, u, &update, now).unwrap();
        assert!(updated.quiet_hours_enabled);
        assert_eq!(updated.quiet_hours_start, Some(22));
        // Re-read.
        let cfg = get_notification_config(&conn, u).unwrap();
        assert!(cfg.quiet_hours_enabled);
        assert_eq!(cfg.quiet_hours_tz.as_deref(), Some("Europe/Prague"));
    }

    #[test]
    fn invalid_hour_rejected() {
        let conn = open_db();
        let u = user(2);
        let update = NotificationConfigUpdate {
            quiet_hours_start: Some(99),
            ..Default::default()
        };
        let res = update_notification_config(&conn, u, &update, 0);
        assert!(matches!(res, Err(Error::InvalidArgument(_))));
    }

    #[test]
    fn sparse_update_preserves_other_fields() {
        let conn = open_db();
        let u = user(3);
        update_notification_config(
            &conn,
            u,
            &NotificationConfigUpdate {
                quiet_hours_enabled: Some(true),
                quiet_hours_start: Some(22),
                quiet_hours_end: Some(7),
                quiet_hours_tz: Some("Europe/Prague".into()),
            },
            0,
        )
        .unwrap();
        // Sparse update — only flip enabled to false.
        update_notification_config(
            &conn,
            u,
            &NotificationConfigUpdate {
                quiet_hours_enabled: Some(false),
                ..Default::default()
            },
            1,
        )
        .unwrap();
        let cfg = get_notification_config(&conn, u).unwrap();
        assert!(!cfg.quiet_hours_enabled);
        assert_eq!(cfg.quiet_hours_start, Some(22));
        assert_eq!(cfg.quiet_hours_end, Some(7));
        assert_eq!(cfg.quiet_hours_tz.as_deref(), Some("Europe/Prague"));
    }
}
