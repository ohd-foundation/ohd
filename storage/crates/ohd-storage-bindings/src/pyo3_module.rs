//! PyO3 bindings for OHD Storage.
//!
//! Mirrors the uniffi facade in [`crate`] but uses `pyo3 0.28` directly so the
//! resulting cdylib can be packaged as a real Python wheel via `maturin`. The
//! Python module is named `ohd_storage`; import it like:
//!
//! ```python
//! import ohd_storage
//! storage = ohd_storage.OhdStorage.create(path="/tmp/ohd.db", key_hex="")
//! storage.user_ulid()
//! ```
//!
//! # Surface
//!
//! - [`OhdStorage`] (`#[pyclass]`) — handle to one open per-user storage file.
//!   Constructed via `OhdStorage.create(...)` / `OhdStorage.open(...)` factory
//!   classmethods. Methods: `path()`, `user_ulid()`,
//!   `issue_self_session_token()`, `put_event(EventInputDto)`,
//!   `query_events(EventFilterDto)`, `format_version()`,
//!   `protocol_version()`.
//! - [`EventInputDto`] / [`EventFilterDto`] / [`EventDto`] /
//!   [`PutEventOutcomeDto`] / [`ChannelValueDto`] (`#[pyclass]`) — DTOs
//!   crossing the FFI boundary. All have `__init__` (with keyword-only
//!   defaults) and `__repr__` for debugging.
//! - [`ValueKind`] (`#[pyclass(eq, eq_int)]`) — discriminant for
//!   [`ChannelValueDto`].
//! - Exception hierarchy under `OhdError` (a `RuntimeError` subclass) with
//!   five concrete subclasses: `OpenFailed`, `Auth`, `InvalidInput`,
//!   `NotFound`, `Internal`. Mirrors the uniffi enum collapse from
//!   `ohd_storage_core::Error`.
//!
//! # GIL handling
//!
//! Long-running ops (`put_event`, `query_events`, `issue_self_session_token`)
//! release the GIL via `Python::detach` (formerly `Python::allow_threads`
//! pre-pyo3-0.28) so other Python threads can run while SQLite is busy. The
//! SQLite mutex inside `Storage` makes this safe.
//!
//! # Why a separate file (not in `lib.rs`)
//!
//! Keeps the uniffi macro expansion (`setup_scaffolding!`) and the PyO3
//! `#[pymodule]` expansion textually separate. Both compile in the same
//! cdylib — uniffi exports `_uniffi_*` symbols, pyo3 exports `PyInit_*` —
//! they don't collide.

use std::path::PathBuf;
use std::sync::Arc;

use pyo3::create_exception;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyType;

use ohd_storage_core as core;

// =============================================================================
// Exceptions
// =============================================================================
//
// `create_exception!` generates a Python exception class that subclasses the
// supplied parent. We root the OHD hierarchy at a single `OhdError`
// (subclass of `RuntimeError`) so callers can `except ohd_storage.OhdError:`
// to catch any storage error.
//
// The mapping from `core::Error` follows the same collapse used by the
// uniffi facade (see `OhdError` enum in `lib.rs`): I/O / SQLite open
// problems are `OpenFailed`; auth is `Auth`; validation/registry/JSON is
// `InvalidInput`; missing rows is `NotFound`; everything else is `Internal`.

create_exception!(ohd_storage, OhdError, PyRuntimeError);
create_exception!(ohd_storage, OpenFailed, OhdError);
create_exception!(ohd_storage, Auth, OhdError);
create_exception!(ohd_storage, InvalidInput, OhdError);
create_exception!(ohd_storage, NotFound, OhdError);
create_exception!(ohd_storage, Internal, OhdError);

fn map_core_error(e: core::Error) -> PyErr {
    let code = e.code().to_string();
    let message = e.to_string();
    use core::Error as E;
    match e {
        E::Io(_) | E::Sqlite(_) => OpenFailed::new_err(message),
        E::Unauthenticated
        | E::TokenExpired
        | E::TokenRevoked
        | E::WrongTokenKind(_)
        | E::OutOfScope
        | E::ApprovalTimeout => Auth::new_err(format!("{code}: {message}")),
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
        | E::Json(_) => InvalidInput::new_err(format!("{code}: {message}")),
        E::NotFound | E::CaseNotFound | E::EventDeleted => NotFound::new_err(message),
        _ => Internal::new_err(format!("{code}: {message}")),
    }
}

fn invalid_input(code: &str, message: impl Into<String>) -> PyErr {
    InvalidInput::new_err(format!("{code}: {}", message.into()))
}

// =============================================================================
// ValueKind
// =============================================================================

/// Discriminant for [`ChannelValueDto`]. One of `REAL` / `INT` / `BOOL` /
/// `TEXT` / `ENUM_ORDINAL`.
///
/// In Python:
///
/// ```python
/// import ohd_storage
/// vk = ohd_storage.ValueKind.REAL
/// assert vk == ohd_storage.ValueKind.REAL
/// ```
// `ENUM_ORDINAL` is upper-snake to mirror the wire-side proto3 enum names
// (`REAL` / `INT` / `BOOL` / `TEXT` / `ENUM_ORDINAL`). Python convention is
// upper-snake for enum values too, so this matches both ecosystems.
// `from_py_object` opts into the `FromPyObject` derive so the variant can
// round-trip from a Python `ValueKind.REAL` value when used as a method arg.
#[pyclass(module = "ohd_storage", eq, eq_int, from_py_object)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum ValueKind {
    /// `f64`.
    REAL,
    /// `i64`.
    INT,
    /// `bool`.
    BOOL,
    /// `String`.
    TEXT,
    /// Append-only enum ordinal.
    ENUM_ORDINAL,
}

#[pymethods]
impl ValueKind {
    fn __repr__(&self) -> &'static str {
        match self {
            ValueKind::REAL => "ValueKind.REAL",
            ValueKind::INT => "ValueKind.INT",
            ValueKind::BOOL => "ValueKind.BOOL",
            ValueKind::TEXT => "ValueKind.TEXT",
            ValueKind::ENUM_ORDINAL => "ValueKind.ENUM_ORDINAL",
        }
    }
}

// =============================================================================
// ChannelValueDto
// =============================================================================

/// A typed channel scalar, tagged by [`ValueKind`].
///
/// Exactly one of `real_value` / `int_value` / `bool_value` / `text_value` /
/// `enum_ordinal` should be set, matching `value_kind`. Mirrors the uniffi
/// `ChannelValueDto`.
///
/// ```python
/// import ohd_storage
/// cv = ohd_storage.ChannelValueDto(
///     channel_path="value",
///     value_kind=ohd_storage.ValueKind.REAL,
///     real_value=6.4,
/// )
/// ```
#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone)]
pub struct ChannelValueDto {
    pub channel_path: String,
    pub value_kind: ValueKind,
    pub real_value: Option<f64>,
    pub int_value: Option<i64>,
    pub bool_value: Option<bool>,
    pub text_value: Option<String>,
    pub enum_ordinal: Option<i32>,
}

