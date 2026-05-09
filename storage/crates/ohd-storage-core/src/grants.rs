//! Grants — full CRUD + scope materialization.
//!
//! Backs `grants`, `grant_event_type_rules`, `grant_channel_rules`,
//! `grant_sensitivity_rules`, `grant_write_event_type_rules`,
//! `grant_auto_approve_event_types`, `grant_time_windows`, `grant_cases`. The
//! resolution-algorithm precedence ladder (sensitivity-deny > channel-deny >
//! type-deny > sensitivity-allow > channel-allow > type-allow > default)
//! lives partly here (rule-row materialization) and partly in
//! [`crate::events::GrantScope`].

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::encryption::{self, EnvelopeKey, RecoveryKeypair, NONCE_LEN, RECOVERY_KEY_LEN};
use crate::registry;
use crate::ulid::{self, Ulid};
use crate::{Error, Result};

/// One sensitivity-class DEK wrapped for delivery to a grantee.
///
/// When a grant scope includes encrypted classes (`mental_health`,
/// `sexual_health`, `substance_use`, `reproductive`), the per-class DEK is
/// re-wrapped under a key the grantee can unwrap. v1 only handles the
/// **single-storage** case: the grant lives in the same SQLCipher file as
/// the data being granted, so the grantee's `K_envelope` is the same
/// `K_envelope` we already have. For the **multi-storage** case (grantee
/// runs their own storage daemon), the wrap is keyed to the grantee's public
/// key — that flow is documented as v0.x in STATUS.md.
///
/// See `spec/encryption.md` "End-to-end channel encryption (deferred)" →
/// "Per-grant key sharing".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassKeyWrap {
    /// 12-byte AES-GCM nonce used to wrap the DEK.
    pub nonce: [u8; NONCE_LEN],
    /// AES-GCM ciphertext + 16-byte tag (32 + 16 = 48 bytes for a 32-byte
    /// DEK).
    pub ciphertext: Vec<u8>,
    /// `class_key_history.id` of the DEK that was wrapped — pins the
    /// grantee's view to a specific generation in case of rotation.
    pub key_id: i64,
}

/// CBOR-encoded map `{sensitivity_class -> ClassKeyWrap}` stored in
/// `grants.class_key_wraps`.
pub type ClassKeyWraps = BTreeMap<String, ClassKeyWrap>;

/// Grantee-side unwrap of a multi-storage grant's `ClassKeyWrap` entry.
///
/// Mirrors the issuer-side wrap from
/// [`encryption::wrap_class_key_for_grantee`]: ECDH(grantee_seckey,
/// issuer_pubkey) → HKDF-SHA256 → wrap KEK → AES-256-GCM unwrap.
///
/// Codex review #9: the unwrap binds `(grant_ulid, sensitivity_class,
/// class_key_history_id)` into the AEAD AAD and `(class, issuer_pk,
/// grantee_pk)` into the HKDF info. Replay across grants by the same
/// (issuer, grantee) pair fails the AEAD verify.
///
/// `grant_row` is the grant carrying the wrap material (including
/// `issuer_recovery_pubkey`, populated by the issuer's storage). The grantee's
/// `RecoveryKeypair` comes from `Storage::recovery_keypair()` on the grantee's
/// side.
///
/// Returns the unwrapped `ClassKey` ready to feed into
/// `channel_encryption::decrypt_channel_value`.
pub fn unwrap_class_key_for_grantee(
    grantee_recovery: &RecoveryKeypair,
    grant_row: &GrantRow,
    sensitivity_class: &str,
) -> Result<encryption::ClassKey> {
    let issuer_pk = grant_row.issuer_recovery_pubkey.as_ref().ok_or_else(|| {
        Error::InvalidArgument(
            "unwrap_class_key_for_grantee: grant has no issuer_recovery_pubkey \
                 (single-storage grant — use envelope-key unwrap instead)"
                .into(),
        )
    })?;
    let wrap = grant_row
        .class_key_wraps
        .get(sensitivity_class)
        .ok_or_else(|| Error::NotFound)?;
    let wrapped = encryption::WrappedClassKey {
        nonce: wrap.nonce,
        ciphertext: wrap.ciphertext.clone(),
    };
    encryption::unwrap_class_key_from_issuer(
        grantee_recovery,
        issuer_pk,
        sensitivity_class,
        &wrapped,
        &grant_row.ulid,
        wrap.key_id,
    )
}

/// Encode a wraps map to bytes for the `grants.class_key_wraps` BLOB.
fn encode_class_key_wraps(map: &ClassKeyWraps) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(64 + 64 * map.len());
    ciborium::ser::into_writer(map, &mut buf)
        .map_err(|e| Error::Internal(anyhow::anyhow!("CBOR encode class_key_wraps: {e}")))?;
    Ok(buf)
}

