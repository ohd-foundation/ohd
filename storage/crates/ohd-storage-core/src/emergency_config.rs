//! Emergency / break-glass configuration (operator-side state).
//!
//! Per `connect/spec/screens-emergency.md` "OHD Connect — patient side →
//! Settings tab: 'Emergency / Break-glass'". Stores the patient's
//! emergency-access preferences as a singleton row keyed by `user_ulid` in
//! `_emergency_config` (migration 017).
//!
//! # Why operator-side
//!
//! The emergency feature is patient policy that drives behaviour across
//! every OHD surface (Connect bystander proxy, Emergency tablet break-glass
//! flow, the relay's broadcast / discovery). Storing it inside the per-user
//! storage file keeps the state authoritative on the patient's device and
//! lets multiple Connect surfaces (Android, web, desktop) see the same
//! configuration without a cloud round-trip.
//!
//! # Sections
//!
//! Mirrors the eight sections of `screens-emergency.md`:
//!
//! 1. **Feature toggle**         → `enabled`
//! 2. **Discovery**              → `bluetooth_beacon`
//! 3. **Approval timing**        → `approval_timeout_seconds`,
//!                                 `default_action_on_timeout`
//! 4. **Lock-screen behaviour**  → `lock_screen_visibility`
//! 5. **What responders see**    → `history_window_hours`,
//!                                 `channel_paths_allowed`,
//!                                 `sensitivity_classes_allowed`
//! 6. **Location**               → `share_location`
//! 7. **Trusted authorities**    → `trusted_authorities`
//! 8. **Advanced**               → `bystander_proxy_enabled`

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::ulid::Ulid;
use crate::{Error, Result};

/// Materialized emergency-config row.
///
/// Defaults follow the spec's "spec mandates these as the ship-default"
/// values: feature off, beacon on, 30s timeout, allow-on-timeout,
/// full-dialog visibility, 24h history, no location share, bystander proxy
/// on, empty allowlists (UI fills them on first enable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergencyConfig {
    /// Section 1 — master enable.
    pub enabled: bool,
    /// Section 2 — broadcast a low-power BLE beacon when enabled.
    pub bluetooth_beacon: bool,
    /// Section 3 — slider 10..=300 seconds (default 30).
    pub approval_timeout_seconds: i32,
    /// Section 3 — `"allow"` (better for unconscious) or `"refuse"` (safer
    /// against malicious requests).
    pub default_action_on_timeout: String,
    /// Section 4 — `"full"` (full dialog above lock screen) or
    /// `"basic_only"` (responder name + details hidden until unlock).
    pub lock_screen_visibility: String,
    /// Section 5 — 0 / 3 / 12 / 24 (current vitals always visible
    /// regardless).
    pub history_window_hours: i32,
    /// Section 5 — explicit list of dotted channel paths (e.g.
    /// `"std.blood_glucose.value"`) the emergency profile exposes.
    pub channel_paths_allowed: Vec<String>,
    /// Section 5 — sensitivity classes the emergency profile exposes
    /// (defaults to `["general"]`; users can opt-in mental_health,
    /// substance_use, sexual_health, reproductive).
    pub sensitivity_classes_allowed: Vec<String>,
    /// Section 6 — share GPS to the responding authority on grant.
    pub share_location: bool,
    /// Section 7 — trust roots the patient accepts (per-row label;
    /// PEM/cert handling deferred to UI plumbing).
    pub trusted_authorities: Vec<TrustedAuthority>,
    /// Section 8 — Good Samaritan bystander proxy.
    pub bystander_proxy_enabled: bool,
    /// Last update time (Unix ms).
    pub updated_at_ms: i64,
}

impl Default for EmergencyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bluetooth_beacon: true,
            approval_timeout_seconds: 30,
            default_action_on_timeout: "allow".to_string(),
            lock_screen_visibility: "full".to_string(),
            history_window_hours: 24,
            channel_paths_allowed: Vec::new(),
            sensitivity_classes_allowed: Vec::new(),
            share_location: false,
            trusted_authorities: Vec::new(),
            bystander_proxy_enabled: true,
            updated_at_ms: 0,
        }
    }
}

/// One trust root the patient has installed.
///
/// v1 carries label + an opaque key/cert blob (PEM string). The spec's
/// "Add a trust root" UX paste-and-verify flow lands the verified PEM
/// here; the storage doesn't introspect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustedAuthority {
    /// Display label (e.g. `"OHD Project (default root)"`).
    pub label: String,
    /// Country / scope tag (e.g. `"global"`, `"cz"`).
    pub scope: Option<String>,
    /// PEM-encoded certificate blob (opaque to storage).
    pub public_key_pem: Option<String>,
    /// Whether this root is the project default (non-removable in UI).
    #[serde(default)]
    pub is_default: bool,
}