#[pymethods]
impl ChannelValueDto {
    #[new]
    #[pyo3(signature = (
        channel_path,
        value_kind,
        *,
        real_value = None,
        int_value = None,
        bool_value = None,
        text_value = None,
        enum_ordinal = None,
    ))]
    fn new(
        channel_path: String,
        value_kind: ValueKind,
        real_value: Option<f64>,
        int_value: Option<i64>,
        bool_value: Option<bool>,
        text_value: Option<String>,
        enum_ordinal: Option<i32>,
    ) -> Self {
        Self {
            channel_path,
            value_kind,
            real_value,
            int_value,
            bool_value,
            text_value,
            enum_ordinal,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ChannelValueDto(channel_path={:?}, value_kind={:?}, real_value={:?}, int_value={:?}, bool_value={:?}, text_value={:?}, enum_ordinal={:?})",
            self.channel_path,
            self.value_kind,
            self.real_value,
            self.int_value,
            self.bool_value,
            self.text_value,
            self.enum_ordinal,
        )
    }
}

impl ChannelValueDto {
    fn into_core(self) -> PyResult<core::events::ChannelValue> {
        use core::events::{ChannelScalar, ChannelValue};
        let value = match self.value_kind {
            ValueKind::REAL => ChannelScalar::Real {
                real_value: self.real_value.ok_or_else(|| {
                    invalid_input(
                        "WRONG_VALUE_TYPE",
                        format!(
                            "channel {} marked REAL but no real_value",
                            self.channel_path
                        ),
                    )
                })?,
            },
            ValueKind::INT => ChannelScalar::Int {
                int_value: self.int_value.ok_or_else(|| {
                    invalid_input(
                        "WRONG_VALUE_TYPE",
                        format!("channel {} marked INT but no int_value", self.channel_path),
                    )
                })?,
            },
            ValueKind::BOOL => ChannelScalar::Bool {
                bool_value: self.bool_value.ok_or_else(|| {
                    invalid_input(
                        "WRONG_VALUE_TYPE",
                        format!(
                            "channel {} marked BOOL but no bool_value",
                            self.channel_path
                        ),
                    )
                })?,
            },
            ValueKind::TEXT => ChannelScalar::Text {
                text_value: self.text_value.clone().ok_or_else(|| {
                    invalid_input(
                        "WRONG_VALUE_TYPE",
                        format!(
                            "channel {} marked TEXT but no text_value",
                            self.channel_path
                        ),
                    )
                })?,
            },
            ValueKind::ENUM_ORDINAL => ChannelScalar::EnumOrdinal {
                enum_ordinal: self.enum_ordinal.ok_or_else(|| {
                    invalid_input(
                        "WRONG_VALUE_TYPE",
                        format!(
                            "channel {} marked ENUM but no enum_ordinal",
                            self.channel_path
                        ),
                    )
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
            value_kind: ValueKind::REAL,
            real_value: None,
            int_value: None,
            bool_value: None,
            text_value: None,
            enum_ordinal: None,
        };
        match v.value {
            ChannelScalar::Real { real_value } => {
                out.value_kind = ValueKind::REAL;
                out.real_value = Some(real_value);
            }
            ChannelScalar::Int { int_value } => {
                out.value_kind = ValueKind::INT;
                out.int_value = Some(int_value);
            }
            ChannelScalar::Bool { bool_value } => {
                out.value_kind = ValueKind::BOOL;
                out.bool_value = Some(bool_value);
            }
            ChannelScalar::Text { text_value } => {
                out.value_kind = ValueKind::TEXT;
                out.text_value = Some(text_value);
            }
            ChannelScalar::EnumOrdinal { enum_ordinal } => {
                out.value_kind = ValueKind::ENUM_ORDINAL;
                out.enum_ordinal = Some(enum_ordinal);
            }
        }
        out
    }
}

// =============================================================================
// EventInputDto
// =============================================================================

/// Sparse event input. Mirrors the uniffi `EventInputDto`. Sample blocks are
/// not yet exposed across the binding boundary (see lib.rs comment); for
/// bulk-time-series the Connect-RPC `PutEvents` is the path.
#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone)]
pub struct EventInputDto {
    pub timestamp_ms: i64,
    pub duration_ms: Option<i64>,
    pub tz_offset_minutes: Option<i32>,
    pub tz_name: Option<String>,
    pub event_type: String,
    pub channels: Vec<ChannelValueDto>,
    pub device_id: Option<String>,
    pub app_name: Option<String>,
    pub app_version: Option<String>,
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub notes: Option<String>,
}

#[pymethods]
impl EventInputDto {
    #[new]
    #[pyo3(signature = (
        timestamp_ms,
        event_type,
        channels,
        *,
        duration_ms = None,
        tz_offset_minutes = None,
        tz_name = None,
        device_id = None,
        app_name = None,
        app_version = None,
        source = None,
        source_id = None,
        notes = None,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        timestamp_ms: i64,
        event_type: String,
        channels: Vec<ChannelValueDto>,
        duration_ms: Option<i64>,
        tz_offset_minutes: Option<i32>,
        tz_name: Option<String>,
        device_id: Option<String>,
        app_name: Option<String>,
        app_version: Option<String>,
        source: Option<String>,
        source_id: Option<String>,
        notes: Option<String>,
    ) -> Self {
        Self {
            timestamp_ms,
            duration_ms,
            tz_offset_minutes,
            tz_name,
            event_type,
            channels,
            device_id,
            app_name,
            app_version,
            source,
            source_id,
            notes,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "EventInputDto(timestamp_ms={}, event_type={:?}, channels=[{} items])",
            self.timestamp_ms,
            self.event_type,
            self.channels.len()
        )
    }
}

impl EventInputDto {
    fn into_core(self) -> PyResult<core::events::EventInput> {
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
            // See the lib.rs uniffi comment — sample blocks need a future
            // pass; today the bindings carry channel scalars only.
            sample_blocks: vec![],
            // Source signing not exposed through the binding boundary today;
            // signed-write callers go through the in-process API.
            source_signature: None,
        })
    }
}

// =============================================================================
// EventDto
// =============================================================================

/// One stored event, returned by `query_events`.
#[pyclass(module = "ohd_storage", get_all, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct EventDto {
    pub ulid: String,
    pub timestamp_ms: i64,
    pub duration_ms: Option<i64>,
    pub event_type: String,
    pub channels: Vec<ChannelValueDto>,
    pub notes: Option<String>,
    pub source: Option<String>,
    pub deleted_at_ms: Option<i64>,
}