/// Decode a wraps map from the `grants.class_key_wraps` BLOB.
fn decode_class_key_wraps(bytes: &[u8]) -> Result<ClassKeyWraps> {
    ciborium::de::from_reader(bytes)
        .map_err(|e| Error::Internal(anyhow::anyhow!("CBOR decode class_key_wraps: {e}")))
}

/// Re-wrap each currently-active per-class DEK for storage on a grant row.
///
/// For each `sensitivity_class` in [`encryption::DEFAULT_ENCRYPTED_CLASSES`]
/// that the grant's scope allows reads of, this function:
///
/// 1. Loads the active `K_class` (unwrapping under `envelope_key`).
/// 2. **Multi-storage path** (when `grantee_recovery_pubkey` is set):
///    ECDH(issuer_seckey, grantee_pubkey) → HKDF-SHA256 → wrap KEK; AES-256-
///    GCM-encrypt K_class under the KEK with AAD = sensitivity class. The
///    grantee's storage performs the mirror ECDH against the issuer's
///    pubkey (`grants.issuer_recovery_pubkey`) to unwrap.
/// 3. **Single-storage path** (when `grantee_recovery_pubkey` is `None`):
///    re-wrap the DEK under the same `envelope_key`. Backwards-compatible
///    for grantees who open the same storage file (or a sync'd cache) and
///    have the same K_envelope.
/// 4. Builds a [`ClassKeyWrap`] keyed by sensitivity class.
///
/// The "scope allows reads of" check is conservative: any of the four
/// scope-resolution channels (event-type rules, channel rules, sensitivity
/// rules, default_action) that allows the class triggers a wrap. Encrypted
/// classes the grant explicitly denies (`grant_sensitivity_rules` deny) are
/// skipped — those rows wouldn't be returned anyway.
pub fn build_class_key_wraps_for_grant(
    conn: &Connection,
    envelope_key: &EnvelopeKey,
    issuer_recovery: Option<&RecoveryKeypair>,
    new_grant: &NewGrant,
    grant_ulid: &Ulid,
) -> Result<ClassKeyWraps> {
    let mut out: ClassKeyWraps = BTreeMap::new();
    for class in encryption::DEFAULT_ENCRYPTED_CLASSES {
        // Skip if the scope explicitly denies this class.
        let denied = new_grant
            .sensitivity_rules
            .iter()
            .any(|(c, eff)| c == *class && *eff == RuleEffect::Deny);
        if denied {
            continue;
        }
        // Otherwise, conservatively include the wrap. The grant's runtime
        // resolver still gates which rows are returned — including a wrap
        // for a class the grant doesn't actively need is harmless.
        let active = match encryption::load_active_class_key(conn, envelope_key, class) {
            Ok(a) => a,
            // No K_class for this class yet (e.g. bootstrap hasn't run for a
            // newly-added class) — skip; the next bootstrap will mint one.
            Err(Error::NotFound) => continue,
            Err(e) => return Err(e),
        };
        let wrapped = match (new_grant.grantee_recovery_pubkey.as_ref(), issuer_recovery) {
            (Some(grantee_pk), Some(issuer)) => {
                // Codex review #9: wrap binds (grant_ulid, class, key_id) into
                // AAD and (class, issuer_pk, grantee_pk) into HKDF info.
                encryption::wrap_class_key_for_grantee(
                    issuer,
                    grantee_pk,
                    class,
                    &active.key,
                    grant_ulid,
                    active.key_id,
                )?
            }
            // Backwards-compat: no grantee pubkey supplied → re-wrap under
            // the issuer's K_envelope (single-storage case).
            _ => encryption::wrap_class_key(envelope_key, class, &active.key)?,
        };
        out.insert(
            (*class).to_string(),
            ClassKeyWrap {
                nonce: wrapped.nonce,
                ciphertext: wrapped.ciphertext,
                key_id: active.key_id,
            },
        );
    }
    Ok(out)
}

/// Effect: `allow` or `deny`. Stored as the literal string in `grant_*_rules.effect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuleEffect {
    /// Allow events / channels matching this rule.
    Allow,
    /// Deny events / channels matching this rule. Deny wins on conflict.
    /// Default — closed-by-default access matches the spec's allowlist preset.
    #[default]
    Deny,
}

impl RuleEffect {
    /// On-disk string form.
    pub fn as_str(self) -> &'static str {
        match self {
            RuleEffect::Allow => "allow",
            RuleEffect::Deny => "deny",
        }
    }

    /// Parse the on-disk string form. Anything other than `"allow"` is
    /// treated as `deny` (defensive default).
    pub fn parse(s: &str) -> Self {
        match s {
            "allow" => RuleEffect::Allow,
            _ => RuleEffect::Deny,
        }
    }
}

