//! OHD Storage foreign-language bindings (uniffi).
//!
//! Wraps the `ohd-storage-core` API in a thin uniffi-compatible facade so a
//! single Rust core can be linked from Android / iOS / Python (Connect mobile,
//! Connect desktop, the conformance harness) without hand-written FFI shims.
//!
//! # Why a thin facade
//!
//! `ohd-storage-core` exposes plenty of Rust-idiomatic types
//! (`std::sync::Mutex<rusqlite::Connection>`, builder patterns, lifetime-bound
//! error variants) that uniffi can't represent across the FFI boundary
//! cleanly. The facade in this crate hides those and surfaces a small
//! kotlin/swift/python-friendly object graph:
//!
//! ```text
//! OhdStorage              ← Object (Arc<…>)
//! ├── open(path, key_hex)
//! ├── create(path, key_hex)
//! ├── path() -> String
//! ├── user_ulid() -> String          (Crockford-base32)
//! ├── put_event(EventInputDto) -> PutEventOutcomeDto
//! ├── query_events(EventFilterDto) -> Vec<EventDto>
//! └── issue_self_session_token() -> String
//!
//! OhdError                ← typed enum surfaced as a Kotlin sealed class /
//!                           Swift enum / Python exception subclasses.
//! ```
//!
//! # Generation flow
//!
//! 1. Compile this crate with `cargo build -p ohd-storage-bindings --release`
//!    (and per-Android-ABI via `cargo ndk -t arm64-v8a -t armeabi-v7a -t
//!    x86_64 build --release` from inside this crate). Produces
//!    `libohd_storage_bindings.so` per ABI.
//! 2. Run `cargo run --features cli --bin uniffi-bindgen -- generate
//!    --library target/<abi>/release/libohd_storage_bindings.so --language
//!    kotlin --out-dir <android-app>/src/main/java/uniffi`. Produces
//!    `uniffi/ohd_storage/ohd_storage.kt` + the JNA loader stubs.
//! 3. Drop the `.so` files into `app/src/main/jniLibs/<abi>/` and the `.kt`
//!    into the source set; the Android Gradle Plugin packages both into the
//!    `.aar`.
//!
//! See `connect/android/BUILD.md` for the end-to-end recipe.
//!
//! # Concurrency
//!
//! `Storage` is `!Sync` because it owns a `Mutex<Connection>` — but the
//! `Mutex` itself is what makes it externally sharable. The uniffi `Object`
//! macro wraps the facade in an `Arc`, and we serialise calls through the
//! inner `Storage` mutex. Foreign callers see a single thread-safe handle.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

use ohd_storage_core as core;

// PyO3 module — only compiled when the `pyo3` feature is on. Lives in a
// sibling source file so the uniffi `setup_scaffolding!` macro and the PyO3
// `#[pymodule]` macro stay textually separate. Both compile into the same
// cdylib without symbol collision (`_uniffi_*` vs `PyInit_*`). Build via
// `maturin build --release --features pyo3`. See
// `crates/ohd-storage-bindings/README.md` for the recipe.
#[cfg(feature = "pyo3")]
mod pyo3_module;

// =============================================================================
// uniffi setup
// =============================================================================
//
// Proc-macro mode (no `.udl`). The scaffolding macro emits a `_scaffolding_*`
// module containing the FFI shims uniffi-bindgen consumes. The single
// argument is the namespace under which generated bindings are exposed
// (Kotlin: `package uniffi.ohd_storage`; Swift: `enum OhdStorage`; Python:
// `import ohd_storage`). Keep it stable — changing it is a breaking API
// change for downstream consumers.
uniffi::setup_scaffolding!("ohd_storage");

// =============================================================================
// Errors
// =============================================================================

/// Errors surfaced over the FFI boundary.
///
/// Maps from `ohd_storage_core::Error`. We collapse the >25 internal variants
/// to a smaller surface — foreign callers only see the categories that are
/// actionable from a UI: open failure, auth/scope, validation, not-found,
/// internal. The original code string survives in `code` for audit/logs.
///
/// `code` follows the OHDC `ErrorInfo.code` catalog (see
/// `spec/ohdc-protocol.md`).
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum OhdError {
    /// Storage file couldn't be opened — wrong key, corrupt file, missing
    /// directory.
    #[error("open failed: {message}")]
    OpenFailed {
        /// Underlying error message.
        message: String,
    },
    /// Token missing, wrong kind, expired, revoked, or operation out of
    /// scope.
    #[error("auth failed ({code}): {message}")]
    Auth {
        /// OHDC error code (e.g. `WRONG_TOKEN_KIND`, `OUT_OF_SCOPE`).
        code: String,
        /// Human-readable message.
        message: String,
    },
    /// Input validation failure — unknown event type, wrong value type,
    /// invalid ULID, etc.
    #[error("invalid input ({code}): {message}")]
    InvalidInput {
        /// OHDC error code.
        code: String,
        /// Human-readable message.
        message: String,
    },
    /// Resource not found.
    #[error("not found")]
    NotFound,
    /// Anything not in the categories above (I/O, internal SQLite errors,
    /// JSON failures, etc.).
    #[error("internal ({code}): {message}")]
    Internal {
        /// OHDC error code.
        code: String,
        /// Human-readable message.
        message: String,
    },
}

impl From<core::Error> for OhdError {
    fn from(e: core::Error) -> Self {
        let code = e.code().to_string();
        let message = e.to_string();
        use core::Error as E;
        match e {
            E::Io(_) => OhdError::OpenFailed { message },
            E::Sqlite(_) => OhdError::OpenFailed { message },
            E::Unauthenticated
            | E::TokenExpired
            | E::TokenRevoked
            | E::WrongTokenKind(_)
            | E::OutOfScope
            | E::ApprovalTimeout => OhdError::Auth { code, message },
            E::UnknownType(_)
            | E::UnknownChannel { .. }
            | E::WrongValueType(_)
            | E::InvalidUnit(_)
            | E::InvalidEnum(_)
            | E::MissingRequiredChannel(_)
            | E::InvalidUlid
            | E::InvalidTimestamp
            | E::InvalidFilter(_)
            | E::InvalidArgument(_)
            | E::Json(_) => OhdError::InvalidInput { code, message },
            E::NotFound | E::CaseNotFound | E::EventDeleted => OhdError::NotFound,
            _ => OhdError::Internal { code, message },
        }
    }
}

type Result<T> = std::result::Result<T, OhdError>;

// =============================================================================
// DTOs (records crossing the FFI boundary)
// =============================================================================

/// A typed channel scalar surfaced over FFI.
///
/// uniffi 0.28 doesn't support untagged enums (Rust's `#[serde(untagged)]`
/// pattern), so we flatten to a tagged record where exactly one of the
/// `*_value` fields is set per the `value_kind` discriminant. Keeping it as a
/// record-with-tag (rather than an enum) makes Kotlin/Swift call sites
/// trivial: `ChannelValueDto("value", ValueKind.REAL, realValue=6.4)`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChannelValueDto {
    /// Channel path within the event's type, e.g. `"value"` or
    /// `"systolic.mmhg"`.
    pub channel_path: String,
    /// Which value variant is set.
    pub value_kind: ValueKind,
    /// Real-typed scalar (set iff `value_kind == REAL`).
    pub real_value: Option<f64>,
    /// Int-typed scalar.
    pub int_value: Option<i64>,
    /// Bool-typed scalar.
    pub bool_value: Option<bool>,
    /// Text-typed scalar.
    pub text_value: Option<String>,
    /// Enum ordinal.
    pub enum_ordinal: Option<i32>,
}