#[pymethods]
impl EventDto {
    fn __repr__(&self) -> String {
        format!(
            "EventDto(ulid={:?}, event_type={:?}, timestamp_ms={}, channels=[{} items])",
            self.ulid,
            self.event_type,
            self.timestamp_ms,
            self.channels.len()
        )
    }
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
        }
    }
}

// =============================================================================
// PutEventOutcomeDto
// =============================================================================

/// Outcome of a single `put_event` call.
///
/// Tagged by `outcome` (`"committed"` / `"pending"` / `"error"`). For
/// `committed` and `pending`, `ulid` is set; for `error`, `error_code` and
/// `error_message` carry the reason.
#[pyclass(module = "ohd_storage", get_all, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct PutEventOutcomeDto {
    pub outcome: String,
    pub ulid: String,
    pub timestamp_ms: i64,
    pub error_code: String,
    pub error_message: String,
}

#[pymethods]
impl PutEventOutcomeDto {
    fn __repr__(&self) -> String {
        format!(
            "PutEventOutcomeDto(outcome={:?}, ulid={:?}, timestamp_ms={}, error_code={:?})",
            self.outcome, self.ulid, self.timestamp_ms, self.error_code
        )
    }
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

// =============================================================================
// EventFilterDto
// =============================================================================

/// Filter for `query_events`. Mirrors the uniffi `EventFilterDto` (subset of
/// the full OHDC `EventFilter`; the deeper predicate language stays
/// server-side for now).
#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone, Default)]
pub struct EventFilterDto {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub event_types_in: Vec<String>,
    pub event_types_not_in: Vec<String>,
    pub include_deleted: bool,
    pub limit: Option<i64>,
}

#[pymethods]
impl EventFilterDto {
    #[new]
    #[pyo3(signature = (
        *,
        from_ms = None,
        to_ms = None,
        event_types_in = vec![],
        event_types_not_in = vec![],
        include_deleted = false,
        limit = None,
    ))]
    fn new(
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        event_types_in: Vec<String>,
        event_types_not_in: Vec<String>,
        include_deleted: bool,
        limit: Option<i64>,
    ) -> Self {
        Self {
            from_ms,
            to_ms,
            event_types_in,
            event_types_not_in,
            include_deleted,
            limit,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "EventFilterDto(from_ms={:?}, to_ms={:?}, event_types_in={:?}, event_types_not_in={:?}, include_deleted={}, limit={:?})",
            self.from_ms,
            self.to_ms,
            self.event_types_in,
            self.event_types_not_in,
            self.include_deleted,
            self.limit
        )
    }
}

impl EventFilterDto {
    fn into_core(self) -> core::events::EventFilter {
        core::events::EventFilter {
            from_ms: self.from_ms,
            to_ms: self.to_ms,
            event_types_in: self.event_types_in,
            event_types_not_in: self.event_types_not_in,
            include_deleted: self.include_deleted,
            include_superseded: true,
            limit: self.limit,
            device_id_in: vec![],
            source_in: vec![],
            event_ulids_in: vec![],
            sensitivity_classes_in: vec![],
            sensitivity_classes_not_in: vec![],
            channel_predicates: vec![],
            case_ulids_in: vec![],
        }
    }
}

// =============================================================================
// Grants
// =============================================================================

/// Filter for [`OhdStorage::list_grants`].
#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone, Default)]
pub struct ListGrantsFilterDto {
    pub include_revoked: bool,
    pub include_expired: bool,
    pub grantee_kind: Option<String>,
    pub limit: Option<i64>,
}

#[pymethods]
impl ListGrantsFilterDto {
    #[new]
    #[pyo3(signature = (
        *,
        include_revoked = false,
        include_expired = false,
        grantee_kind = None,
        limit = None,
    ))]
    fn new(
        include_revoked: bool,
        include_expired: bool,
        grantee_kind: Option<String>,
        limit: Option<i64>,
    ) -> Self {
        Self {
            include_revoked,
            include_expired,
            grantee_kind,
            limit,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ListGrantsFilterDto(include_revoked={}, include_expired={}, grantee_kind={:?}, limit={:?})",
            self.include_revoked, self.include_expired, self.grantee_kind, self.limit,
        )
    }
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

/// Per-event-type rule.
#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone)]
pub struct GrantEventTypeRuleDto {
    pub event_type: String,
    pub effect: String,
}

#[pymethods]
impl GrantEventTypeRuleDto {
    #[new]
    fn new(event_type: String, effect: String) -> Self {
        Self { event_type, effect }
    }

    fn __repr__(&self) -> String {
        format!(
            "GrantEventTypeRuleDto(event_type={:?}, effect={:?})",
            self.event_type, self.effect
        )
    }
}

/// Per-channel rule.
#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone)]
pub struct GrantChannelRuleDto {
    pub event_type: String,
    pub channel_path: String,
    pub effect: String,
}

#[pymethods]
impl GrantChannelRuleDto {
    #[new]
    fn new(event_type: String, channel_path: String, effect: String) -> Self {
        Self {
            event_type,
            channel_path,
            effect,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "GrantChannelRuleDto(event_type={:?}, channel_path={:?}, effect={:?})",
            self.event_type, self.channel_path, self.effect
        )
    }
}

/// Per-sensitivity-class rule.
#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone)]
pub struct GrantSensitivityRuleDto {
    pub sensitivity_class: String,
    pub effect: String,
}

#[pymethods]
impl GrantSensitivityRuleDto {
    #[new]
    fn new(sensitivity_class: String, effect: String) -> Self {
        Self {
            sensitivity_class,
            effect,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "GrantSensitivityRuleDto(sensitivity_class={:?}, effect={:?})",
            self.sensitivity_class, self.effect
        )
    }
}

/// Materialized grant row.
#[pyclass(module = "ohd_storage", get_all, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct GrantDto {
    pub ulid: String,
    pub grantee_label: String,
    pub grantee_kind: String,
    pub purpose: Option<String>,
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
    pub default_action: String,
    pub approval_mode: String,
    pub aggregation_only: bool,
    pub strip_notes: bool,
    pub notify_on_access: bool,
    pub event_type_rules: Vec<GrantEventTypeRuleDto>,
    pub channel_rules: Vec<GrantChannelRuleDto>,
    pub sensitivity_rules: Vec<GrantSensitivityRuleDto>,
    pub auto_approve_event_types: Vec<String>,
}