/// Sparse builder used by [`create_grant`]. Self-session-only callers must
/// call this; grant tokens cannot create grants per
/// `spec/privacy-access.md` "Grants don't chain".
#[derive(Debug, Clone, Default)]
pub struct NewGrant {
    /// Display label for the grantee.
    pub grantee_label: String,
    /// `human` / `app` / `service` / `emergency` / `device` / `delegate`.
    pub grantee_kind: String,
    /// For `grantee_kind="delegate"`: the user ULID being delegated *for*
    /// (parent → child, caregiver → elderly parent). The grant token's
    /// bearer is the *delegate*; reads return the delegated-for user's
    /// data. NULL on every non-delegate grant.
    ///
    /// Authority is **scoped**, not unrestricted: the delegate sees only
    /// what the grant's per-event-type / per-channel / per-sensitivity
    /// rules allow. Channels / event types the user wants to keep
    /// "self-only" are denied via normal `grant_*_rules`.
    pub delegate_for_user_ulid: Option<Ulid>,
    /// Free-text purpose.
    pub purpose: Option<String>,
    /// Default action for events not matching any explicit rule.
    pub default_action: RuleEffect,
    /// Approval mode: `always` / `auto_for_event_types` / `never_required`.
    pub approval_mode: String,
    /// Optional hard expiry, Unix ms.
    pub expires_at_ms: Option<i64>,
    /// Per-event-type read rules (allow/deny) — by dotted name.
    pub event_type_rules: Vec<(String, RuleEffect)>,
    /// Per-channel read rules.
    pub channel_rules: Vec<ChannelRuleSpec>,
    /// Per-sensitivity-class rules.
    pub sensitivity_rules: Vec<(String, RuleEffect)>,
    /// Per-event-type write rules.
    pub write_event_type_rules: Vec<(String, RuleEffect)>,
    /// Event types that auto-approve under `approval_mode='auto_for_event_types'`.
    pub auto_approve_event_types: Vec<String>,
    /// Aggregation-only flag.
    pub aggregation_only: bool,
    /// Strip `events.notes` on returned rows.
    pub strip_notes: bool,
    /// Push notification on every read.
    pub notify_on_access: bool,
    /// Per-query approval (extreme privacy).
    pub require_approval_per_query: bool,
    /// Token-bucket rate limit per day.
    pub max_queries_per_day: Option<i32>,
    /// Token-bucket rate limit per hour.
    pub max_queries_per_hour: Option<i32>,
    /// Rolling N-day visibility window (e.g. last 30 days).
    pub rolling_window_days: Option<i32>,
    /// Absolute time window `[from_ms, to_ms]`.
    pub absolute_window: Option<(i64, i64)>,
    /// Optional 32-byte X25519 recovery pubkey of the grantee's storage. When
    /// supplied, encrypted-class K_class wraps target the grantee's pubkey
    /// via ECDH(issuer_seckey, grantee_pubkey) → HKDF → AES-GCM. When `None`,
    /// the existing single-storage path (re-wrap under issuer's K_envelope)
    /// is used — backwards-compatible for grantees who run on the same
    /// storage as the issuer.
    pub grantee_recovery_pubkey: Option<[u8; RECOVERY_KEY_LEN]>,
}

/// One row spec for `grant_channel_rules`.
#[derive(Debug, Clone)]
pub struct ChannelRuleSpec {
    /// Dotted event-type name, e.g. `"std.blood_glucose"`.
    pub event_type: String,
    /// Channel path within that type, e.g. `"value"` or `"nutrition.fat"`.
    pub channel_path: String,
    /// Allow or deny.
    pub effect: RuleEffect,
}