/// Discriminant for [`ChannelValueDto`].
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum ValueKind {
    /// `f64`.
    Real,
    /// `i64`.
    Int,
    /// `bool`.
    Bool,
    /// `String`.
    Text,
    /// Append-only enum ordinal.
    EnumOrdinal,
}

impl ChannelValueDto {
    fn into_core(self) -> Result<core::events::ChannelValue> {
        use core::events::{ChannelScalar, ChannelValue};
        let value = match self.value_kind {
            ValueKind::Real => ChannelScalar::Real {
                real_value: self.real_value.ok_or_else(|| OhdError::InvalidInput {
                    code: "WRONG_VALUE_TYPE".into(),
                    message: format!(
                        "channel {} marked REAL but no real_value",
                        self.channel_path
                    ),
                })?,
            },
            ValueKind::Int => ChannelScalar::Int {
                int_value: self.int_value.ok_or_else(|| OhdError::InvalidInput {
                    code: "WRONG_VALUE_TYPE".into(),
                    message: format!("channel {} marked INT but no int_value", self.channel_path),
                })?,
            },
            ValueKind::Bool => ChannelScalar::Bool {
                bool_value: self.bool_value.ok_or_else(|| OhdError::InvalidInput {
                    code: "WRONG_VALUE_TYPE".into(),
                    message: format!(
                        "channel {} marked BOOL but no bool_value",
                        self.channel_path
                    ),
                })?,
            },
            ValueKind::Text => ChannelScalar::Text {
                text_value: self
                    .text_value
                    .clone()
                    .ok_or_else(|| OhdError::InvalidInput {
                        code: "WRONG_VALUE_TYPE".into(),
                        message: format!(
                            "channel {} marked TEXT but no text_value",
                            self.channel_path
                        ),
                    })?,
            },
            ValueKind::EnumOrdinal => ChannelScalar::EnumOrdinal {
                enum_ordinal: self.enum_ordinal.ok_or_else(|| OhdError::InvalidInput {
                    code: "WRONG_VALUE_TYPE".into(),
                    message: format!(
                        "channel {} marked ENUM but no enum_ordinal",
                        self.channel_path
                    ),
                })?,
            },
        };
        Ok(ChannelValue {
            channel_path: self.channel_path,
            value,
        })
    }

    fn from_core(v: core::events::ChannelValue) -> Self {
        use core::events::ChannelScalar;
        let mut out = Self {
            channel_path: v.channel_path,
            value_kind: ValueKind::Real,
            real_value: None,
            int_value: None,
            bool_value: None,
            text_value: None,
            enum_ordinal: None,
        };
        match v.value {
            ChannelScalar::Real { real_value } => {
                out.value_kind = ValueKind::Real;
                out.real_value = Some(real_value);
            }
            ChannelScalar::Int { int_value } => {
                out.value_kind = ValueKind::Int;
                out.int_value = Some(int_value);
            }
            ChannelScalar::Bool { bool_value } => {
                out.value_kind = ValueKind::Bool;
                out.bool_value = Some(bool_value);
            }
            ChannelScalar::Text { text_value } => {
                out.value_kind = ValueKind::Text;
                out.text_value = Some(text_value);
            }
            ChannelScalar::EnumOrdinal { enum_ordinal } => {
                out.value_kind = ValueKind::EnumOrdinal;
                out.enum_ordinal = Some(enum_ordinal);
            }
        }
        out
    }
}

/// Sparse event input crossing the FFI boundary.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EventInputDto {
    /// Measurement time (signed Unix ms; negative = pre-1970).
    pub timestamp_ms: i64,
    /// Optional duration.
    pub duration_ms: Option<i64>,
    /// Local offset.
    pub tz_offset_minutes: Option<i32>,
    /// IANA zone name.
    pub tz_name: Option<String>,
    /// Namespaced event type.
    pub event_type: String,
    /// Channel values.
    pub channels: Vec<ChannelValueDto>,
    /// Logical device id.
    pub device_id: Option<String>,
    /// Recording app name.
    pub app_name: Option<String>,
    /// Recording app version.
    pub app_version: Option<String>,
    /// Source string.
    pub source: Option<String>,
    /// Idempotency key.
    pub source_id: Option<String>,
    /// Notes.
    pub notes: Option<String>,
    /// `false` on detail rows (intake.* / measurement.ecg_second / sample
    /// rows). Defaults to `true` so every existing call site keeps minting
    /// top-level events. See `EventDto.top_level`.
    pub top_level: Option<bool>,
}

impl EventInputDto {
    fn into_core(self) -> Result<core::events::EventInput> {
        let mut channels = Vec::with_capacity(self.channels.len());
        for c in self.channels {
            channels.push(c.into_core()?);
        }
        Ok(core::events::EventInput {
            timestamp_ms: self.timestamp_ms,
            duration_ms: self.duration_ms,
            tz_offset_minutes: self.tz_offset_minutes,
            tz_name: self.tz_name,
            event_type: self.event_type,
            channels,
            device_id: self.device_id,
            app_name: self.app_name,
            app_version: self.app_version,
            source: self.source,
            source_id: self.source_id,
            notes: self.notes,
            top_level: self.top_level.unwrap_or(true),
            // Sample blocks aren't yet exposed across the FFI boundary; the
            // bindings caller writes channel scalars only. Adding sample-block
            // support to the uniffi DTO is a v1.x deliverable; today the
            // server-side `OhdcService.PutEvents` is the canonical path.
            sample_blocks: vec![],
            // Source signing (P2 of the closeout pass) lands as an
            // in-process API; the FFI surface doesn't yet carry signature
            // bytes. Future revision: `EventInputDto::source_signature`.
            source_signature: None,
        })
    }
}

/// One stored event.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EventDto {
    /// ULID (Crockford-base32).
    pub ulid: String,
    /// Signed Unix ms.
    pub timestamp_ms: i64,
    /// Duration.
    pub duration_ms: Option<i64>,
    /// Event type.
    pub event_type: String,
    /// Channels.
    pub channels: Vec<ChannelValueDto>,
    /// Optional notes.
    pub notes: Option<String>,
    /// Source.
    pub source: Option<String>,
    /// Soft-delete marker.
    pub deleted_at_ms: Option<i64>,
    /// `true` for entry-level events (what timelines / Recent / home counts
    /// surface by default). `false` for detail rows (intake.* under a
    /// food.eaten, per-second ECG samples, …). Search queries that target a
    /// specific event type ignore this — it's a UI hint, not a permission gate.
    pub top_level: bool,
}

impl EventDto {
    fn from_core(e: core::events::Event) -> Self {
        Self {
            ulid: e.ulid,
            timestamp_ms: e.timestamp_ms,
            duration_ms: e.duration_ms,
            event_type: e.event_type,
            channels: e
                .channels
                .into_iter()
                .map(ChannelValueDto::from_core)
                .collect(),
            notes: e.notes,
            source: e.source,
            deleted_at_ms: e.deleted_at_ms,
            top_level: e.top_level,
        }
    }
}

/// Outcome of a single `put_event` call.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PutEventOutcomeDto {
    /// `"committed" | "pending" | "error"`.
    pub outcome: String,
    /// ULID for committed/pending; empty for errors.
    pub ulid: String,
    /// Wall-clock ms when committed; pending expiry for pending; 0 for errors.
    pub timestamp_ms: i64,
    /// OHDC error code; empty unless `outcome == "error"`.
    pub error_code: String,
    /// Human-readable message; empty unless `outcome == "error"`.
    pub error_message: String,
}