#[pymethods]
impl GrantDto {
    fn __repr__(&self) -> String {
        format!(
            "GrantDto(ulid={:?}, grantee_label={:?}, grantee_kind={:?}, revoked_at_ms={:?})",
            self.ulid, self.grantee_label, self.grantee_kind, self.revoked_at_ms
        )
    }
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

/// Sparse builder for [`OhdStorage::create_grant`] /
/// `issue_retrospective_grant`.
#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone)]
pub struct CreateGrantInputDto {
    pub grantee_label: String,
    pub grantee_kind: String,
    pub purpose: Option<String>,
    pub default_action: String,
    pub approval_mode: String,
    pub expires_at_ms: Option<i64>,
    pub event_type_rules: Vec<GrantEventTypeRuleDto>,
    pub channel_rules: Vec<GrantChannelRuleDto>,
    pub sensitivity_rules: Vec<GrantSensitivityRuleDto>,
    pub write_event_type_rules: Vec<GrantEventTypeRuleDto>,
    pub auto_approve_event_types: Vec<String>,
    pub aggregation_only: bool,
    pub strip_notes: bool,
    pub notify_on_access: bool,
}

#[pymethods]
impl CreateGrantInputDto {
    #[new]
    #[pyo3(signature = (
        grantee_label,
        grantee_kind,
        *,
        purpose = None,
        default_action = String::from("deny"),
        approval_mode = String::from("always"),
        expires_at_ms = None,
        event_type_rules = vec![],
        channel_rules = vec![],
        sensitivity_rules = vec![],
        write_event_type_rules = vec![],
        auto_approve_event_types = vec![],
        aggregation_only = false,
        strip_notes = false,
        notify_on_access = false,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        grantee_label: String,
        grantee_kind: String,
        purpose: Option<String>,
        default_action: String,
        approval_mode: String,
        expires_at_ms: Option<i64>,
        event_type_rules: Vec<GrantEventTypeRuleDto>,
        channel_rules: Vec<GrantChannelRuleDto>,
        sensitivity_rules: Vec<GrantSensitivityRuleDto>,
        write_event_type_rules: Vec<GrantEventTypeRuleDto>,
        auto_approve_event_types: Vec<String>,
        aggregation_only: bool,
        strip_notes: bool,
        notify_on_access: bool,
    ) -> Self {
        Self {
            grantee_label,
            grantee_kind,
            purpose,
            default_action,
            approval_mode,
            expires_at_ms,
            event_type_rules,
            channel_rules,
            sensitivity_rules,
            write_event_type_rules,
            auto_approve_event_types,
            aggregation_only,
            strip_notes,
            notify_on_access,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "CreateGrantInputDto(grantee_label={:?}, grantee_kind={:?}, default_action={:?})",
            self.grantee_label, self.grantee_kind, self.default_action
        )
    }
}

impl CreateGrantInputDto {
    fn into_core(self) -> PyResult<core::grants::NewGrant> {
        let default_action = match self.default_action.as_str() {
            "allow" => core::grants::RuleEffect::Allow,
            "deny" | "" => core::grants::RuleEffect::Deny,
            other => {
                return Err(invalid_input(
                    "INVALID_ARGUMENT",
                    format!("default_action must be 'allow' | 'deny'; got {other:?}"),
                ))
            }
        };
        let approval_mode = if self.approval_mode.is_empty() {
            "always".to_string()
        } else {
            self.approval_mode
        };
        Ok(core::grants::NewGrant {
            grantee_label: self.grantee_label,
            grantee_kind: self.grantee_kind,
            delegate_for_user_ulid: None,
            purpose: self.purpose,
            default_action,
            approval_mode,
            expires_at_ms: self.expires_at_ms,
            event_type_rules: self
                .event_type_rules
                .into_iter()
                .map(|r| (r.event_type, core::grants::RuleEffect::parse(&r.effect)))
                .collect(),
            channel_rules: self
                .channel_rules
                .into_iter()
                .map(|c| core::grants::ChannelRuleSpec {
                    event_type: c.event_type,
                    channel_path: c.channel_path,
                    effect: core::grants::RuleEffect::parse(&c.effect),
                })
                .collect(),
            sensitivity_rules: self
                .sensitivity_rules
                .into_iter()
                .map(|r| {
                    (
                        r.sensitivity_class,
                        core::grants::RuleEffect::parse(&r.effect),
                    )
                })
                .collect(),
            write_event_type_rules: self
                .write_event_type_rules
                .into_iter()
                .map(|r| (r.event_type, core::grants::RuleEffect::parse(&r.effect)))
                .collect(),
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

/// `(grant_ulid, token, share_url)` triple.
#[pyclass(module = "ohd_storage", get_all, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct GrantTokenDto {
    pub grant_ulid: String,
    pub token: String,
    pub share_url: String,
}

#[pymethods]
impl GrantTokenDto {
    fn __repr__(&self) -> String {
        format!(
            "GrantTokenDto(grant_ulid={:?}, share_url={:?})",
            self.grant_ulid, self.share_url
        )
    }
}

/// Sparse update for [`OhdStorage::update_grant`].
#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone, Default)]
pub struct GrantUpdateDto {
    pub grantee_label: Option<String>,
    pub expires_at_ms: Option<i64>,
}

#[pymethods]
impl GrantUpdateDto {
    #[new]
    #[pyo3(signature = (*, grantee_label = None, expires_at_ms = None))]
    fn new(grantee_label: Option<String>, expires_at_ms: Option<i64>) -> Self {
        Self {
            grantee_label,
            expires_at_ms,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "GrantUpdateDto(grantee_label={:?}, expires_at_ms={:?})",
            self.grantee_label, self.expires_at_ms
        )
    }
}

// =============================================================================
// Pending events
// =============================================================================

#[pyclass(module = "ohd_storage", get_all, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct PendingEventDto {
    pub ulid: String,
    pub submitted_at_ms: i64,
    pub submitting_grant_ulid: Option<String>,
    pub status: String,
    pub reviewed_at_ms: Option<i64>,
    pub rejection_reason: Option<String>,
    pub expires_at_ms: i64,
    pub event: EventDto,
}

#[pymethods]
impl PendingEventDto {
    fn __repr__(&self) -> String {
        format!(
            "PendingEventDto(ulid={:?}, status={:?}, submitted_at_ms={})",
            self.ulid, self.status, self.submitted_at_ms
        )
    }
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

#[pyclass(module = "ohd_storage", get_all, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct CaseDto {
    pub ulid: String,
    pub case_type: String,
    pub case_label: Option<String>,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub parent_case_ulid: Option<String>,
    pub predecessor_case_ulid: Option<String>,
    pub opening_authority_grant_ulid: Option<String>,
    pub inactivity_close_after_h: Option<i32>,
    pub last_activity_at_ms: i64,
}

#[pymethods]
impl CaseDto {
    fn __repr__(&self) -> String {
        format!(
            "CaseDto(ulid={:?}, case_type={:?}, ended_at_ms={:?})",
            self.ulid, self.case_type, self.ended_at_ms
        )
    }
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

#[pyclass(module = "ohd_storage", get_all, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct CaseDetailDto {
    pub case: CaseDto,
    pub audit: Vec<AuditEntryDto>,
}

#[pymethods]
impl CaseDetailDto {
    fn __repr__(&self) -> String {
        format!(
            "CaseDetailDto(case={}, audit=[{} items])",
            self.case.ulid,
            self.audit.len()
        )
    }
}

// =============================================================================
// Audit
// =============================================================================

#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone, Default)]
pub struct AuditFilterDto {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub actor_type: Option<String>,
    pub action: Option<String>,
    pub result: Option<String>,
    pub limit: Option<i64>,
}

#[pymethods]
impl AuditFilterDto {
    #[new]
    #[pyo3(signature = (
        *,
        from_ms = None,
        to_ms = None,
        actor_type = None,
        action = None,
        result = None,
        limit = None,
    ))]
    fn new(
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        actor_type: Option<String>,
        action: Option<String>,
        result: Option<String>,
        limit: Option<i64>,
    ) -> Self {
        Self {
            from_ms,
            to_ms,
            actor_type,
            action,
            result,
            limit,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "AuditFilterDto(from_ms={:?}, to_ms={:?}, actor_type={:?}, action={:?}, result={:?}, limit={:?})",
            self.from_ms, self.to_ms, self.actor_type, self.action, self.result, self.limit
        )
    }
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