/// Full grant record materialized for the wire.
#[derive(Debug, Clone)]
pub struct GrantRow {
    /// Internal rowid.
    pub id: i64,
    /// For delegate grants, the user being delegated for.
    pub delegate_for_user_ulid: Option<Ulid>,
    /// Wire ULID (16 bytes, `(created_at_ms, ulid_random)`).
    pub ulid: Ulid,
    /// Grantee display label.
    pub grantee_label: String,
    /// Grantee kind (`human` / `app` / …).
    pub grantee_kind: String,
    /// Optional grantee identity ULID.
    pub grantee_ulid: Option<Ulid>,
    /// Free-text purpose.
    pub purpose: Option<String>,
    /// Creation timestamp.
    pub created_at_ms: i64,
    /// Hard-expiry.
    pub expires_at_ms: Option<i64>,
    /// Revocation timestamp.
    pub revoked_at_ms: Option<i64>,
    /// `"allow"` or `"deny"`.
    pub default_action: String,
    /// Aggregation-only.
    pub aggregation_only: bool,
    /// Strip notes on returned rows.
    pub strip_notes: bool,
    /// Per-query approval required.
    pub require_approval_per_query: bool,
    /// Approval mode.
    pub approval_mode: String,
    /// Notify on each access.
    pub notify_on_access: bool,
    /// Per-day rate limit.
    pub max_queries_per_day: Option<i32>,
    /// Per-hour rate limit.
    pub max_queries_per_hour: Option<i32>,
    /// Rolling window in days.
    pub rolling_window_days: Option<i32>,
    /// Absolute time window `[from_ms, to_ms]`.
    pub absolute_window: Option<(i64, i64)>,
    /// Read rules per event type.
    pub event_type_rules: Vec<(String, RuleEffect)>,
    /// Read rules per channel.
    pub channel_rules: Vec<ChannelRuleSpec>,
    /// Read rules per sensitivity class.
    pub sensitivity_rules: Vec<(String, RuleEffect)>,
    /// Write rules per event type.
    pub write_event_type_rules: Vec<(String, RuleEffect)>,
    /// Auto-approve event-type allowlist.
    pub auto_approve_event_types: Vec<String>,
    /// Per-sensitivity-class wrap material for value-level encryption (see
    /// [`ClassKeyWrap`]). Empty when the grant predates v1.x channel
    /// encryption or doesn't touch any encrypted class.
    pub class_key_wraps: ClassKeyWraps,
    /// Grantee's published X25519 recovery pubkey, when the grant was
    /// re-targeted via ECDH (multi-storage case). `None` for single-storage
    /// grants (the wrap is under the issuer's K_envelope).
    pub grantee_recovery_pubkey: Option<[u8; RECOVERY_KEY_LEN]>,
    /// Issuer's published X25519 recovery pubkey at issue time. Same value
    /// regardless of grantee for any given storage (deterministic from
    /// K_file). The grantee's daemon ECDHs against this pubkey to unwrap.
    pub issuer_recovery_pubkey: Option<[u8; RECOVERY_KEY_LEN]>,
}

/// Insert a grant row + its rule rows. Returns `(grant_id, grant_ulid)`.
///
/// Backwards-compatible wrapper that does NOT populate
/// `grants.class_key_wraps`. Callers that have an envelope key (i.e. all
/// production callers — see [`crate::storage::Storage::envelope_key`])
/// should prefer [`create_grant_with_envelope`] so a clinician grant with
/// access to encrypted classes carries the unwrap material.
pub fn create_grant(conn: &mut Connection, g: &NewGrant) -> Result<(i64, Ulid)> {
    create_grant_inner(conn, g, None, None)
}

/// Same as [`create_grant`] but additionally re-wraps the active per-class
/// DEKs for the grantee. The wraps are stored as a CBOR map in the
/// `grants.class_key_wraps` BLOB.
///
/// When `issuer_recovery` is supplied AND `g.grantee_recovery_pubkey` is set,
/// the wraps target the grantee's pubkey via X25519 ECDH (multi-storage
/// path). Otherwise the wraps re-target the issuer's `envelope_key`
/// (single-storage path; backwards-compatible).
pub fn create_grant_with_envelope(
    conn: &mut Connection,
    g: &NewGrant,
    envelope_key: &EnvelopeKey,
    issuer_recovery: Option<&RecoveryKeypair>,
) -> Result<(i64, Ulid)> {
    create_grant_inner(conn, g, Some(envelope_key), issuer_recovery)
}