impl PutEventOutcomeDto {
    fn from_core(r: core::events::PutEventResult) -> Self {
        use core::events::PutEventResult as R;
        match r {
            R::Committed {
                ulid,
                committed_at_ms,
            } => Self {
                outcome: "committed".into(),
                ulid,
                timestamp_ms: committed_at_ms,
                error_code: String::new(),
                error_message: String::new(),
            },
            R::Pending {
                ulid,
                expires_at_ms,
            } => Self {
                outcome: "pending".into(),
                ulid,
                timestamp_ms: expires_at_ms,
                error_code: String::new(),
                error_message: String::new(),
            },
            R::Error { code, message } => Self {
                outcome: "error".into(),
                ulid: String::new(),
                timestamp_ms: 0,
                error_code: code,
                error_message: message,
            },
        }
    }
}

/// Filter for [`OhdStorage::query_events`]. Subset of the full OHDC
/// `EventFilter`; the deeper predicate language stays server-side for now.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EventFilterDto {
    /// Inclusive lower time bound.
    pub from_ms: Option<i64>,
    /// Inclusive upper time bound.
    pub to_ms: Option<i64>,
    /// Allowlist of dotted event-type names.
    pub event_types_in: Vec<String>,
    /// Denylist of dotted event-type names.
    pub event_types_not_in: Vec<String>,
    /// Whether to include soft-deleted events.
    pub include_deleted: bool,
    /// Result cap.
    pub limit: Option<i64>,
    /// Predicate over the `top_level` column. `"all"` (default), `"top_level_only"`,
    /// `"non_top_level_only"`. Anything else is treated as `"all"`.
    pub visibility: Option<String>,
    /// Restrict to events with `source` exactly in this list. Empty = no filter.
    #[uniffi(default = [])]
    pub source_in: Vec<String>,
}

impl EventFilterDto {
    fn into_core(self) -> core::events::EventFilter {
        let visibility = match self.visibility.as_deref() {
            Some("top_level_only") => core::events::EventVisibility::TopLevelOnly,
            Some("non_top_level_only") => core::events::EventVisibility::NonTopLevelOnly,
            _ => core::events::EventVisibility::All,
        };
        core::events::EventFilter {
            from_ms: self.from_ms,
            to_ms: self.to_ms,
            event_types_in: self.event_types_in,
            event_types_not_in: self.event_types_not_in,
            include_deleted: self.include_deleted,
            include_superseded: true,
            limit: self.limit,
            device_id_in: vec![],
            source_in: self.source_in,
            event_ulids_in: vec![],
            sensitivity_classes_in: vec![],
            sensitivity_classes_not_in: vec![],
            channel_predicates: vec![],
            case_ulids_in: vec![],
            visibility,
        }
    }
}

// =============================================================================
// Grants
// =============================================================================

/// Filter for [`OhdStorage::list_grants`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct ListGrantsFilterDto {
    /// Include revoked grants.
    pub include_revoked: bool,
    /// Include hard-expired grants.
    pub include_expired: bool,
    /// Filter by grantee_kind exact match.
    pub grantee_kind: Option<String>,
    /// Page size (default 100).
    pub limit: Option<i64>,
}

impl ListGrantsFilterDto {
    fn into_core(self) -> core::grants::ListGrantsFilter {
        core::grants::ListGrantsFilter {
            include_revoked: self.include_revoked,
            include_expired: self.include_expired,
            grantee_kind: self.grantee_kind,
            only_grant_id: None,
            limit: self.limit,
        }
    }
}

/// Per-event-type rule (allow or deny).
#[derive(Debug, Clone, uniffi::Record)]
pub struct GrantEventTypeRuleDto {
    /// Dotted event-type name.
    pub event_type: String,
    /// `"allow"` or `"deny"`.
    pub effect: String,
}

/// Per-channel rule.
#[derive(Debug, Clone, uniffi::Record)]
pub struct GrantChannelRuleDto {
    /// Dotted event-type name.
    pub event_type: String,
    /// Channel path within that type.
    pub channel_path: String,
    /// `"allow"` or `"deny"`.
    pub effect: String,
}

/// Per-sensitivity-class rule.
#[derive(Debug, Clone, uniffi::Record)]
pub struct GrantSensitivityRuleDto {
    /// `"general" | "mental_health" | …`.
    pub sensitivity_class: String,
    /// `"allow"` or `"deny"`.
    pub effect: String,
}

/// Materialized grant row.
#[derive(Debug, Clone, uniffi::Record)]
pub struct GrantDto {
    /// Wire ULID (Crockford-base32).
    pub ulid: String,
    /// Grantee display label.
    pub grantee_label: String,
    /// `"human" | "app" | "service" | "emergency" | "device" | "delegate"`.
    pub grantee_kind: String,
    /// Free-text purpose.
    pub purpose: Option<String>,
    /// Creation timestamp (Unix ms).
    pub created_at_ms: i64,
    /// Hard-expiry; `None` = no hard expiry.
    pub expires_at_ms: Option<i64>,
    /// Revocation timestamp; `None` = active.
    pub revoked_at_ms: Option<i64>,
    /// `"allow"` or `"deny"`.
    pub default_action: String,
    /// `"always" | "auto_for_event_types" | "never_required"`.
    pub approval_mode: String,
    /// Aggregation-only.
    pub aggregation_only: bool,
    /// Strip notes on returned rows.
    pub strip_notes: bool,
    /// Notify on each access.
    pub notify_on_access: bool,
    /// Per-event-type read rules.
    pub event_type_rules: Vec<GrantEventTypeRuleDto>,
    /// Per-channel read rules.
    pub channel_rules: Vec<GrantChannelRuleDto>,
    /// Per-sensitivity-class read rules.
    pub sensitivity_rules: Vec<GrantSensitivityRuleDto>,
    /// Auto-approve event-type allowlist (when `approval_mode == "auto_for_event_types"`).
    pub auto_approve_event_types: Vec<String>,
}

impl GrantDto {
    fn from_core(g: core::grants::GrantRow) -> Self {
        Self {
            ulid: core::ulid::to_crockford(&g.ulid),
            grantee_label: g.grantee_label,
            grantee_kind: g.grantee_kind,
            purpose: g.purpose,
            created_at_ms: g.created_at_ms,
            expires_at_ms: g.expires_at_ms,
            revoked_at_ms: g.revoked_at_ms,
            default_action: g.default_action,
            approval_mode: g.approval_mode,
            aggregation_only: g.aggregation_only,
            strip_notes: g.strip_notes,
            notify_on_access: g.notify_on_access,
            event_type_rules: g
                .event_type_rules
                .into_iter()
                .map(|(et, eff)| GrantEventTypeRuleDto {
                    event_type: et,
                    effect: eff.as_str().to_string(),
                })
                .collect(),
            channel_rules: g
                .channel_rules
                .into_iter()
                .map(|c| GrantChannelRuleDto {
                    event_type: c.event_type,
                    channel_path: c.channel_path,
                    effect: c.effect.as_str().to_string(),
                })
                .collect(),
            sensitivity_rules: g
                .sensitivity_rules
                .into_iter()
                .map(|(c, eff)| GrantSensitivityRuleDto {
                    sensitivity_class: c,
                    effect: eff.as_str().to_string(),
                })
                .collect(),
            auto_approve_event_types: g.auto_approve_event_types,
        }
    }
}