#[pyclass(module = "ohd_storage", get_all, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct AuditEntryDto {
    pub ts_ms: i64,
    pub actor_type: String,
    pub action: String,
    pub query_kind: Option<String>,
    pub query_params_json: Option<String>,
    pub rows_returned: Option<i64>,
    pub rows_filtered: Option<i64>,
    pub result: String,
    pub reason: Option<String>,
}

#[pymethods]
impl AuditEntryDto {
    fn __repr__(&self) -> String {
        format!(
            "AuditEntryDto(ts_ms={}, actor_type={:?}, action={:?}, result={:?})",
            self.ts_ms, self.actor_type, self.action, self.result
        )
    }
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

#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone)]
pub struct TrustedAuthorityDto {
    pub label: String,
    pub scope: Option<String>,
    pub public_key_pem: Option<String>,
    pub is_default: bool,
}

#[pymethods]
impl TrustedAuthorityDto {
    #[new]
    #[pyo3(signature = (label, *, scope = None, public_key_pem = None, is_default = false))]
    fn new(
        label: String,
        scope: Option<String>,
        public_key_pem: Option<String>,
        is_default: bool,
    ) -> Self {
        Self {
            label,
            scope,
            public_key_pem,
            is_default,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "TrustedAuthorityDto(label={:?}, scope={:?}, is_default={})",
            self.label, self.scope, self.is_default
        )
    }
}

#[pyclass(module = "ohd_storage", get_all, set_all, from_py_object)]
#[derive(Debug, Clone)]
pub struct EmergencyConfigDto {
    pub enabled: bool,
    pub bluetooth_beacon: bool,
    pub approval_timeout_seconds: i32,
    pub default_action_on_timeout: String,
    pub lock_screen_visibility: String,
    pub history_window_hours: i32,
    pub channel_paths_allowed: Vec<String>,
    pub sensitivity_classes_allowed: Vec<String>,
    pub share_location: bool,
    pub trusted_authorities: Vec<TrustedAuthorityDto>,
    pub bystander_proxy_enabled: bool,
    pub updated_at_ms: i64,
}

#[pymethods]
impl EmergencyConfigDto {
    #[new]
    #[pyo3(signature = (
        *,
        enabled = false,
        bluetooth_beacon = true,
        approval_timeout_seconds = 30,
        default_action_on_timeout = String::from("allow"),
        lock_screen_visibility = String::from("full"),
        history_window_hours = 24,
        channel_paths_allowed = vec![],
        sensitivity_classes_allowed = vec![],
        share_location = false,
        trusted_authorities = vec![],
        bystander_proxy_enabled = true,
        updated_at_ms = 0,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        enabled: bool,
        bluetooth_beacon: bool,
        approval_timeout_seconds: i32,
        default_action_on_timeout: String,
        lock_screen_visibility: String,
        history_window_hours: i32,
        channel_paths_allowed: Vec<String>,
        sensitivity_classes_allowed: Vec<String>,
        share_location: bool,
        trusted_authorities: Vec<TrustedAuthorityDto>,
        bystander_proxy_enabled: bool,
        updated_at_ms: i64,
    ) -> Self {
        Self {
            enabled,
            bluetooth_beacon,
            approval_timeout_seconds,
            default_action_on_timeout,
            lock_screen_visibility,
            history_window_hours,
            channel_paths_allowed,
            sensitivity_classes_allowed,
            share_location,
            trusted_authorities,
            bystander_proxy_enabled,
            updated_at_ms,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "EmergencyConfigDto(enabled={}, approval_timeout_seconds={}, history_window_hours={})",
            self.enabled, self.approval_timeout_seconds, self.history_window_hours
        )
    }
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

#[pyclass(module = "ohd_storage", get_all, skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct SignerDto {
    pub signer_kid: String,
    pub signer_label: String,
    pub sig_alg: String,
    pub public_key_pem: String,
    pub registered_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
}

#[pymethods]
impl SignerDto {
    fn __repr__(&self) -> String {
        format!(
            "SignerDto(signer_kid={:?}, sig_alg={:?}, revoked_at_ms={:?})",
            self.signer_kid, self.sig_alg, self.revoked_at_ms
        )
    }
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
// OhdStorage
// =============================================================================

/// Python handle to one open OHD Storage file.
///
/// Construct via [`OhdStorage::create`] or [`OhdStorage::open`] (both are
/// classmethods on the Python class). Methods are thread-safe — every call
/// serializes through the inner `Storage` mutex.
///
/// ```python
/// import ohd_storage
/// s = ohd_storage.OhdStorage.create(path="/tmp/ohd.db", key_hex="")
/// token = s.issue_self_session_token()
/// ulid = s.put_event(ohd_storage.EventInputDto(
///     timestamp_ms=1_700_000_000_000,
///     event_type="std.blood_glucose",
///     channels=[ohd_storage.ChannelValueDto(
///         channel_path="value",
///         value_kind=ohd_storage.ValueKind.REAL,
///         real_value=5.4,
///     )],
/// ))
/// rows = s.query_events(ohd_storage.EventFilterDto(from_ms=0, to_ms=2_000_000_000_000))
/// ```
#[pyclass(module = "ohd_storage")]
pub struct OhdStorage {
    inner: Arc<core::Storage>,
}

#[pymethods]
impl OhdStorage {
    /// Open an existing storage file. Raises `OpenFailed` if the file
    /// doesn't exist (use [`OhdStorage::create`] to first-create).
    #[classmethod]
    #[pyo3(signature = (path, key_hex = String::new()))]
    fn open(
        _cls: &Bound<'_, PyType>,
        py: Python<'_>,
        path: String,
        key_hex: String,
    ) -> PyResult<Self> {
        py.detach(|| Self::open_inner(path, key_hex, false))
    }