fn create_grant_inner(
    conn: &mut Connection,
    g: &NewGrant,
    envelope_key: Option<&EnvelopeKey>,
    issuer_recovery: Option<&RecoveryKeypair>,
) -> Result<(i64, Ulid)> {
    validate_default_action(g.default_action)?;
    validate_approval_mode(&g.approval_mode)?;
    let now = crate::format::now_ms();
    let new_ulid = ulid::mint(now);
    let rand_tail = ulid::random_tail(&new_ulid);
    // delegate validation: grantee_kind="delegate" requires
    // delegate_for_user_ulid; non-delegate kinds must not set it.
    if g.grantee_kind == "delegate" && g.delegate_for_user_ulid.is_none() {
        return Err(Error::InvalidArgument(
            "grantee_kind='delegate' requires delegate_for_user_ulid".into(),
        ));
    }
    if g.grantee_kind != "delegate" && g.delegate_for_user_ulid.is_some() {
        return Err(Error::InvalidArgument(
            "delegate_for_user_ulid only valid for grantee_kind='delegate'".into(),
        ));
    }
    // Build wrap material before opening the write transaction so the read
    // side of `load_active_class_key` doesn't deadlock with the write side.
    // Codex review #9: pass `new_ulid` so the wrap's AAD binds the grant.
    let class_key_wraps_blob: Option<Vec<u8>> = match envelope_key {
        Some(env) => {
            let map = build_class_key_wraps_for_grant(conn, env, issuer_recovery, g, &new_ulid)?;
            if map.is_empty() {
                None
            } else {
                Some(encode_class_key_wraps(&map)?)
            }
        }
        None => None,
    };
    // Issuer pubkey is published whenever we have a recovery keypair, so the
    // grantee can ECDH against it. Single-storage grants get NULL here; that
    // also signals "wraps are under K_envelope, not ECDH-targeted".
    let issuer_pubkey_col: Option<Vec<u8>> =
        match (issuer_recovery, g.grantee_recovery_pubkey.as_ref()) {
            (Some(kp), Some(_)) => Some(kp.public_bytes().to_vec()),
            _ => None,
        };
    let grantee_pubkey_col: Option<Vec<u8>> = g.grantee_recovery_pubkey.map(|p| p.to_vec());
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO grants
            (ulid_random, grantee_label, grantee_kind, grantee_ulid, is_template,
             created_at_ms, expires_at_ms, purpose, default_action,
             aggregation_only, strip_notes, require_approval_per_query,
             approval_mode, notify_on_access, max_queries_per_day,
             max_queries_per_hour, rolling_window_days, delegate_for_user_ulid,
             class_key_wraps, grantee_recovery_pubkey, issuer_recovery_pubkey)
         VALUES (?1, ?2, ?3, NULL, 0, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            rand_tail.to_vec(),
            g.grantee_label,
            g.grantee_kind,
            now,
            g.expires_at_ms,
            g.purpose,
            g.default_action.as_str(),
            g.aggregation_only as i64,
            g.strip_notes as i64,
            g.require_approval_per_query as i64,
            g.approval_mode,
            g.notify_on_access as i64,
            g.max_queries_per_day,
            g.max_queries_per_hour,
            g.rolling_window_days,
            g.delegate_for_user_ulid.map(|u| u.to_vec()),
            class_key_wraps_blob,
            grantee_pubkey_col,
            issuer_pubkey_col,
        ],
    )?;
    let grant_id = tx.last_insert_rowid();

    write_event_type_rules_inner(&tx, grant_id, &g.event_type_rules, "grant_event_type_rules")?;
    write_channel_rules_inner(&tx, grant_id, &g.channel_rules)?;
    write_sensitivity_rules_inner(&tx, grant_id, &g.sensitivity_rules)?;
    write_event_type_rules_inner(
        &tx,
        grant_id,
        &g.write_event_type_rules,
        "grant_write_event_type_rules",
    )?;
    write_auto_approve_inner(&tx, grant_id, &g.auto_approve_event_types)?;
    if let Some((from_ms, to_ms)) = g.absolute_window {
        tx.execute(
            "INSERT OR REPLACE INTO grant_time_windows (grant_id, from_ms, to_ms)
             VALUES (?1, ?2, ?3)",
            params![grant_id, from_ms, to_ms],
        )?;
    }
    tx.commit()?;
    Ok((grant_id, new_ulid))
}

/// Mark a grant as revoked (synchronous). See
/// `spec/privacy-access.md` "Revocation semantics".
pub fn revoke_grant(conn: &Connection, grant_id: i64, reason: Option<&str>) -> Result<i64> {
    let now = crate::format::now_ms();
    conn.execute(
        "UPDATE grants SET revoked_at_ms = ?1 WHERE id = ?2 AND revoked_at_ms IS NULL",
        params![now, grant_id],
    )?;
    crate::audit::append(
        conn,
        &crate::audit::AuditEntry {
            ts_ms: now,
            actor_type: crate::audit::ActorType::Self_,
            auto_granted: false,
            grant_id: Some(grant_id),
            action: "grant_revoke".into(),
            query_kind: None,
            query_params_json: None,
            rows_returned: None,
            rows_filtered: None,
            result: crate::audit::AuditResult::Success,
            reason: reason.map(str::to_string),
            caller_ip: None,
            caller_ua: None,
            delegated_for_user_ulid: None,
        },
    )?;
    Ok(now)
}

/// Sparse update for an existing grant. Mirrors the proto contract: only
/// `grantee_label` and `expires_at_ms` are mutable in v1; the proto reserves
/// room for richer updates in v1.x.
#[derive(Debug, Clone, Default)]
pub struct GrantUpdate {
    /// New display label.
    pub grantee_label: Option<String>,
    /// New hard expiry (None leaves it unchanged; pass Some(0) to clear).
    pub expires_at_ms: Option<i64>,
}