/// Sparse builder used by [`OhdStorage::create_grant`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct CreateGrantInputDto {
    /// Display label for the grantee.
    pub grantee_label: String,
    /// `"human" | "app" | "service" | "emergency" | "device" | "delegate"`.
    pub grantee_kind: String,
    /// Free-text purpose.
    pub purpose: Option<String>,
    /// `"allow"` or `"deny"` (default `"deny"`).
    pub default_action: String,
    /// `"always" | "auto_for_event_types" | "never_required"`.
    pub approval_mode: String,
    /// Hard-expiry timestamp (Unix ms).
    pub expires_at_ms: Option<i64>,
    /// Per-event-type read rules.
    pub event_type_rules: Vec<GrantEventTypeRuleDto>,
    /// Per-channel read rules.
    pub channel_rules: Vec<GrantChannelRuleDto>,
    /// Per-sensitivity-class read rules.
    pub sensitivity_rules: Vec<GrantSensitivityRuleDto>,
    /// Per-event-type write rules.
    pub write_event_type_rules: Vec<GrantEventTypeRuleDto>,
    /// Auto-approve event-type allowlist.
    pub auto_approve_event_types: Vec<String>,
    /// Aggregation-only flag.
    pub aggregation_only: bool,
    /// Strip notes on returned rows.
    pub strip_notes: bool,
    /// Notify on every access.
    pub notify_on_access: bool,
}

impl CreateGrantInputDto {
    fn into_core(self) -> Result<core::grants::NewGrant> {
        let default_action = match self.default_action.as_str() {
            "allow" => core::grants::RuleEffect::Allow,
            "deny" | "" => core::grants::RuleEffect::Deny,
            other => {
                return Err(OhdError::InvalidInput {
                    code: "INVALID_ARGUMENT".into(),
                    message: format!("default_action must be 'allow' | 'deny'; got {other:?}"),
                })
            }
        };
        let approval_mode = if self.approval_mode.is_empty() {
            "always".to_string()
        } else {
            self.approval_mode
        };
        let event_type_rules = self
            .event_type_rules
            .into_iter()
            .map(|r| (r.event_type, core::grants::RuleEffect::parse(&r.effect)))
            .collect();
        let sensitivity_rules = self
            .sensitivity_rules
            .into_iter()
            .map(|r| {
                (
                    r.sensitivity_class,
                    core::grants::RuleEffect::parse(&r.effect),
                )
            })
            .collect();
        let write_event_type_rules = self
            .write_event_type_rules
            .into_iter()
            .map(|r| (r.event_type, core::grants::RuleEffect::parse(&r.effect)))
            .collect();
        let channel_rules = self
            .channel_rules
            .into_iter()
            .map(|c| core::grants::ChannelRuleSpec {
                event_type: c.event_type,
                channel_path: c.channel_path,
                effect: core::grants::RuleEffect::parse(&c.effect),
            })
            .collect();
        Ok(core::grants::NewGrant {
            grantee_label: self.grantee_label,
            grantee_kind: self.grantee_kind,
            delegate_for_user_ulid: None,
            purpose: self.purpose,
            default_action,
            approval_mode,
            expires_at_ms: self.expires_at_ms,
            event_type_rules,
            channel_rules,
            sensitivity_rules,
            write_event_type_rules,
            auto_approve_event_types: self.auto_approve_event_types,
            aggregation_only: self.aggregation_only,
            strip_notes: self.strip_notes,
            notify_on_access: self.notify_on_access,
            require_approval_per_query: false,
            max_queries_per_day: None,
            max_queries_per_hour: None,
            rolling_window_days: None,
            absolute_window: None,
            grantee_recovery_pubkey: None,
        })
    }
}

/// Result of [`OhdStorage::create_grant`] / `issue_retrospective_grant`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct GrantTokenDto {
    /// New grant ULID (Crockford-base32).
    pub grant_ulid: String,
    /// Cleartext bearer token (`ohdg_…`); shown to the user once.
    pub token: String,
    /// Convenience share URL.
    pub share_url: String,
}

/// Sparse update for [`OhdStorage::update_grant`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct GrantUpdateDto {
    /// New display label.
    pub grantee_label: Option<String>,
    /// New hard-expiry (`Some(0)` clears).
    pub expires_at_ms: Option<i64>,
}

/// Wrapper for [`OhdStorage::issue_retrospective_grant`] — same shape as
/// [`CreateGrantInputDto`] but separate type to keep the parameter list
/// readable from foreign-language call sites.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RetroGrantInputDto {
    /// Grant policy + scope.
    pub input: CreateGrantInputDto,
}

// =============================================================================
// Pending events
// =============================================================================

/// One pending event row.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PendingEventDto {
    /// Pending ULID (Crockford-base32).
    pub ulid: String,
    /// Submission time (Unix ms).
    pub submitted_at_ms: i64,
    /// Submitting grant ULID, when resolvable.
    pub submitting_grant_ulid: Option<String>,
    /// `"pending" | "approved" | "rejected" | "expired"`.
    pub status: String,
    /// Review time.
    pub reviewed_at_ms: Option<i64>,
    /// Optional rejection reason.
    pub rejection_reason: Option<String>,
    /// Auto-expiry (Unix ms).
    pub expires_at_ms: i64,
    /// The materialized event (decoded from the queued payload).
    pub event: EventDto,
}

impl PendingEventDto {
    fn from_core(p: core::pending::PendingRow) -> Self {
        Self {
            ulid: core::ulid::to_crockford(&p.ulid),
            submitted_at_ms: p.submitted_at_ms,
            submitting_grant_ulid: p
                .submitting_grant_ulid
                .as_ref()
                .map(core::ulid::to_crockford),
            status: p.status.as_str().to_string(),
            reviewed_at_ms: p.reviewed_at_ms,
            rejection_reason: p.rejection_reason,
            expires_at_ms: p.expires_at_ms,
            event: EventDto::from_core(p.event),
        }
    }
}

// =============================================================================
// Cases
// =============================================================================

/// State filter for [`OhdStorage::list_cases`].
#[derive(Debug, Clone, Copy, uniffi::Enum)]
pub enum CaseStateDto {
    /// `ended_at_ms IS NULL`.
    Open,
    /// `ended_at_ms IS NOT NULL`.
    Closed,
}

/// One case row.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CaseDto {
    /// Wire ULID (Crockford-base32).
    pub ulid: String,
    /// Type tag (`"emergency" | "admission" | "visit" | …`).
    pub case_type: String,
    /// Optional human-readable label.
    pub case_label: Option<String>,
    /// Start time (Unix ms).
    pub started_at_ms: i64,
    /// Close time (`None` = ongoing).
    pub ended_at_ms: Option<i64>,
    /// Parent case (structural rollup).
    pub parent_case_ulid: Option<String>,
    /// Predecessor case (handoff chain).
    pub predecessor_case_ulid: Option<String>,
    /// Authority that opened the case.
    pub opening_authority_grant_ulid: Option<String>,
    /// Inactivity threshold (hours) for auto-close.
    pub inactivity_close_after_h: Option<i32>,
    /// Last activity timestamp.
    pub last_activity_at_ms: i64,
}