    /// Create-or-open a storage file. Stamps `_meta.user_ulid`,
    /// `format_version`, runs migrations.
    #[classmethod]
    #[pyo3(signature = (path, key_hex = String::new()))]
    fn create(
        _cls: &Bound<'_, PyType>,
        py: Python<'_>,
        path: String,
        key_hex: String,
    ) -> PyResult<Self> {
        py.detach(|| Self::open_inner(path, key_hex, true))
    }

    /// Backing file path.
    fn path(&self) -> String {
        self.inner.path().to_string_lossy().into_owned()
    }

    /// User ULID stamped into `_meta.user_ulid` (Crockford-base32).
    fn user_ulid(&self) -> String {
        core::ulid::to_crockford(&self.inner.user_ulid())
    }

    /// Mint a fresh self-session token (`ohds_…`). Cleartext returned exactly
    /// once — store it in your platform keystore.
    fn issue_self_session_token(&self, py: Python<'_>) -> PyResult<String> {
        let user_ulid = self.inner.user_ulid();
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            storage
                .with_conn(|conn| core::auth::issue_self_session_token(conn, user_ulid, None, None))
                .map_err(map_core_error)
        })
    }

    /// Write one event. Returns a [`PutEventOutcomeDto`].
    ///
    /// For bulk imports, call this in a loop or wait for the future
    /// `put_events_batch` deliverable (uniffi facade carries the same
    /// limitation).
    fn put_event(&self, py: Python<'_>, input: EventInputDto) -> PyResult<PutEventOutcomeDto> {
        let core_input = input.into_core()?;
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let envelope = storage.envelope_key().cloned();
            let mut results = storage
                .with_conn_mut(|conn| {
                    core::events::put_events(conn, &[core_input], None, false, envelope.as_ref())
                })
                .map_err(map_core_error)?;
            Ok(PutEventOutcomeDto::from_core(results.remove(0)))
        })
    }

    /// Read events under a filter. Returns rows in `TIME_DESC` order.
    fn query_events(&self, py: Python<'_>, filter: EventFilterDto) -> PyResult<Vec<EventDto>> {
        let core_filter = filter.into_core();
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let (events, _filtered) = storage
                .with_conn(|conn| core::events::query_events(conn, &core_filter, None))
                .map_err(map_core_error)?;
            Ok(events.into_iter().map(EventDto::from_core).collect())
        })
    }

    /// On-disk format version (e.g. `"1.0"`).
    fn format_version(&self) -> &'static str {
        core::FORMAT_VERSION
    }

    /// OHDC protocol version this binding's core implements
    /// (e.g. `"ohdc.v0"`).
    fn protocol_version(&self) -> &'static str {
        core::PROTOCOL_VERSION
    }

    fn __repr__(&self) -> String {
        format!(
            "OhdStorage(path={:?}, user_ulid={})",
            self.inner.path().to_string_lossy(),
            core::ulid::to_crockford(&self.inner.user_ulid())
        )
    }

    // -------------------------------------------------------------------------
    // Grants
    // -------------------------------------------------------------------------

    /// List grants. Returns a list of [`GrantDto`].
    fn list_grants(&self, py: Python<'_>, filter: ListGrantsFilterDto) -> PyResult<Vec<GrantDto>> {
        let core_filter = filter.into_core();
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let rows = storage
                .with_conn(|conn| core::grants::list_grants(conn, &core_filter))
                .map_err(map_core_error)?;
            Ok(rows.into_iter().map(GrantDto::from_core).collect())
        })
    }

    /// Create a grant. Returns `(grant_ulid, token, share_url)` packed in
    /// [`GrantTokenDto`].
    fn create_grant(&self, py: Python<'_>, req: CreateGrantInputDto) -> PyResult<GrantTokenDto> {
        let new_grant = req.into_core()?;
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let envelope = storage.envelope_key().cloned();
            let recovery = storage.recovery_keypair().cloned();
            let user_ulid = storage.user_ulid();
            let (grant_id, grant_ulid) = storage
                .with_conn_mut(|conn| match envelope.as_ref() {
                    Some(env) => core::grants::create_grant_with_envelope(
                        conn,
                        &new_grant,
                        env,
                        recovery.as_ref(),
                    ),
                    None => core::grants::create_grant(conn, &new_grant),
                })
                .map_err(map_core_error)?;
            let ttl_ms = new_grant
                .expires_at_ms
                .map(|exp| exp - core::format::now_ms());
            let token = storage
                .with_conn(|conn| {
                    core::auth::issue_grant_token(
                        conn,
                        user_ulid,
                        grant_id,
                        core::auth::TokenKind::Grant,
                        ttl_ms,
                    )
                })
                .map_err(map_core_error)?;
            Ok(GrantTokenDto {
                grant_ulid: core::ulid::to_crockford(&grant_ulid),
                token,
                share_url: format!("ohd://grant/{}", core::ulid::to_crockford(&grant_ulid)),
            })
        })
    }

    /// Revoke a grant by its ULID (Crockford).
    #[pyo3(signature = (grant_ulid, reason = None))]
    fn revoke_grant(
        &self,
        py: Python<'_>,
        grant_ulid: String,
        reason: Option<String>,
    ) -> PyResult<()> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let ulid_bytes = core::ulid::parse_crockford(&grant_ulid).map_err(map_core_error)?;
            let grant_id = storage
                .with_conn(|conn| core::grants::grant_id_by_ulid(conn, &ulid_bytes))
                .map_err(map_core_error)?;
            storage
                .with_conn(|conn| core::grants::revoke_grant(conn, grant_id, reason.as_deref()))
                .map_err(map_core_error)?;
            Ok(())
        })
    }

    /// Update a grant.
    fn update_grant(
        &self,
        py: Python<'_>,
        grant_ulid: String,
        update: GrantUpdateDto,
    ) -> PyResult<()> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let ulid_bytes = core::ulid::parse_crockford(&grant_ulid).map_err(map_core_error)?;
            let grant_id = storage
                .with_conn(|conn| core::grants::grant_id_by_ulid(conn, &ulid_bytes))
                .map_err(map_core_error)?;
            let core_update = core::grants::GrantUpdate {
                grantee_label: update.grantee_label,
                expires_at_ms: update.expires_at_ms,
            };
            storage
                .with_conn_mut(|conn| core::grants::update_grant(conn, grant_id, &core_update))
                .map_err(map_core_error)?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // Pending events
    // -------------------------------------------------------------------------

    /// List pending events.
    fn list_pending(&self, py: Python<'_>) -> PyResult<Vec<PendingEventDto>> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let filter = core::pending::ListPendingFilter::default();
            let rows = storage
                .with_conn(|conn| core::pending::list_pending(conn, &filter))
                .map_err(map_core_error)?;
            Ok(rows.into_iter().map(PendingEventDto::from_core).collect())
        })
    }

    /// Approve a pending event.
    #[pyo3(signature = (pending_ulid, also_auto_approve_event_type = false))]
    fn approve_pending(
        &self,
        py: Python<'_>,
        pending_ulid: String,
        also_auto_approve_event_type: bool,
    ) -> PyResult<()> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let ulid_bytes = core::ulid::parse_crockford(&pending_ulid).map_err(map_core_error)?;
            let envelope = storage.envelope_key().cloned();
            storage
                .with_conn_mut(|conn| {
                    core::pending::approve_pending(
                        conn,
                        &ulid_bytes,
                        also_auto_approve_event_type,
                        envelope.as_ref(),
                    )
                })
                .map_err(map_core_error)?;
            Ok(())
        })
    }

    /// Reject a pending event.
    #[pyo3(signature = (pending_ulid, reason = None))]
    fn reject_pending(
        &self,
        py: Python<'_>,
        pending_ulid: String,
        reason: Option<String>,
    ) -> PyResult<()> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let ulid_bytes = core::ulid::parse_crockford(&pending_ulid).map_err(map_core_error)?;
            storage
                .with_conn_mut(|conn| {
                    core::pending::reject_pending(conn, &ulid_bytes, reason.as_deref())
                })
                .map_err(map_core_error)?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // Cases
    // -------------------------------------------------------------------------

    /// List cases. Filter by `"open"` / `"closed"` / `None` (both).
    #[pyo3(signature = (state_filter = None))]
    fn list_cases(&self, py: Python<'_>, state_filter: Option<String>) -> PyResult<Vec<CaseDto>> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let include_closed = !matches!(state_filter.as_deref(), Some("open"));
            let filter = core::cases::ListCasesFilter {
                include_closed,
                ..Default::default()
            };
            let rows = storage
                .with_conn(|conn| core::cases::list_cases(conn, &filter))
                .map_err(map_core_error)?;
            let mut out: Vec<CaseDto> = rows.into_iter().map(CaseDto::from_core).collect();
            if matches!(state_filter.as_deref(), Some("closed")) {
                out.retain(|c| c.ended_at_ms.is_some());
            }
            Ok(out)
        })
    }

    /// Get one case + its recent audit entries.
    fn get_case(&self, py: Python<'_>, case_ulid: String) -> PyResult<CaseDetailDto> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let ulid_bytes = core::ulid::parse_crockford(&case_ulid).map_err(map_core_error)?;
            let case_id = storage
                .with_conn(|conn| core::cases::case_id_by_ulid(conn, &ulid_bytes))
                .map_err(map_core_error)?;
            let case = storage
                .with_conn(|conn| core::cases::read_case(conn, case_id))
                .map_err(map_core_error)?;
            let mut audit_entries: Vec<AuditEntryDto> = Vec::new();
            if let Some(authority_ulid) = case.opening_authority_grant_ulid.as_ref() {
                let gid = storage
                    .with_conn(|conn| core::grants::grant_id_by_ulid(conn, authority_ulid))
                    .map_err(map_core_error)?;
                let q = core::audit::AuditQuery {
                    grant_id: Some(gid),
                    limit: Some(50),
                    ..Default::default()
                };
                let rows = storage
                    .with_conn(|conn| core::audit::query(conn, &q))
                    .map_err(map_core_error)?;
                audit_entries = rows.into_iter().map(AuditEntryDto::from_core).collect();
            }
            Ok(CaseDetailDto {
                case: CaseDto::from_core(case),
                audit: audit_entries,
            })
        })
    }

    /// Force-close a case (self-session). Idempotent.
    fn force_close_case(&self, py: Python<'_>, case_ulid: String) -> PyResult<()> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let ulid_bytes = core::ulid::parse_crockford(&case_ulid).map_err(map_core_error)?;
            let case_id = storage
                .with_conn(|conn| core::cases::case_id_by_ulid(conn, &ulid_bytes))
                .map_err(map_core_error)?;
            storage
                .with_conn_mut(|conn| core::cases::close_case(conn, case_id, None, false, None))
                .map_err(map_core_error)?;
            Ok(())
        })
    }

    /// Issue a retrospective grant against an existing case.
    fn issue_retrospective_grant(
        &self,
        py: Python<'_>,
        case_ulid: String,
        req: CreateGrantInputDto,
    ) -> PyResult<GrantTokenDto> {
        let storage = Arc::clone(&self.inner);
        let new_grant = req.into_core()?;
        py.detach(|| {
            let ulid_bytes = core::ulid::parse_crockford(&case_ulid).map_err(map_core_error)?;
            let case_id = storage
                .with_conn(|conn| core::cases::case_id_by_ulid(conn, &ulid_bytes))
                .map_err(map_core_error)?;
            let envelope = storage.envelope_key().cloned();
            let recovery = storage.recovery_keypair().cloned();
            let user_ulid = storage.user_ulid();
            let (grant_id, grant_ulid) = storage
                .with_conn_mut(|conn| match envelope.as_ref() {
                    Some(env) => core::grants::create_grant_with_envelope(
                        conn,
                        &new_grant,
                        env,
                        recovery.as_ref(),
                    ),
                    None => core::grants::create_grant(conn, &new_grant),
                })
                .map_err(map_core_error)?;
            storage
                .with_conn(|conn| core::cases::bind_grant_to_cases(conn, grant_id, &[case_id]))
                .map_err(map_core_error)?;
            let ttl_ms = new_grant
                .expires_at_ms
                .map(|exp| exp - core::format::now_ms());
            let token = storage
                .with_conn(|conn| {
                    core::auth::issue_grant_token(
                        conn,
                        user_ulid,
                        grant_id,
                        core::auth::TokenKind::Grant,
                        ttl_ms,
                    )
                })
                .map_err(map_core_error)?;
            Ok(GrantTokenDto {
                grant_ulid: core::ulid::to_crockford(&grant_ulid),
                token,
                share_url: format!("ohd://grant/{}", core::ulid::to_crockford(&grant_ulid)),
            })
        })
    }

    // -------------------------------------------------------------------------
    // Audit
    // -------------------------------------------------------------------------

    /// Run an audit query.
    fn audit_query(&self, py: Python<'_>, filter: AuditFilterDto) -> PyResult<Vec<AuditEntryDto>> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let q = filter.into_core();
            let rows = storage
                .with_conn(|conn| core::audit::query(conn, &q))
                .map_err(map_core_error)?;
            Ok(rows.into_iter().map(AuditEntryDto::from_core).collect())
        })
    }

    // -------------------------------------------------------------------------
    // Emergency config
    // -------------------------------------------------------------------------

    /// Read the user's emergency configuration.
    fn get_emergency_config(&self, py: Python<'_>) -> PyResult<EmergencyConfigDto> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let user = storage.user_ulid();
            let cfg = storage
                .with_conn(|conn| core::emergency_config::get_emergency_config(conn, user))
                .map_err(map_core_error)?;
            Ok(EmergencyConfigDto::from_core(cfg))
        })
    }

    /// Replace the user's emergency configuration.
    fn set_emergency_config(&self, py: Python<'_>, cfg: EmergencyConfigDto) -> PyResult<()> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let user = storage.user_ulid();
            let core_cfg = cfg.into_core();
            let now = core::format::now_ms();
            storage
                .with_conn(|conn| {
                    core::emergency_config::set_emergency_config(conn, user, &core_cfg, now)
                })
                .map_err(map_core_error)?;
            Ok(())
        })
    }

    // -------------------------------------------------------------------------
    // Export
    // -------------------------------------------------------------------------

    /// Export the storage to a CBOR-encoded byte buffer.
    fn export_all<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyBytes>> {
        let storage = Arc::clone(&self.inner);
        let bytes = py.detach(|| {
            let user = storage.user_ulid();
            let token = core::auth::ResolvedToken {
                kind: core::auth::TokenKind::SelfSession,
                user_ulid: user,
                grant_id: None,
                grant_ulid: None,
                grantee_label: None,
                delegate_for_user_ulid: None,
            };
            core::ohdc::export_all(&storage, &token, None, None, &[]).map_err(map_core_error)
        })?;
        Ok(pyo3::types::PyBytes::new(py, &bytes))
    }

    // -------------------------------------------------------------------------
    // Source signing (operator registry)
    // -------------------------------------------------------------------------

    /// Register a source signer.
    fn register_signer(
        &self,
        py: Python<'_>,
        signer_kid: String,
        signer_label: String,
        sig_alg: String,
        public_key_pem: String,
    ) -> PyResult<SignerDto> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let signer = storage
                .with_conn(|conn| {
                    core::source_signing::register_signer(
                        conn,
                        &signer_kid,
                        &signer_label,
                        &sig_alg,
                        &public_key_pem,
                    )
                })
                .map_err(map_core_error)?;
            Ok(SignerDto::from_core(signer))
        })
    }

    /// List all registered signers.
    fn list_signers(&self, py: Python<'_>) -> PyResult<Vec<SignerDto>> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            let rows = storage
                .with_conn(|conn| core::source_signing::list_signers(conn))
                .map_err(map_core_error)?;
            Ok(rows.into_iter().map(SignerDto::from_core).collect())
        })
    }

    /// Revoke a signer by KID.
    fn revoke_signer(&self, py: Python<'_>, signer_kid: String) -> PyResult<()> {
        let storage = Arc::clone(&self.inner);
        py.detach(|| {
            storage
                .with_conn(|conn| core::source_signing::revoke_signer(conn, &signer_kid))
                .map_err(map_core_error)?;
            Ok(())
        })
    }
}

