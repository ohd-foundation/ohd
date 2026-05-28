//! `RemoteOhdStorage` — a uniffi object that talks to a remote
//! `ohd-storage-server` over ConnectRPC.
//!
//! Phase 1 of the OHD Connect remote-storage feature. `RemoteOhdStorage`
//! mirrors the local [`OhdStorage`](crate::OhdStorage) read/write data ops
//! and returns the **same uniffi DTOs** (`EventDto`, `GrantDto`,
//! `EventFilterDto`, …). The Android `StorageRepository` can therefore swap
//! local↔remote behind a single interface with no DTO churn — that swap is
//! Phase 3.
//!
//! # Layering
//!
//! The networking lives in the `ohd-storage-client` crate ([`rc`]), which
//! hosts the connectrpc-generated `OhdcServiceClient` and an `async`
//! plain-Rust [`OhdcRemoteClient`](rc::OhdcRemoteClient). This module is the
//! thin uniffi facade: it owns a long-lived multi-thread `tokio` runtime
//! (mirroring [`remote_access`](crate::remote_access)), maps the binding's
//! DTOs ↔ the client crate's plain structs, and `block_on`s each RPC so the
//! uniffi surface stays synchronous — exactly as the local `query_events`
//! returns `Vec<EventDto>` rather than a stream.
//!
//! # Token refresh
//!
//! [`RemoteOhdStorage::set_bearer_token`] swaps the bearer without rebuilding
//! the object. When an RPC fails with [`OhdError::Auth`] whose `code` is
//! `TOKEN_EXPIRED`, the Android layer should mint a fresh self-session
//! token, call `set_bearer_token`, and retry the RPC once.

use std::sync::Arc;

use ohd_storage_client as rc;
use ohd_storage_core as core;
use tokio::runtime::Runtime;

use crate::{
    AuditEntryDto, AuditFilterDto, CaseDto, CaseStateDto, ChannelValueDto, CreateGrantInputDto,
    EventDto, EventFilterDto, EventInputDto, GrantChannelRuleDto, GrantDto, GrantEventTypeRuleDto,
    GrantSensitivityRuleDto, GrantTokenDto, ListGrantsFilterDto, OhdError, PendingEventDto,
    PutEventOutcomeDto, ValueKind,
};

type Result<T> = std::result::Result<T, OhdError>;

// =============================================================================
// Error mapping
// =============================================================================

impl From<rc::RemoteError> for OhdError {
    fn from(e: rc::RemoteError) -> Self {
        match e {
            rc::RemoteError::Transport { message } => OhdError::Internal {
                code: "UNAVAILABLE".to_string(),
                message,
            },
            rc::RemoteError::Auth { code, message } => OhdError::Auth { code, message },
            rc::RemoteError::InvalidInput { code, message } => {
                OhdError::InvalidInput { code, message }
            }
            rc::RemoteError::NotFound => OhdError::NotFound,
            rc::RemoteError::Internal { code, message } => OhdError::Internal { code, message },
        }
    }
}

// =============================================================================
// DTO ↔ client-struct conversions
// =============================================================================

/// Render a raw 16-byte ULID as Crockford-base32 for the DTO surface. An
/// empty / wrong-length input yields an empty string (the DTOs already model
/// "absent" that way for the optional-ULID fields).
fn ulid_to_crockford(bytes: &[u8]) -> String {
    if bytes.len() == 16 {
        let mut arr = [0u8; 16];
        arr.copy_from_slice(bytes);
        core::ulid::to_crockford(&arr)
    } else {
        String::new()
    }
}

/// Parse a Crockford-base32 ULID into raw 16 bytes.
fn ulid_from_crockford(s: &str) -> Result<Vec<u8>> {
    core::ulid::parse_crockford(s)
        .map(|a| a.to_vec())
        .map_err(|_| OhdError::InvalidInput {
            code: "INVALID_ULID".to_string(),
            message: format!("not a valid Crockford ULID: {s:?}"),
        })
}

fn channel_value_dto_to_rc(c: ChannelValueDto) -> rc::ChannelValue {
    rc::ChannelValue {
        channel_path: c.channel_path,
        value_kind: match c.value_kind {
            ValueKind::Real => rc::ValueKind::Real,
            ValueKind::Int => rc::ValueKind::Int,
            ValueKind::Bool => rc::ValueKind::Bool,
            ValueKind::Text => rc::ValueKind::Text,
            ValueKind::EnumOrdinal => rc::ValueKind::EnumOrdinal,
        },
        real_value: c.real_value,
        int_value: c.int_value,
        bool_value: c.bool_value,
        text_value: c.text_value,
        enum_ordinal: c.enum_ordinal,
    }
}