impl CaseDto {
    fn from_core(c: core::cases::Case) -> Self {
        Self {
            ulid: core::ulid::to_crockford(&c.ulid),
            case_type: c.case_type,
            case_label: c.case_label,
            started_at_ms: c.started_at_ms,
            ended_at_ms: c.ended_at_ms,
            parent_case_ulid: c.parent_case_ulid.as_ref().map(core::ulid::to_crockford),
            predecessor_case_ulid: c
                .predecessor_case_ulid
                .as_ref()
                .map(core::ulid::to_crockford),
            opening_authority_grant_ulid: c
                .opening_authority_grant_ulid
                .as_ref()
                .map(core::ulid::to_crockford),
            inactivity_close_after_h: c.inactivity_close_after_h,
            last_activity_at_ms: c.last_activity_at_ms,
        }
    }
}

/// Case detail returned by [`OhdStorage::get_case`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct CaseDetailDto {
    /// The case row.
    pub case: CaseDto,
    /// Recent audit entries scoped to the case's opening authority.
    pub audit: Vec<AuditEntryDto>,
}

// =============================================================================
// Audit
// =============================================================================

/// Filter for [`OhdStorage::audit_query`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct AuditFilterDto {
    /// Inclusive lower time bound (Unix ms).
    pub from_ms: Option<i64>,
    /// Inclusive upper time bound (Unix ms).
    pub to_ms: Option<i64>,
    /// Filter by actor type string (`"self" | "grant" | "system" | "delegate"`).
    pub actor_type: Option<String>,
    /// Filter by action string.
    pub action: Option<String>,
    /// Filter by result string (`"success" | "partial" | "rejected" | "error"`).
    pub result: Option<String>,
    /// Page size.
    pub limit: Option<i64>,
}

impl AuditFilterDto {
    fn into_core(self) -> core::audit::AuditQuery {
        core::audit::AuditQuery {
            from_ms: self.from_ms,
            to_ms: self.to_ms,
            grant_id: None,
            actor_type: self.actor_type,
            action: self.action,
            result: self.result,
            limit: self.limit,
        }
    }
}

/// One audit log entry.
#[derive(Debug, Clone, uniffi::Record)]
pub struct AuditEntryDto {
    /// Time of the operation.
    pub ts_ms: i64,
    /// `"self" | "grant" | "system" | "delegate"`.
    pub actor_type: String,
    /// Action label.
    pub action: String,
    /// Sub-classification.
    pub query_kind: Option<String>,
    /// JSON-encoded request payload.
    pub query_params_json: Option<String>,
    /// Returned rows.
    pub rows_returned: Option<i64>,
    /// Filtered rows.
    pub rows_filtered: Option<i64>,
    /// `"success" | "partial" | "rejected" | "error"`.
    pub result: String,
    /// Failure reason.
    pub reason: Option<String>,
}

impl AuditEntryDto {
    fn from_core(e: core::audit::AuditEntry) -> Self {
        Self {
            ts_ms: e.ts_ms,
            actor_type: e.actor_type.as_str().to_string(),
            action: e.action,
            query_kind: e.query_kind,
            query_params_json: e.query_params_json,
            rows_returned: e.rows_returned,
            rows_filtered: e.rows_filtered,
            result: e.result.as_str().to_string(),
            reason: e.reason,
        }
    }
}

// =============================================================================
// Emergency config
// =============================================================================

/// Trust root entry for [`EmergencyConfigDto`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct TrustedAuthorityDto {
    /// Display label.
    pub label: String,
    /// Country / scope tag.
    pub scope: Option<String>,
    /// PEM-encoded certificate blob (opaque to storage).
    pub public_key_pem: Option<String>,
    /// Whether this root is the project default.
    pub is_default: bool,
}

/// Emergency / break-glass configuration.
///
/// Mirrors the eight sections of `connect/spec/screens-emergency.md` →
/// "OHD Connect — patient side → Settings tab: 'Emergency / Break-glass'".
#[derive(Debug, Clone, uniffi::Record)]
pub struct EmergencyConfigDto {
    /// Section 1 — feature toggle.
    pub enabled: bool,
    /// Section 2 — broadcast a low-power BLE beacon.
    pub bluetooth_beacon: bool,
    /// Section 3 — slider 10..=300 seconds (default 30).
    pub approval_timeout_seconds: i32,
    /// Section 3 — `"allow"` (better for unconscious) or `"refuse"`.
    pub default_action_on_timeout: String,
    /// Section 4 — `"full"` or `"basic_only"`.
    pub lock_screen_visibility: String,
    /// Section 5 — `0 | 3 | 12 | 24` hours.
    pub history_window_hours: i32,
    /// Section 5 — channel paths in the emergency profile.
    pub channel_paths_allowed: Vec<String>,
    /// Section 5 — sensitivity classes the emergency profile exposes.
    pub sensitivity_classes_allowed: Vec<String>,
    /// Section 6 — share GPS to the responding authority.
    pub share_location: bool,
    /// Section 7 — trust roots the patient accepts.
    pub trusted_authorities: Vec<TrustedAuthorityDto>,
    /// Section 8 — Good Samaritan bystander proxy.
    pub bystander_proxy_enabled: bool,
    /// Last update time (Unix ms).
    pub updated_at_ms: i64,
}

impl EmergencyConfigDto {
    fn from_core(c: core::emergency_config::EmergencyConfig) -> Self {
        Self {
            enabled: c.enabled,
            bluetooth_beacon: c.bluetooth_beacon,
            approval_timeout_seconds: c.approval_timeout_seconds,
            default_action_on_timeout: c.default_action_on_timeout,
            lock_screen_visibility: c.lock_screen_visibility,
            history_window_hours: c.history_window_hours,
            channel_paths_allowed: c.channel_paths_allowed,
            sensitivity_classes_allowed: c.sensitivity_classes_allowed,
            share_location: c.share_location,
            trusted_authorities: c
                .trusted_authorities
                .into_iter()
                .map(|t| TrustedAuthorityDto {
                    label: t.label,
                    scope: t.scope,
                    public_key_pem: t.public_key_pem,
                    is_default: t.is_default,
                })
                .collect(),
            bystander_proxy_enabled: c.bystander_proxy_enabled,
            updated_at_ms: c.updated_at_ms,
        }
    }

    fn into_core(self) -> core::emergency_config::EmergencyConfig {
        core::emergency_config::EmergencyConfig {
            enabled: self.enabled,
            bluetooth_beacon: self.bluetooth_beacon,
            approval_timeout_seconds: self.approval_timeout_seconds,
            default_action_on_timeout: self.default_action_on_timeout,
            lock_screen_visibility: self.lock_screen_visibility,
            history_window_hours: self.history_window_hours,
            channel_paths_allowed: self.channel_paths_allowed,
            sensitivity_classes_allowed: self.sensitivity_classes_allowed,
            share_location: self.share_location,
            trusted_authorities: self
                .trusted_authorities
                .into_iter()
                .map(|t| core::emergency_config::TrustedAuthority {
                    label: t.label,
                    scope: t.scope,
                    public_key_pem: t.public_key_pem,
                    is_default: t.is_default,
                })
                .collect(),
            bystander_proxy_enabled: self.bystander_proxy_enabled,
            updated_at_ms: self.updated_at_ms,
        }
    }
}

// =============================================================================
// Source signing
// =============================================================================

/// One registered source signer.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SignerDto {
    /// Operator-assigned KID (e.g. `"libre.eu.2026-01"`).
    pub signer_kid: String,
    /// Human label.
    pub signer_label: String,
    /// `"ed25519" | "rs256" | "es256"`.
    pub sig_alg: String,
    /// PEM-encoded SubjectPublicKeyInfo.
    pub public_key_pem: String,
    /// Registration time (Unix ms).
    pub registered_at_ms: i64,
    /// Revocation time; `None` = active.
    pub revoked_at_ms: Option<i64>,
}