impl OhdStorage {
    fn open_inner(path: String, key_hex: String, create: bool) -> PyResult<Self> {
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
        let storage = core::Storage::open(cfg).map_err(map_core_error)?;
        Ok(Self {
            inner: Arc::new(storage),
        })
    }
}

// =============================================================================
// Top-level helpers
// =============================================================================

/// Build version of the storage core packed into this wheel.
#[pyfunction]
fn storage_version() -> &'static str {
    core::STORAGE_VERSION
}

/// OHDC protocol version this wheel's core implements.
#[pyfunction]
fn protocol_version() -> &'static str {
    core::PROTOCOL_VERSION
}

/// On-disk format version this wheel's core understands.
#[pyfunction]
fn format_version() -> &'static str {
    core::FORMAT_VERSION
}

// =============================================================================
// hex helpers
// =============================================================================

fn hex_decode(s: &str) -> PyResult<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err(invalid_input(
            "INVALID_ARGUMENT",
            "key_hex must have even length",
        ));
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

fn hex_nibble(c: u8) -> PyResult<u8> {
    Ok(match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => {
            return Err(invalid_input(
                "INVALID_ARGUMENT",
                format!("non-hex char in key_hex: {}", c as char),
            ))
        }
    })
}

// =============================================================================
// pymodule entrypoint
// =============================================================================