fn channel_value_rc_to_dto(c: rc::ChannelValue) -> ChannelValueDto {
    ChannelValueDto {
        channel_path: c.channel_path,
        value_kind: match c.value_kind {
            rc::ValueKind::Real => ValueKind::Real,
            rc::ValueKind::Int => ValueKind::Int,
            rc::ValueKind::Bool => ValueKind::Bool,
            rc::ValueKind::Text => ValueKind::Text,
            rc::ValueKind::EnumOrdinal => ValueKind::EnumOrdinal,
        },
        real_value: c.real_value,
        int_value: c.int_value,
        bool_value: c.bool_value,
        text_value: c.text_value,
        enum_ordinal: c.enum_ordinal,
    }
}

fn event_input_dto_to_rc(input: EventInputDto) -> rc::EventInput {
    rc::EventInput {
        timestamp_ms: input.timestamp_ms,
        duration_ms: input.duration_ms,
        tz_offset_minutes: input.tz_offset_minutes,
        tz_name: input.tz_name,
        event_type: input.event_type,
        channels: input
            .channels
            .into_iter()
            .map(channel_value_dto_to_rc)
            .collect(),
        device_id: input.device_id,
        app_name: input.app_name,
        app_version: input.app_version,
        source: input.source,
        source_id: input.source_id,
        notes: input.notes,
    }
}

fn event_rc_to_dto(e: rc::Event) -> EventDto {
    EventDto {
        ulid: ulid_to_crockford(&e.ulid),
        timestamp_ms: e.timestamp_ms,
        duration_ms: e.duration_ms,
        event_type: e.event_type,
        channels: e
            .channels
            .into_iter()
            .map(channel_value_rc_to_dto)
            .collect(),
        notes: e.notes,
        source: e.source,
        deleted_at_ms: e.deleted_at_ms,
        // The OHDC wire `Event` carries no `top_level` flag; remote rows are
        // surfaced as top-level (consistent with the server's write path,
        // which mints top-level events over the RPC surface).
        top_level: true,
    }
}

fn event_filter_dto_to_rc(f: EventFilterDto) -> rc::EventFilter {
    rc::EventFilter {
        from_ms: f.from_ms,
        to_ms: f.to_ms,
        event_types_in: f.event_types_in,
        event_types_not_in: f.event_types_not_in,
        include_deleted: f.include_deleted,
        limit: f.limit,
        source_in: f.source_in,
    }
}

fn put_outcome_rc_to_dto(o: rc::PutEventOutcome) -> PutEventOutcomeDto {
    PutEventOutcomeDto {
        outcome: o.outcome,
        ulid: ulid_to_crockford(&o.ulid),
        timestamp_ms: o.timestamp_ms,
        error_code: o.error_code,
        error_message: o.error_message,
    }
}

fn grant_rc_to_dto(g: rc::Grant) -> GrantDto {
    GrantDto {
        ulid: ulid_to_crockford(&g.ulid),
        grantee_label: g.grantee_label,
        grantee_kind: g.grantee_kind,
        purpose: g.purpose,
        created_at_ms: g.created_at_ms,
        expires_at_ms: g.expires_at_ms,
        revoked_at_ms: g.revoked_at_ms,
        // The OHDC wire `Grant` has no suspension timestamp; remote grants
        // surface as not-suspended.
        suspended_at_ms: None,
        default_action: g.default_action,
        approval_mode: g.approval_mode,
        aggregation_only: g.aggregation_only,
        strip_notes: g.strip_notes,
        notify_on_access: g.notify_on_access,
        event_type_rules: g
            .event_type_rules
            .into_iter()
            .map(|r| GrantEventTypeRuleDto {
                event_type: r.event_type,
                effect: r.effect,
            })
            .collect(),
        channel_rules: g
            .channel_rules
            .into_iter()
            .map(|r| GrantChannelRuleDto {
                event_type: r.event_type,
                channel_path: r.channel_path,
                effect: r.effect,
            })
            .collect(),
        sensitivity_rules: g
            .sensitivity_rules
            .into_iter()
            .map(|r| GrantSensitivityRuleDto {
                sensitivity_class: r.sensitivity_class,
                effect: r.effect,
            })
            .collect(),
        auto_approve_event_types: g.auto_approve_event_types,
    }
}