/// Fetch the user's emergency config. Returns the default config if no row
/// exists.
pub fn get_emergency_config(conn: &Connection, user_ulid: Ulid) -> Result<EmergencyConfig> {
    let row: Option<(
        i64,
        i64,
        i32,
        String,
        String,
        i32,
        String,
        String,
        i64,
        String,
        i64,
        i64,
    )> = conn
        .query_row(
            "SELECT enabled, bluetooth_beacon, approval_timeout_seconds,
                    default_action_on_timeout, lock_screen_visibility,
                    history_window_hours, channel_paths_allowed_json,
                    sensitivity_classes_allowed_json, share_location,
                    trusted_authorities_json, bystander_proxy_enabled,
                    updated_at_ms
               FROM _emergency_config
              WHERE user_ulid = ?1",
            params![user_ulid.to_vec()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                ))
            },
        )
        .optional()?;
    let Some((
        enabled,
        bluetooth_beacon,
        approval_timeout_seconds,
        default_action_on_timeout,
        lock_screen_visibility,
        history_window_hours,
        channel_paths_allowed_json,
        sensitivity_classes_allowed_json,
        share_location,
        trusted_authorities_json,
        bystander_proxy_enabled,
        updated_at_ms,
    )) = row
    else {
        return Ok(EmergencyConfig::default());
    };
    let channel_paths_allowed: Vec<String> =
        serde_json::from_str(&channel_paths_allowed_json).unwrap_or_default();
    let sensitivity_classes_allowed: Vec<String> =
        serde_json::from_str(&sensitivity_classes_allowed_json).unwrap_or_default();
    let trusted_authorities: Vec<TrustedAuthority> =
        serde_json::from_str(&trusted_authorities_json).unwrap_or_default();
    Ok(EmergencyConfig {
        enabled: enabled != 0,
        bluetooth_beacon: bluetooth_beacon != 0,
        approval_timeout_seconds,
        default_action_on_timeout,
        lock_screen_visibility,
        history_window_hours,
        channel_paths_allowed,
        sensitivity_classes_allowed,
        share_location: share_location != 0,
        trusted_authorities,
        bystander_proxy_enabled: bystander_proxy_enabled != 0,
        updated_at_ms,
    })
}