/// Apply an [`GrantUpdate`]. Returns the updated row.
pub fn update_grant(
    conn: &mut Connection,
    grant_id: i64,
    update: &GrantUpdate,
) -> Result<GrantRow> {
    let tx = conn.transaction()?;
    let exists: Option<i64> = tx
        .query_row(
            "SELECT id FROM grants WHERE id = ?1 AND revoked_at_ms IS NULL",
            params![grant_id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(Error::NotFound);
    }
    if let Some(label) = &update.grantee_label {
        tx.execute(
            "UPDATE grants SET grantee_label = ?1 WHERE id = ?2",
            params![label, grant_id],
        )?;
    }
    if let Some(expires) = update.expires_at_ms {
        tx.execute(
            "UPDATE grants SET expires_at_ms = ?1 WHERE id = ?2",
            params![expires, grant_id],
        )?;
    }
    crate::audit::append(
        &tx,
        &crate::audit::AuditEntry {
            ts_ms: crate::format::now_ms(),
            actor_type: crate::audit::ActorType::Self_,
            auto_granted: false,
            grant_id: Some(grant_id),
            action: "grant_update".into(),
            query_kind: Some("update_grant".into()),
            query_params_json: Some(serde_json::to_string(&serde_json::json!({
                "grantee_label": update.grantee_label,
                "expires_at_ms": update.expires_at_ms,
            }))?),
            rows_returned: None,
            rows_filtered: None,
            result: crate::audit::AuditResult::Success,
            reason: None,
            caller_ip: None,
            caller_ua: None,
            delegated_for_user_ulid: None,
        },
    )?;
    tx.commit()?;
    let row = read_grant(conn, grant_id)?;
    Ok(row)
}

/// Filter for [`list_grants`].
#[derive(Debug, Clone, Default)]
pub struct ListGrantsFilter {
    /// Include `revoked_at_ms IS NOT NULL`.
    pub include_revoked: bool,
    /// Include `expires_at_ms <= now`.
    pub include_expired: bool,
    /// Filter by grantee_kind exact match.
    pub grantee_kind: Option<String>,
    /// Filter to a single grant rowid (used by grant-token introspection).
    pub only_grant_id: Option<i64>,
    /// Page size.
    pub limit: Option<i64>,
}

/// List grants. Self-session callers see all rows; grant-token holders pass
/// `only_grant_id` so they only see their own grant.
pub fn list_grants(conn: &Connection, filter: &ListGrantsFilter) -> Result<Vec<GrantRow>> {
    let now = crate::format::now_ms();
    let mut sql = String::from("SELECT id FROM grants WHERE is_template = 0");
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if !filter.include_revoked {
        sql.push_str(" AND revoked_at_ms IS NULL");
    }
    if !filter.include_expired {
        sql.push_str(" AND (expires_at_ms IS NULL OR expires_at_ms > ?)");
        args.push(now.into());
    }
    if let Some(ref kind) = filter.grantee_kind {
        sql.push_str(" AND grantee_kind = ?");
        args.push(kind.clone().into());
    }
    if let Some(gid) = filter.only_grant_id {
        sql.push_str(" AND id = ?");
        args.push(gid.into());
    }
    sql.push_str(" ORDER BY created_at_ms DESC");
    let limit = filter.limit.unwrap_or(100).clamp(1, 1000);
    sql.push_str(&format!(" LIMIT {limit}"));

    let mut stmt = conn.prepare(&sql)?;
    let ids: Vec<i64> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(read_grant(conn, id)?);
    }
    Ok(out)
}