impl SignerDto {
    fn from_core(s: core::source_signing::Signer) -> Self {
        Self {
            signer_kid: s.signer_kid,
            signer_label: s.signer_label,
            sig_alg: s.sig_alg,
            public_key_pem: s.public_key_pem,
            registered_at_ms: s.registered_at_ms,
            revoked_at_ms: s.revoked_at_ms,
        }
    }
}

// =============================================================================
// OhdStorage object
// =============================================================================

/// Foreign-language handle to an open OHD Storage file.
///
/// Thread-safe (every method serializes through the inner `Storage` mutex).
/// uniffi materializes this on Kotlin as `class OhdStorage` (constructed via
/// `OhdStorage.open(...)` / `OhdStorage.create(...)` factory methods) and on
/// Swift as `final class OhdStorage`.
#[derive(uniffi::Object)]
pub struct OhdStorage {
    inner: core::Storage,
}

#[uniffi::export]
impl OhdStorage {
    /// Open an existing storage file. Errors out if the file doesn't exist
    /// (use [`OhdStorage::create`] for first-run).
    ///
    /// `key_hex`: SQLCipher key as a 64-char hex string (32 bytes). Empty
    /// string opens unencrypted (testing only — see
    /// `spec/encryption.md`).
    #[uniffi::constructor]
    pub fn open(path: String, key_hex: String) -> Result<Arc<OhdStorage>> {
        Self::open_inner(path, key_hex, false)
    }

    /// Create a new storage file (or open it if it already exists). Stamps
    /// `_meta.user_ulid`, `format_version`, runs migrations.
    #[uniffi::constructor]
    pub fn create(path: String, key_hex: String) -> Result<Arc<OhdStorage>> {
        Self::open_inner(path, key_hex, true)
    }

    /// Path the storage file is backed by.
    pub fn path(&self) -> String {
        self.inner.path().to_string_lossy().into_owned()
    }

    /// User ULID stamped into `_meta.user_ulid` (Crockford-base32).
    pub fn user_ulid(&self) -> String {
        core::ulid::to_crockford(&self.inner.user_ulid())
    }

    /// Mint a fresh self-session token (`ohds_…`). Cleartext is shown
    /// exactly once — store it in the platform keystore.
    pub fn issue_self_session_token(&self) -> Result<String> {
        let user_ulid = self.inner.user_ulid();
        self.inner
            .with_conn(|conn| core::auth::issue_self_session_token(conn, user_ulid, None, None))
            .map_err(Into::into)
    }

    /// Write one event. The full batch RPC (`put_events`) is wrapped one-at-
    /// a-time here because uniffi 0.28's record support over Vec<…>-of-records
    /// is fine, but Kotlin call sites for "log a single thing" are more
    /// readable when the API takes one at a time.
    ///
    /// For bulk imports, call this in a loop or wait for the future
    /// `put_events_batch` deliverable.
    pub fn put_event(&self, input: EventInputDto) -> Result<PutEventOutcomeDto> {
        let core_input = input.into_core()?;
        let envelope = self.inner.envelope_key().cloned();
        let mut results = self
            .inner
            .with_conn_mut(|conn| {
                core::events::put_events(conn, &[core_input], None, false, envelope.as_ref())
            })
            .map_err(OhdError::from)?;
        Ok(PutEventOutcomeDto::from_core(results.remove(0)))
    }

    /// Read events under a filter. Returns the matching event rows in the
    /// core's natural order (TIME_DESC).
    ///
    /// Self-session scope only — grant scoping is wire-side (the Connect
    /// transport layer applies it). On-device callers always own their data.
    pub fn query_events(&self, filter: EventFilterDto) -> Result<Vec<EventDto>> {
        let core_filter = filter.into_core();
        let (events, _filtered) = self
            .inner
            .with_conn(|conn| core::events::query_events(conn, &core_filter, None))
            .map_err(OhdError::from)?;
        Ok(events.into_iter().map(EventDto::from_core).collect())
    }

    /// Count events matching `filter` without materialising the rows.
    ///
    /// Pure SQL `COUNT(*)` over the same time / event-type / deleted-flag
    /// predicates as [`query_events`], used by the Home stat tile to
    /// side-step the 10 000-row response cap. See
    /// [`core::events::count_events`] for the caveat about which filter
    /// fields are honoured (channel predicates and case scope are not).
    pub fn count_events(&self, filter: EventFilterDto) -> Result<u64> {
        let core_filter = filter.into_core();
        let count = self
            .inner
            .with_conn(|conn| core::events::count_events(conn, &core_filter))
            .map_err(OhdError::from)?;
        Ok(count as u64)
    }

    /// Soft-delete every event with `timestamp_ms < cutoff_ms` that isn't
    /// already deleted. Used by the free-tier 7-day retention worker on
    /// Android. Returns the number of rows touched.
    pub fn soft_delete_events_before(&self, cutoff_ms: i64) -> Result<u64> {
        let n = self
            .inner
            .with_conn(|conn| core::events::soft_delete_events_before(conn, cutoff_ms))
            .map_err(OhdError::from)?;
        Ok(n as u64)
    }

    // -------------------------------------------------------------------------
    // Agent tools (CORD + MCP) — uniffi shim over `ohd-mcp-core`.
    // -------------------------------------------------------------------------

    /// Catalog of agent tools as JSON. Same payload the standalone MCP
    /// server returns from `tools/list`. Kotlin / Android CORD calls this
    /// instead of carrying a hardcoded list.
    pub fn list_tools(&self) -> String {
        ohd_mcp_core::catalog_json()
    }

    /// Execute one agent tool by name. `input_json` is the JSON the
    /// model emitted inside its `tool_use` block; the return is the JSON
    /// that gets handed back as `tool_result`. Errors come back as
    /// `{"error": "..."}` strings — never throw.
    pub fn execute_tool(&self, name: String, input_json: String) -> String {
        ohd_mcp_core::dispatch_json(&name, &input_json, &self.inner)
    }

    /// On-disk format version (e.g. `"1.0"`).
    pub fn format_version(&self) -> String {
        core::FORMAT_VERSION.to_string()
    }

    /// OHDC protocol version this binding's core implements
    /// (e.g. `"ohdc.v0"`).
    pub fn protocol_version(&self) -> String {
        core::PROTOCOL_VERSION.to_string()
    }

    // -------------------------------------------------------------------------
    // Grants
    // -------------------------------------------------------------------------

    /// List grants. Self-session-equivalent (the binding always operates on
    /// the user's own storage).
    pub fn list_grants(&self, filter: ListGrantsFilterDto) -> Result<Vec<GrantDto>> {
        let core_filter = filter.into_core();
        let rows = self
            .inner
            .with_conn(|conn| core::grants::list_grants(conn, &core_filter))
            .map_err(OhdError::from)?;
        Ok(rows.into_iter().map(GrantDto::from_core).collect())
    }