/// Replace the user's emergency config (whole-blob upsert; UI roundtrips
/// the full struct).
pub fn set_emergency_config(
    conn: &Connection,
    user_ulid: Ulid,
    cfg: &EmergencyConfig,
    now_ms: i64,
) -> Result<()> {
    validate(cfg)?;
    let channel_paths_allowed_json = serde_json::to_string(&cfg.channel_paths_allowed)?;
    let sensitivity_classes_allowed_json = serde_json::to_string(&cfg.sensitivity_classes_allowed)?;
    let trusted_authorities_json = serde_json::to_string(&cfg.trusted_authorities)?;
    conn.execute(
        "INSERT INTO _emergency_config
            (user_ulid, enabled, bluetooth_beacon, approval_timeout_seconds,
             default_action_on_timeout, lock_screen_visibility,
             history_window_hours, channel_paths_allowed_json,
             sensitivity_classes_allowed_json, share_location,
             trusted_authorities_json, bystander_proxy_enabled, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(user_ulid) DO UPDATE SET
            enabled                          = excluded.enabled,
            bluetooth_beacon                 = excluded.bluetooth_beacon,
            approval_timeout_seconds         = excluded.approval_timeout_seconds,
            default_action_on_timeout        = excluded.default_action_on_timeout,
            lock_screen_visibility           = excluded.lock_screen_visibility,
            history_window_hours             = excluded.history_window_hours,
            channel_paths_allowed_json       = excluded.channel_paths_allowed_json,
            sensitivity_classes_allowed_json = excluded.sensitivity_classes_allowed_json,
            share_location                   = excluded.share_location,
            trusted_authorities_json         = excluded.trusted_authorities_json,
            bystander_proxy_enabled          = excluded.bystander_proxy_enabled,
            updated_at_ms                    = excluded.updated_at_ms",
        params![
            user_ulid.to_vec(),
            cfg.enabled as i64,
            cfg.bluetooth_beacon as i64,
            cfg.approval_timeout_seconds,
            cfg.default_action_on_timeout,
            cfg.lock_screen_visibility,
            cfg.history_window_hours,
            channel_paths_allowed_json,
            sensitivity_classes_allowed_json,
            cfg.share_location as i64,
            trusted_authorities_json,
            cfg.bystander_proxy_enabled as i64,
            now_ms,
        ],
    )?;
    Ok(())
}

fn validate(cfg: &EmergencyConfig) -> Result<()> {
    if !(10..=300).contains(&cfg.approval_timeout_seconds) {
        return Err(Error::InvalidArgument(format!(
            "approval_timeout_seconds out of range (10..=300): {}",
            cfg.approval_timeout_seconds
        )));
    }
    match cfg.default_action_on_timeout.as_str() {
        "allow" | "refuse" => {}
        other => {
            return Err(Error::InvalidArgument(format!(
                "default_action_on_timeout must be 'allow' or 'refuse'; got {other:?}"
            )))
        }
    }
    match cfg.lock_screen_visibility.as_str() {
        "full" | "basic_only" => {}
        other => {
            return Err(Error::InvalidArgument(format!(
                "lock_screen_visibility must be 'full' or 'basic_only'; got {other:?}"
            )))
        }
    }
    if !matches!(cfg.history_window_hours, 0 | 3 | 12 | 24) {
        return Err(Error::InvalidArgument(format!(
            "history_window_hours must be 0|3|12|24; got {}",
            cfg.history_window_hours
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{open_or_create, DeploymentMode, OpenParams};

    fn open_test_db() -> Connection {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ec.db");
        let (conn, _) = open_or_create(OpenParams {
            path: &path,
            cipher_key: &[],
            create_if_missing: true,
            create_mode: DeploymentMode::Primary,
            create_user_ulid: None,
        })
        .unwrap();
        std::mem::forget(dir);
        conn
    }

    #[test]
    fn defaults_when_unset() {
        let conn = open_test_db();
        let user = [1u8; 16];
        let cfg = get_emergency_config(&conn, user).unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.approval_timeout_seconds, 30);
        assert_eq!(cfg.default_action_on_timeout, "allow");
        assert_eq!(cfg.history_window_hours, 24);
        assert!(cfg.bystander_proxy_enabled);
    }

    #[test]
    fn set_then_get_round_trip() {
        let conn = open_test_db();
        let user = [2u8; 16];
        let cfg = EmergencyConfig {
            enabled: true,
            bluetooth_beacon: true,
            approval_timeout_seconds: 45,
            default_action_on_timeout: "refuse".into(),
            lock_screen_visibility: "basic_only".into(),
            history_window_hours: 12,
            channel_paths_allowed: vec!["std.blood_glucose.value".into()],
            sensitivity_classes_allowed: vec!["general".into(), "mental_health".into()],
            share_location: true,
            trusted_authorities: vec![TrustedAuthority {
                label: "OHD Project".into(),
                scope: Some("global".into()),
                public_key_pem: None,
                is_default: true,
            }],
            bystander_proxy_enabled: false,
            updated_at_ms: 0,
        };
        set_emergency_config(&conn, user, &cfg, 1_700_000_000_000).unwrap();
        let got = get_emergency_config(&conn, user).unwrap();
        assert!(got.enabled);
        assert_eq!(got.approval_timeout_seconds, 45);
        assert_eq!(got.default_action_on_timeout, "refuse");
        assert_eq!(got.lock_screen_visibility, "basic_only");
        assert_eq!(got.history_window_hours, 12);
        assert_eq!(got.channel_paths_allowed, vec!["std.blood_glucose.value"]);
        assert_eq!(
            got.sensitivity_classes_allowed,
            vec!["general", "mental_health"]
        );
        assert!(got.share_location);
        assert_eq!(got.trusted_authorities.len(), 1);
        assert_eq!(got.trusted_authorities[0].label, "OHD Project");
        assert!(!got.bystander_proxy_enabled);
        assert_eq!(got.updated_at_ms, 1_700_000_000_000);
    }

    #[test]
    fn rejects_out_of_range_timeout() {
        let conn = open_test_db();
        let user = [3u8; 16];
        let mut cfg = EmergencyConfig::default();
        cfg.approval_timeout_seconds = 5;
        let err = set_emergency_config(&conn, user, &cfg, 0).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)));
        cfg.approval_timeout_seconds = 600;
        let err = set_emergency_config(&conn, user, &cfg, 0).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)));
    }

    #[test]
    fn rejects_invalid_history_window() {
        let conn = open_test_db();
        let user = [4u8; 16];
        let mut cfg = EmergencyConfig::default();
        cfg.history_window_hours = 5;
        let err = set_emergency_config(&conn, user, &cfg, 0).unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)));
    }
}