/// Read one grant fully (including all rule tables) by rowid.
pub fn read_grant(conn: &Connection, grant_id: i64) -> Result<GrantRow> {
    type ReadGrantRow = (
        Vec<u8>,
        String,
        String,
        Option<Vec<u8>>,
        Option<String>,
        i64,
        Option<i64>,
        Option<i64>,
        String,
        i64,
        i64,
        i64,
        String,
        i64,
        Option<i32>,
        Option<i32>,
        Option<i32>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
    );
    let row: Option<ReadGrantRow> = conn
        .query_row(
            "SELECT ulid_random, grantee_label, grantee_kind, grantee_ulid, purpose,
                    created_at_ms, expires_at_ms, revoked_at_ms, default_action,
                    aggregation_only, strip_notes, require_approval_per_query,
                    approval_mode, notify_on_access, max_queries_per_day,
                    max_queries_per_hour, rolling_window_days, delegate_for_user_ulid,
                    class_key_wraps, grantee_recovery_pubkey, issuer_recovery_pubkey
               FROM grants WHERE id = ?1",
            params![grant_id],
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
                    r.get(12)?,
                    r.get(13)?,
                    r.get(14)?,
                    r.get(15)?,
                    r.get(16)?,
                    r.get(17)?,
                    r.get(18)?,
                    r.get(19)?,
                    r.get(20)?,
                ))
            },
        )
        .optional()?;
    let Some((
        rand_tail,
        grantee_label,
        grantee_kind,
        grantee_ulid_blob,
        purpose,
        created_at_ms,
        expires_at_ms,
        revoked_at_ms,
        default_action,
        aggregation_only,
        strip_notes,
        require_approval_per_query,
        approval_mode,
        notify_on_access,
        max_queries_per_day,
        max_queries_per_hour,
        rolling_window_days,
        delegate_for_user_blob,
        class_key_wraps_blob,
        grantee_recovery_pubkey_blob,
        issuer_recovery_pubkey_blob,
    )) = row
    else {
        return Err(Error::NotFound);
    };

    fn parse_pubkey(b: Option<Vec<u8>>) -> Option<[u8; RECOVERY_KEY_LEN]> {
        b.and_then(|v| {
            if v.len() == RECOVERY_KEY_LEN {
                let mut out = [0u8; RECOVERY_KEY_LEN];
                out.copy_from_slice(&v);
                Some(out)
            } else {
                None
            }
        })
    }
    let grantee_recovery_pubkey = parse_pubkey(grantee_recovery_pubkey_blob);
    let issuer_recovery_pubkey = parse_pubkey(issuer_recovery_pubkey_blob);

    // Stitch the wire ULID from `(created_at_ms, rand_tail)`.
    let mut ulid_buf = [0u8; 16];
    if rand_tail.len() == 10 {
        let prefix = ulid::mint(created_at_ms);
        ulid_buf[..6].copy_from_slice(&prefix[..6]);
        ulid_buf[6..].copy_from_slice(&rand_tail);
    }
    let grantee_ulid = grantee_ulid_blob.and_then(|b| {
        let mut out = [0u8; 16];
        if b.len() == 16 {
            out.copy_from_slice(&b);
            Some(out)
        } else {
            None
        }
    });

    let event_type_rules = read_event_type_rules(conn, grant_id, "grant_event_type_rules")?;
    let channel_rules = read_channel_rules(conn, grant_id)?;
    let sensitivity_rules = read_sensitivity_rules(conn, grant_id)?;
    let write_event_type_rules =
        read_event_type_rules(conn, grant_id, "grant_write_event_type_rules")?;
    let auto_approve_event_types = read_auto_approve(conn, grant_id)?;
    let absolute_window: Option<(i64, i64)> = conn
        .query_row(
            "SELECT from_ms, to_ms FROM grant_time_windows WHERE grant_id = ?1",
            params![grant_id],
            |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .optional()?
        .and_then(|(f, t)| match (f, t) {
            (Some(f), Some(t)) => Some((f, t)),
            _ => None,
        });

    let delegate_for_user_ulid = delegate_for_user_blob.and_then(|b| {
        if b.len() == 16 {
            let mut out = [0u8; 16];
            out.copy_from_slice(&b);
            Some(out)
        } else {
            None
        }
    });

    let class_key_wraps = match class_key_wraps_blob {
        Some(bytes) if !bytes.is_empty() => decode_class_key_wraps(&bytes).unwrap_or_default(),
        _ => ClassKeyWraps::new(),
    };

    Ok(GrantRow {
        id: grant_id,
        ulid: ulid_buf,
        grantee_label,
        grantee_kind,
        grantee_ulid,
        delegate_for_user_ulid,
        purpose,
        created_at_ms,
        expires_at_ms,
        revoked_at_ms,
        default_action,
        aggregation_only: aggregation_only != 0,
        strip_notes: strip_notes != 0,
        require_approval_per_query: require_approval_per_query != 0,
        approval_mode,
        notify_on_access: notify_on_access != 0,
        max_queries_per_day,
        max_queries_per_hour,
        rolling_window_days,
        absolute_window,
        event_type_rules,
        channel_rules,
        sensitivity_rules,
        write_event_type_rules,
        auto_approve_event_types,
        class_key_wraps,
        grantee_recovery_pubkey,
        issuer_recovery_pubkey,
    })
}

/// Look up a grant rowid by its wire ULID.
pub fn grant_id_by_ulid(conn: &Connection, grant_ulid: &Ulid) -> Result<i64> {
    let rand_tail = ulid::random_tail(grant_ulid);
    conn.query_row(
        "SELECT id FROM grants WHERE ulid_random = ?1",
        params![rand_tail.to_vec()],
        |r| r.get::<_, i64>(0),
    )
    .optional()?
    .ok_or(Error::NotFound)
}

// ---- internal writers --------------------------------------------------