/// PyO3 module entrypoint. Maturin uses the function name (`ohd_storage`) as
/// the Python module name. Cargo's `[lib].name` (`ohd_storage_bindings`)
/// names the cdylib symbol; `pyproject.toml`'s `[tool.maturin].module-name`
/// resolves the wheel name back to `ohd_storage`.
#[pymodule]
fn ohd_storage(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();

    // Classes.
    m.add_class::<OhdStorage>()?;
    m.add_class::<EventInputDto>()?;
    m.add_class::<EventFilterDto>()?;
    m.add_class::<EventDto>()?;
    m.add_class::<PutEventOutcomeDto>()?;
    m.add_class::<ChannelValueDto>()?;
    m.add_class::<ValueKind>()?;

    // Grant DTOs.
    m.add_class::<ListGrantsFilterDto>()?;
    m.add_class::<GrantEventTypeRuleDto>()?;
    m.add_class::<GrantChannelRuleDto>()?;
    m.add_class::<GrantSensitivityRuleDto>()?;
    m.add_class::<GrantDto>()?;
    m.add_class::<CreateGrantInputDto>()?;
    m.add_class::<GrantTokenDto>()?;
    m.add_class::<GrantUpdateDto>()?;

    // Pending events / cases / audit / emergency / signers.
    m.add_class::<PendingEventDto>()?;
    m.add_class::<CaseDto>()?;
    m.add_class::<CaseDetailDto>()?;
    m.add_class::<AuditFilterDto>()?;
    m.add_class::<AuditEntryDto>()?;
    m.add_class::<TrustedAuthorityDto>()?;
    m.add_class::<EmergencyConfigDto>()?;
    m.add_class::<SignerDto>()?;

    // Top-level helpers.
    m.add_function(wrap_pyfunction!(storage_version, m)?)?;
    m.add_function(wrap_pyfunction!(protocol_version, m)?)?;
    m.add_function(wrap_pyfunction!(format_version, m)?)?;

    // Exception hierarchy. `OhdError` is the root; the five subclasses
    // mirror the uniffi `OhdError` enum variants.
    m.add("OhdError", py.get_type::<OhdError>())?;
    m.add("OpenFailed", py.get_type::<OpenFailed>())?;
    m.add("Auth", py.get_type::<Auth>())?;
    m.add("InvalidInput", py.get_type::<InvalidInput>())?;
    m.add("NotFound", py.get_type::<NotFound>())?;
    m.add("Internal", py.get_type::<Internal>())?;

    // Version constants.
    m.add("__version__", core::STORAGE_VERSION)?;
    m.add("PROTOCOL_VERSION", core::PROTOCOL_VERSION)?;
    m.add("FORMAT_VERSION", core::FORMAT_VERSION)?;

    Ok(())
}