    /// Create a new grant. Returns `(grant_ulid_crockford, token_string)`;
    /// the token is the cleartext bearer (`ohdg_…`) shown to the user once.
    pub fn create_grant(&self, req: CreateGrantInputDto) -> Result<GrantTokenDto> {
        let new_grant = req.into_core()?;
        let envelope = self.inner.envelope_key().cloned();
        let recovery = self.inner.recovery_keypair().cloned();
        let user_ulid = self.inner.user_ulid();
        let (grant_id, grant_ulid) = self
            .inner
            .with_conn_mut(|conn| match envelope.as_ref() {
                Some(env) => core::grants::create_grant_with_envelope(
                    conn,
                    &new_grant,
                    env,
                    recovery.as_ref(),
                ),
                None => core::grants::create_grant(conn, &new_grant),
            })
            .map_err(OhdError::from)?;
        let ttl_ms = new_grant
            .expires_at_ms
            .map(|exp| exp - core::format::now_ms());
        let token = self
            .inner
            .with_conn(|conn| {
                core::auth::issue_grant_token(
                    conn,
                    user_ulid,
                    grant_id,
                    core::auth::TokenKind::Grant,
                    ttl_ms,
                )
            })
            .map_err(OhdError::from)?;
        Ok(GrantTokenDto {
            grant_ulid: core::ulid::to_crockford(&grant_ulid),
            token,
            share_url: format!("ohd://grant/{}", core::ulid::to_crockford(&grant_ulid)),
        })
    }

    /// Revoke a grant by its ULID (Crockford).
    pub fn revoke_grant(&self, grant_ulid: String, reason: Option<String>) -> Result<()> {
        let ulid_bytes = core::ulid::parse_crockford(&grant_ulid).map_err(OhdError::from)?;
        let grant_id = self
            .inner
            .with_conn(|conn| core::grants::grant_id_by_ulid(conn, &ulid_bytes))
            .map_err(OhdError::from)?;
        self.inner
            .with_conn(|conn| core::grants::revoke_grant(conn, grant_id, reason.as_deref()))
            .map_err(OhdError::from)?;
        Ok(())
    }