fn create_grant_dto_to_rc(req: CreateGrantInputDto) -> rc::CreateGrantInput {
    let map_et = |rules: Vec<GrantEventTypeRuleDto>| {
        rules
            .into_iter()
            .map(|r| rc::GrantEventTypeRule {
                event_type: r.event_type,
                effect: r.effect,
            })
            .collect()
    };
    rc::CreateGrantInput {
        grantee_label: req.grantee_label,
        grantee_kind: req.grantee_kind,
        purpose: req.purpose,
        default_action: req.default_action,
        approval_mode: req.approval_mode,
        expires_at_ms: req.expires_at_ms,
        event_type_rules: map_et(req.event_type_rules),
        channel_rules: req
            .channel_rules
            .into_iter()
            .map(|r| rc::GrantChannelRule {
                event_type: r.event_type,
                channel_path: r.channel_path,
                effect: r.effect,
            })
            .collect(),
        sensitivity_rules: req
            .sensitivity_rules
            .into_iter()
            .map(|r| rc::GrantSensitivityRule {
                sensitivity_class: r.sensitivity_class,
                effect: r.effect,
            })
            .collect(),
        write_event_type_rules: map_et(req.write_event_type_rules),
        auto_approve_event_types: req.auto_approve_event_types,
        aggregation_only: req.aggregation_only,
        strip_notes: req.strip_notes,
        notify_on_access: req.notify_on_access,
    }
}

fn pending_rc_to_dto(p: rc::PendingEvent) -> PendingEventDto {
    PendingEventDto {
        ulid: ulid_to_crockford(&p.ulid),
        submitted_at_ms: p.submitted_at_ms,
        submitting_grant_ulid: p
            .submitting_grant_ulid
            .as_deref()
            .map(ulid_to_crockford),
        status: p.status,
        reviewed_at_ms: p.reviewed_at_ms,
        rejection_reason: p.rejection_reason,
        expires_at_ms: p.expires_at_ms,
        event: event_rc_to_dto(p.event),
    }
}

fn case_rc_to_dto(c: rc::Case) -> CaseDto {
    CaseDto {
        ulid: ulid_to_crockford(&c.ulid),
        case_type: c.case_type,
        case_label: c.case_label,
        started_at_ms: c.started_at_ms,
        ended_at_ms: c.ended_at_ms,
        parent_case_ulid: c.parent_case_ulid.as_deref().map(ulid_to_crockford),
        predecessor_case_ulid: c.predecessor_case_ulid.as_deref().map(ulid_to_crockford),
        opening_authority_grant_ulid: c
            .opening_authority_grant_ulid
            .as_deref()
            .map(ulid_to_crockford),
        inactivity_close_after_h: c.inactivity_close_after_h,
        last_activity_at_ms: c.last_activity_at_ms,
    }
}

fn audit_rc_to_dto(e: rc::AuditEntry) -> AuditEntryDto {
    AuditEntryDto {
        ts_ms: e.ts_ms,
        actor_type: e.actor_type,
        action: e.action,
        query_kind: e.query_kind,
        query_params_json: e.query_params_json,
        rows_returned: e.rows_returned,
        rows_filtered: e.rows_filtered,
        result: e.result,
        reason: e.reason,
    }
}

// =============================================================================
// WhoAmIDto
// =============================================================================

/// Identity of the bearer token a [`RemoteOhdStorage`] holds, returned by
/// [`RemoteOhdStorage::whoami`].
#[derive(Debug, Clone, uniffi::Record)]
pub struct WhoAmIDto {
    /// Calling user's ULID (Crockford-base32).
    pub user_ulid: String,
    /// `"self_session" | "grant" | "device"`.
    pub token_kind: String,
    /// Grant ULID when the token is a grant bearer (Crockford-base32);
    /// `None` for self-session / device tokens.
    pub grant_ulid: Option<String>,
    /// Grantee display label when the token is a grant bearer.
    pub grantee_label: Option<String>,
    /// Caller IP as observed by the server (may be empty).
    pub caller_ip: String,
}

// =============================================================================
// RemoteOhdStorage object
// =============================================================================