fn write_event_type_rules_inner(
    tx: &rusqlite::Transaction<'_>,
    grant_id: i64,
    rules: &[(String, RuleEffect)],
    table: &str,
) -> Result<()> {
    let sql = format!(
        "INSERT OR REPLACE INTO {table} (grant_id, event_type_id, effect)
         VALUES (?1, ?2, ?3)"
    );
    for (et_name, effect) in rules {
        let etn = registry::EventTypeName::parse(et_name)?;
        let et = registry::resolve_event_type(tx, &etn)?;
        tx.execute(&sql, params![grant_id, et.id, effect.as_str()])?;
    }
    Ok(())
}

fn write_channel_rules_inner(
    tx: &rusqlite::Transaction<'_>,
    grant_id: i64,
    rules: &[ChannelRuleSpec],
) -> Result<()> {
    for spec in rules {
        let etn = registry::EventTypeName::parse(&spec.event_type)?;
        let et = registry::resolve_event_type(tx, &etn)?;
        let chan = registry::resolve_channel(tx, et.id, &spec.channel_path)?;
        tx.execute(
            "INSERT OR REPLACE INTO grant_channel_rules (grant_id, channel_id, effect)
             VALUES (?1, ?2, ?3)",
            params![grant_id, chan.id, spec.effect.as_str()],
        )?;
    }
    Ok(())
}

fn write_sensitivity_rules_inner(
    tx: &rusqlite::Transaction<'_>,
    grant_id: i64,
    rules: &[(String, RuleEffect)],
) -> Result<()> {
    for (cls, effect) in rules {
        tx.execute(
            "INSERT OR REPLACE INTO grant_sensitivity_rules (grant_id, sensitivity_class, effect)
             VALUES (?1, ?2, ?3)",
            params![grant_id, cls, effect.as_str()],
        )?;
    }
    Ok(())
}

fn write_auto_approve_inner(
    tx: &rusqlite::Transaction<'_>,
    grant_id: i64,
    types: &[String],
) -> Result<()> {
    for et_name in types {
        let etn = registry::EventTypeName::parse(et_name)?;
        let et = registry::resolve_event_type(tx, &etn)?;
        tx.execute(
            "INSERT OR IGNORE INTO grant_auto_approve_event_types (grant_id, event_type_id)
             VALUES (?1, ?2)",
            params![grant_id, et.id],
        )?;
    }
    Ok(())
}

// ---- internal readers --------------------------------------------------

fn read_event_type_rules(
    conn: &Connection,
    grant_id: i64,
    table: &str,
) -> Result<Vec<(String, RuleEffect)>> {
    let sql = format!(
        "SELECT et.namespace, et.name, r.effect
           FROM {table} r
           JOIN event_types et ON et.id = r.event_type_id
          WHERE r.grant_id = ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![grant_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|(ns, n, eff)| (format!("{ns}.{n}"), RuleEffect::parse(&eff)))
        .collect())
}

fn read_channel_rules(conn: &Connection, grant_id: i64) -> Result<Vec<ChannelRuleSpec>> {
    let mut stmt = conn.prepare(
        "SELECT et.namespace, et.name, c.path, r.effect
           FROM grant_channel_rules r
           JOIN channels c ON c.id = r.channel_id
           JOIN event_types et ON et.id = c.event_type_id
          WHERE r.grant_id = ?1",
    )?;
    let rows = stmt
        .query_map(params![grant_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|(ns, n, path, eff)| ChannelRuleSpec {
            event_type: format!("{ns}.{n}"),
            channel_path: path,
            effect: RuleEffect::parse(&eff),
        })
        .collect())
}

fn read_sensitivity_rules(conn: &Connection, grant_id: i64) -> Result<Vec<(String, RuleEffect)>> {
    let mut stmt = conn.prepare(
        "SELECT sensitivity_class, effect FROM grant_sensitivity_rules WHERE grant_id = ?1",
    )?;
    let rows = stmt
        .query_map(params![grant_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|(c, e)| (c, RuleEffect::parse(&e)))
        .collect())
}

fn read_auto_approve(conn: &Connection, grant_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT et.namespace, et.name
           FROM grant_auto_approve_event_types a
           JOIN event_types et ON et.id = a.event_type_id
          WHERE a.grant_id = ?1",
    )?;
    let rows = stmt
        .query_map(params![grant_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|(ns, n)| format!("{ns}.{n}"))
        .collect())
}

fn validate_default_action(action: RuleEffect) -> Result<()> {
    match action {
        RuleEffect::Allow | RuleEffect::Deny => Ok(()),
    }
}

fn validate_approval_mode(s: &str) -> Result<()> {
    match s {
        "always" | "auto_for_event_types" | "never_required" => Ok(()),
        other => Err(Error::InvalidArgument(format!(
            "invalid approval_mode {other:?}; expected 'always' | 'auto_for_event_types' | 'never_required'"
        ))),
    }
}