    /// Update a grant's mutable fields.
    pub fn update_grant(&self, grant_ulid: String, update: GrantUpdateDto) -> Result<()> {
        let ulid_bytes = core::ulid::parse_crockford(&grant_ulid).map_err(OhdError::from)?;
        let grant_id = self
            .inner
            .with_conn(|conn| core::grants::grant_id_by_ulid(conn, &ulid_bytes))
            .map_err(OhdError::from)?;
        let core_update = core::grants::GrantUpdate {
            grantee_label: update.grantee_label,
            expires_at_ms: update.expires_at_ms,
        };
        self.inner
            .with_conn_mut(|conn| core::grants::update_grant(conn, grant_id, &core_update))
            .map_err(OhdError::from)?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Pending events
    // -------------------------------------------------------------------------

    /// List pending events (always returns the user's full pending queue;
    /// `pending` status only by default — pass `status=None` from a higher
    /// layer if you want all states).
    pub fn list_pending(&self) -> Result<Vec<PendingEventDto>> {
        let filter = core::pending::ListPendingFilter::default();
        let rows = self
            .inner
            .with_conn(|conn| core::pending::list_pending(conn, &filter))
            .map_err(OhdError::from)?;
        Ok(rows.into_iter().map(PendingEventDto::from_core).collect())
    }

    /// Approve a pending event. When `also_auto_approve_event_type` is true,
    /// adds the event's type to the submitting grant's auto-approve list.
    pub fn approve_pending(
        &self,
        pending_ulid: String,
        also_auto_approve_event_type: bool,
    ) -> Result<()> {
        let ulid_bytes = core::ulid::parse_crockford(&pending_ulid).map_err(OhdError::from)?;
        let envelope = self.inner.envelope_key().cloned();
        self.inner
            .with_conn_mut(|conn| {
                core::pending::approve_pending(
                    conn,
                    &ulid_bytes,
                    also_auto_approve_event_type,
                    envelope.as_ref(),
                )
            })
            .map_err(OhdError::from)?;
        Ok(())
    }

    /// Reject a pending event with an optional reason.
    pub fn reject_pending(&self, pending_ulid: String, reason: Option<String>) -> Result<()> {
        let ulid_bytes = core::ulid::parse_crockford(&pending_ulid).map_err(OhdError::from)?;
        self.inner
            .with_conn_mut(|conn| {
                core::pending::reject_pending(conn, &ulid_bytes, reason.as_deref())
            })
            .map_err(OhdError::from)?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Cases
    // -------------------------------------------------------------------------

    /// List cases. Filter by state (`Open` / `Closed` / `null` = both).
    pub fn list_cases(&self, state_filter: Option<CaseStateDto>) -> Result<Vec<CaseDto>> {
        let include_closed = match state_filter {
            None => true,
            Some(CaseStateDto::Closed) => true,
            Some(CaseStateDto::Open) => false,
        };
        let filter = core::cases::ListCasesFilter {
            include_closed,
            ..Default::default()
        };
        let rows = self
            .inner
            .with_conn(|conn| core::cases::list_cases(conn, &filter))
            .map_err(OhdError::from)?;
        let mut out: Vec<CaseDto> = rows.into_iter().map(CaseDto::from_core).collect();
        // When the caller asked for closed-only, drop the open ones (the core
        // filter has no `closed_only` predicate; keep the surface simple here).
        if matches!(state_filter, Some(CaseStateDto::Closed)) {
            out.retain(|c| c.ended_at_ms.is_some());
        }
        Ok(out)
    }

    /// Get one case by its ULID (Crockford). Returns the case + recent audit.
    pub fn get_case(&self, case_ulid: String) -> Result<CaseDetailDto> {
        let ulid_bytes = core::ulid::parse_crockford(&case_ulid).map_err(OhdError::from)?;
        let case_id = self
            .inner
            .with_conn(|conn| core::cases::case_id_by_ulid(conn, &ulid_bytes))
            .map_err(OhdError::from)?;
        let case = self
            .inner
            .with_conn(|conn| core::cases::read_case(conn, case_id))
            .map_err(OhdError::from)?;
        // Pull the most recent audit entries scoped to the case's opening
        // authority. v1's audit table doesn't carry case_id; we use
        // opening_authority_grant_id as the proxy. Patient-curated cases
        // (no authority) carry no audit list here.
        let mut audit_entries: Vec<AuditEntryDto> = Vec::new();
        if let Some(authority_ulid) = case.opening_authority_grant_ulid.as_ref() {
            let gid = self
                .inner
                .with_conn(|conn| core::grants::grant_id_by_ulid(conn, authority_ulid))
                .map_err(OhdError::from)?;
            let q = core::audit::AuditQuery {
                grant_id: Some(gid),
                limit: Some(50),
                ..Default::default()
            };
            let rows = self
                .inner
                .with_conn(|conn| core::audit::query(conn, &q))
                .map_err(OhdError::from)?;
            audit_entries = rows.into_iter().map(AuditEntryDto::from_core).collect();
        }
        let _ = case_id; // kept to satisfy the resolution flow above
        Ok(CaseDetailDto {
            case: CaseDto::from_core(case),
            audit: audit_entries,
        })
    }

    /// Force-close a case (self-session authority). Idempotent.
    pub fn force_close_case(&self, case_ulid: String) -> Result<()> {
        let ulid_bytes = core::ulid::parse_crockford(&case_ulid).map_err(OhdError::from)?;
        let case_id = self
            .inner
            .with_conn(|conn| core::cases::case_id_by_ulid(conn, &ulid_bytes))
            .map_err(OhdError::from)?;
        // No reopen token issued for patient-driven force-close.
        self.inner
            .with_conn_mut(|conn| core::cases::close_case(conn, case_id, None, false, None))
            .map_err(OhdError::from)?;
        Ok(())
    }

    /// Issue a retrospective grant against an existing case. Returns the
    /// freshly-minted grant ULID + bearer token.
    pub fn issue_retrospective_grant(
        &self,
        case_ulid: String,
        req: RetroGrantInputDto,
    ) -> Result<GrantTokenDto> {
        let ulid_bytes = core::ulid::parse_crockford(&case_ulid).map_err(OhdError::from)?;
        let case_id = self
            .inner
            .with_conn(|conn| core::cases::case_id_by_ulid(conn, &ulid_bytes))
            .map_err(OhdError::from)?;
        let new_grant = req.input.into_core()?;
        let envelope = self.inner.envelope_key().cloned();
        let recovery = self.inner.recovery_keypair().cloned();
        let user_ulid = self.inner.user_ulid();
        let (grant_id, grant_ulid) = self
            .inner
            .with_conn_mut(|conn| match envelope.as_ref() {
                Some(env) => core::grants::create_grant_with_envelope(
                    conn,
                    &new_grant,
                    env,
                    recovery.as_ref(),
                ),
                None => core::grants::create_grant(conn, &new_grant),
            })
            .map_err(OhdError::from)?;
        self.inner
            .with_conn(|conn| core::cases::bind_grant_to_cases(conn, grant_id, &[case_id]))
            .map_err(OhdError::from)?;
        let ttl_ms = new_grant
            .expires_at_ms
            .map(|exp| exp - core::format::now_ms());
        let token = self
            .inner
            .with_conn(|conn| {
                core::auth::issue_grant_token(
                    conn,
                    user_ulid,
                    grant_id,
                    core::auth::TokenKind::Grant,
                    ttl_ms,
                )
            })
            .map_err(OhdError::from)?;
        Ok(GrantTokenDto {
            grant_ulid: core::ulid::to_crockford(&grant_ulid),
            token,
            share_url: format!("ohd://grant/{}", core::ulid::to_crockford(&grant_ulid)),
        })
    }

    // -------------------------------------------------------------------------
    // Audit
    // -------------------------------------------------------------------------

    /// Run an audit query and return matching rows.
    pub fn audit_query(&self, filter: AuditFilterDto) -> Result<Vec<AuditEntryDto>> {
        let q = filter.into_core();
        let rows = self
            .inner
            .with_conn(|conn| core::audit::query(conn, &q))
            .map_err(OhdError::from)?;
        Ok(rows.into_iter().map(AuditEntryDto::from_core).collect())
    }

    // -------------------------------------------------------------------------
    // Emergency config (operator-side)
    // -------------------------------------------------------------------------

    /// Read the user's emergency / break-glass configuration. Returns the
    /// default (feature off) if no row exists yet.
    pub fn get_emergency_config(&self) -> Result<EmergencyConfigDto> {
        let user = self.inner.user_ulid();
        let cfg = self
            .inner
            .with_conn(|conn| core::emergency_config::get_emergency_config(conn, user))
            .map_err(OhdError::from)?;
        Ok(EmergencyConfigDto::from_core(cfg))
    }

    /// Replace the user's emergency configuration. Validates timeout range
    /// (10..=300s), default action (`allow|refuse`), visibility
    /// (`full|basic_only`), and history window (`0|3|12|24`).
    pub fn set_emergency_config(&self, cfg: EmergencyConfigDto) -> Result<()> {
        let user = self.inner.user_ulid();
        let core_cfg = cfg.into_core();
        let now = core::format::now_ms();
        self.inner
            .with_conn(|conn| {
                core::emergency_config::set_emergency_config(conn, user, &core_cfg, now)
            })
            .map_err(OhdError::from)?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Export
    // -------------------------------------------------------------------------

    /// Export every event + grant + audit row this storage owns to a single
    /// CBOR-encoded byte buffer suitable for writing to a portable `.ohd`
    /// file. Self-session by definition.
    pub fn export_all(&self) -> Result<Vec<u8>> {
        let user = self.inner.user_ulid();
        let token = core::auth::ResolvedToken {
            kind: core::auth::TokenKind::SelfSession,
            user_ulid: user,
            grant_id: None,
            grant_ulid: None,
            grantee_label: None,
            delegate_for_user_ulid: None,
        };
        core::ohdc::export_all(&self.inner, &token, None, None, &[]).map_err(OhdError::from)
    }

    // -------------------------------------------------------------------------
    // Source signing (operator registry)
    // -------------------------------------------------------------------------

    /// Register a source signer (high-trust integration's public key).
    pub fn register_signer(
        &self,
        signer_kid: String,
        signer_label: String,
        sig_alg: String,
        public_key_pem: String,
    ) -> Result<SignerDto> {
        let signer = self
            .inner
            .with_conn(|conn| {
                core::source_signing::register_signer(
                    conn,
                    &signer_kid,
                    &signer_label,
                    &sig_alg,
                    &public_key_pem,
                )
            })
            .map_err(OhdError::from)?;
        Ok(SignerDto::from_core(signer))
    }

    /// List all registered signers (active + revoked).
    pub fn list_signers(&self) -> Result<Vec<SignerDto>> {
        let rows = self
            .inner
            .with_conn(|conn| core::source_signing::list_signers(conn))
            .map_err(OhdError::from)?;
        Ok(rows.into_iter().map(SignerDto::from_core).collect())
    }

    /// Revoke a signer by KID. Existing signed events stay verifiable; new
    /// submissions under this KID are rejected.
    pub fn revoke_signer(&self, signer_kid: String) -> Result<()> {
        self.inner
            .with_conn(|conn| core::source_signing::revoke_signer(conn, &signer_kid))
            .map_err(OhdError::from)?;
        Ok(())
    }
}

impl OhdStorage {
    fn open_inner(path: String, key_hex: String, create: bool) -> Result<Arc<Self>> {
        let cipher_key = if key_hex.is_empty() {
            vec![]
        } else {
            hex_decode(&key_hex)?
        };
        let cfg = core::StorageConfig {
            path: PathBuf::from(path),
            cipher_key,
            create_if_missing: create,
            create_mode: core::format::DeploymentMode::Primary,
            create_user_ulid: None,
        };
        let storage = core::Storage::open(cfg).map_err(OhdError::from)?;
        Ok(Arc::new(Self { inner: storage }))
    }
}

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    // ohd-storage-core depends on `hex`, but uniffi rejects external trait
    // imports in this position; reuse the tiny inline decoder so the FFI
    // surface stays self-contained.
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err(OhdError::InvalidInput {
            code: "INVALID_ARGUMENT".into(),
            message: "key_hex must have even length".into(),
        });
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(c: u8) -> Result<u8> {
    Ok(match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => {
            return Err(OhdError::InvalidInput {
                code: "INVALID_ARGUMENT".into(),
                message: format!("non-hex char in key_hex: {}", c as char),
            })
        }
    })
}

// =============================================================================
// Top-level helpers
// =============================================================================

/// Build version of the storage core packed into this binding.
#[uniffi::export]
pub fn storage_version() -> String {
    core::STORAGE_VERSION.to_string()
}

/// OHDC protocol version this binding's core implements.
#[uniffi::export]
pub fn protocol_version() -> String {
    core::PROTOCOL_VERSION.to_string()
}

/// On-disk format version this binding's core understands.
#[uniffi::export]
pub fn format_version() -> String {
    core::FORMAT_VERSION.to_string()
}