/// Foreign-language handle to a remote `ohd-storage-server`.
///
/// uniffi materializes this on Kotlin as `class RemoteOhdStorage`
/// (constructed via `RemoteOhdStorage(baseUrl, bearerToken)`) and on Swift as
/// `final class RemoteOhdStorage`. Every method blocks the calling thread
/// until the RPC completes — call it off the Android main thread.
#[derive(uniffi::Object)]
pub struct RemoteOhdStorage {
    /// Long-lived multi-thread runtime the RPCs run on. Mirrors the
    /// `remote_access` share responder's runtime ownership: built once at
    /// construction, dropped (joining its worker threads) when the object is
    /// released.
    runtime: Runtime,
    /// The async ConnectRPC client. Cheap to clone; the bearer token lives
    /// behind its own `Mutex` so `set_bearer_token` is a pure swap.
    client: rc::OhdcRemoteClient,
}

#[uniffi::export]
impl RemoteOhdStorage {
    /// Connect to `base_url` (e.g. `https://storage.example.com` or, for dev,
    /// `http://10.0.2.2:18443`), authenticating every RPC with
    /// `bearer_token`.
    ///
    /// Building the object does **not** perform a network round-trip — the
    /// first RPC (commonly [`whoami`](Self::whoami)) is what actually
    /// connects. `https://` URLs use a rustls (ring) TLS stack.
    #[uniffi::constructor]
    pub fn connect(base_url: String, bearer_token: String) -> Result<Arc<RemoteOhdStorage>> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| OhdError::Internal {
                code: "INTERNAL".to_string(),
                message: format!("build remote runtime: {e}"),
            })?;
        let client = rc::OhdcRemoteClient::new(&base_url, &bearer_token)?;
        Ok(Arc::new(Self { runtime, client }))
    }

    /// Replace the bearer token used for subsequent RPCs without rebuilding
    /// the object. The Android layer calls this after minting a refreshed
    /// self-session token in response to a [`OhdError::Auth`] whose `code`
    /// is `TOKEN_EXPIRED`.
    pub fn set_bearer_token(&self, token: String) {
        self.client.set_bearer_token(&token);
    }

    /// Resolve the calling token's identity (`WhoAmI` RPC).
    pub fn whoami(&self) -> Result<WhoAmIDto> {
        let who = self.runtime.block_on(self.client.whoami())?;
        Ok(WhoAmIDto {
            user_ulid: ulid_to_crockford(&who.user_ulid),
            token_kind: who.token_kind,
            grant_ulid: who.grant_ulid.as_deref().map(ulid_to_crockford),
            grantee_label: who.grantee_label,
            caller_ip: who.caller_ip,
        })
    }

    /// Server liveness + version probe (`Health` RPC).
    pub fn protocol_version(&self) -> Result<String> {
        let h = self.runtime.block_on(self.client.health())?;
        Ok(h.protocol_version)
    }

    /// Write one event (`PutEvents` RPC, single-event batch). Mirrors the
    /// local [`OhdStorage::put_event`](crate::OhdStorage::put_event).
    pub fn put_event(&self, input: EventInputDto) -> Result<PutEventOutcomeDto> {
        let outcome = self
            .runtime
            .block_on(self.client.put_event(event_input_dto_to_rc(input)))?;
        Ok(put_outcome_rc_to_dto(outcome))
    }

    /// Write a batch of events in a single `PutEvents` RPC — one round trip
    /// for the whole list instead of one per event. `atomic = true` asks the
    /// server to commit all-or-nothing. Returns one outcome per input, in
    /// order. The bulk path for Health Connect sync, importers, and
    /// multi-event logs.
    pub fn put_events(
        &self,
        inputs: Vec<EventInputDto>,
        atomic: bool,
    ) -> Result<Vec<PutEventOutcomeDto>> {
        let core_inputs = inputs.into_iter().map(event_input_dto_to_rc).collect();
        let outcomes = self
            .runtime
            .block_on(self.client.put_events(core_inputs, atomic))?;
        Ok(outcomes.into_iter().map(put_outcome_rc_to_dto).collect())
    }

    /// Bulk hard-delete events matching the filter (`DeleteEvents` RPC).
    /// All fields optional; an unfiltered call wipes ALL events owned by the
    /// authenticated identity. Returns the number of `events` rows removed
    /// (cascaded channels not counted). Self-session only; grant tokens are
    /// rejected server-side.
    pub fn delete_events(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        event_types: Vec<String>,
    ) -> Result<u64> {
        let filter = rc::DeleteFilter {
            from_ms,
            to_ms,
            event_types,
        };
        Ok(self.runtime.block_on(self.client.delete_events(filter))?)
    }

    /// Read events under a filter (`QueryEvents` server-streaming RPC,
    /// collected into a `Vec`). Mirrors the local
    /// [`OhdStorage::query_events`](crate::OhdStorage::query_events).
    pub fn query_events(&self, filter: EventFilterDto) -> Result<Vec<EventDto>> {
        let events = self
            .runtime
            .block_on(self.client.query_events(event_filter_dto_to_rc(filter)))?;
        Ok(events.into_iter().map(event_rc_to_dto).collect())
    }

    /// Count events matching `filter`. The OHDC service has no count RPC, so
    /// this drains `QueryEvents` and counts — same observable result as the
    /// local `count_events`.
    pub fn count_events(&self, filter: EventFilterDto) -> Result<u64> {
        Ok(self
            .runtime
            .block_on(self.client.count_events(event_filter_dto_to_rc(filter)))?)
    }

    /// List grants (`ListGrants` RPC).
    pub fn list_grants(&self, filter: ListGrantsFilterDto) -> Result<Vec<GrantDto>> {
        let rc_filter = rc::ListGrantsFilter {
            include_revoked: filter.include_revoked,
            include_expired: filter.include_expired,
            grantee_kind: filter.grantee_kind,
            limit: filter.limit,
        };
        let grants = self.runtime.block_on(self.client.list_grants(rc_filter))?;
        Ok(grants.into_iter().map(grant_rc_to_dto).collect())
    }

    /// Create a new grant (`CreateGrant` RPC). Returns the grant ULID +
    /// cleartext bearer token (`ohdg_…`).
    pub fn create_grant(&self, req: CreateGrantInputDto) -> Result<GrantTokenDto> {
        let token = self
            .runtime
            .block_on(self.client.create_grant(create_grant_dto_to_rc(req)))?;
        Ok(GrantTokenDto {
            grant_ulid: ulid_to_crockford(&token.grant_ulid),
            token: token.token,
            share_url: token.share_url,
        })
    }

    /// List the user's pending-event queue (`ListPending` RPC).
    pub fn list_pending(&self) -> Result<Vec<PendingEventDto>> {
        let rows = self.runtime.block_on(self.client.list_pending())?;
        Ok(rows.into_iter().map(pending_rc_to_dto).collect())
    }

    /// List cases (`ListCases` RPC). `state_filter = None` lists open +
    /// closed.
    pub fn list_cases(&self, state_filter: Option<CaseStateDto>) -> Result<Vec<CaseDto>> {
        let include_closed = !matches!(state_filter, Some(CaseStateDto::Open));
        let mut cases = self
            .runtime
            .block_on(self.client.list_cases(include_closed))?;
        if matches!(state_filter, Some(CaseStateDto::Closed)) {
            cases.retain(|c| c.ended_at_ms.is_some());
        }
        Ok(cases.into_iter().map(case_rc_to_dto).collect())
    }

    /// Get one case by ULID (Crockford-base32) — `GetCase` RPC.
    pub fn get_case(&self, case_ulid: String) -> Result<CaseDto> {
        let bytes = ulid_from_crockford(&case_ulid)?;
        let case = self.runtime.block_on(self.client.get_case(bytes))?;
        Ok(case_rc_to_dto(case))
    }

    /// Run an audit query (`AuditQuery` server-streaming RPC, collected into
    /// a `Vec`).
    pub fn audit_query(&self, filter: AuditFilterDto) -> Result<Vec<AuditEntryDto>> {
        let rc_filter = rc::AuditFilter {
            from_ms: filter.from_ms,
            to_ms: filter.to_ms,
            actor_type: filter.actor_type,
            action: filter.action,
            result: filter.result,
            limit: filter.limit,
        };
        let rows = self.runtime.block_on(self.client.audit_query(rc_filter))?;
        Ok(rows.into_iter().map(audit_rc_to_dto).collect())
    }

    /// Drive a full `Export` (server-streaming RPC) and return the number of
    /// export frames the server streamed. Phase 1 surfaces the frame count
    /// as a reachability proof; assembling a portable `.ohd` buffer from the
    /// `ExportChunk` framing is a later-phase deliverable.
    pub fn export(&self) -> Result<u64> {
        Ok(self.runtime.block_on(self.client.export())?)
    }
}
