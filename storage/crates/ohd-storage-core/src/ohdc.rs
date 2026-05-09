//! In-process OHDC service surface.
//!
//! v1 implements: Health, WhoAmI, PutEvents, QueryEvents, GetEventByUlid,
//! ListPending, ApprovePending, RejectPending, CreateGrant, ListGrants,
//! UpdateGrant, RevokeGrant. Every operation appends an `audit_log` row.
//! Token-kind matrix enforced per [`auth::check_kind_for_op`].
//!
//! Wire transport (Connect-RPC over HTTP/2 + HTTP/3) lives in
//! `crates/ohd-storage-server`; this module is the storage-side handler each
//! transport call dispatches to.

use crate::audit::{self, AuditEntry, AuditResult};
use crate::auth::{self, OhdcOp, ResolvedToken, TokenKind};
use crate::cases::{
    self, Case, CaseFilterRow, CaseReopenToken, CaseUpdate, ListCasesFilter, NewCase,
};
use crate::events::{
    self, ChannelScalar, ChannelValue, Event, EventFilter, EventInput, GrantScope, PutEventResult,
};
use crate::grants::{self, GrantRow, GrantUpdate, ListGrantsFilter, NewGrant};
use crate::pending::{self, ListPendingFilter, PendingRow, PendingStatus};
use crate::storage::Storage;
use crate::ulid::{self, Ulid};
use crate::{Error, HealthSummary, Result};

/// Health smoke check.
pub fn health() -> Result<HealthSummary> {
    Ok(HealthSummary::ok())
}

/// `OhdcService.WhoAmI` — return the resolved actor info.
pub fn whoami(storage: &Storage, token: &ResolvedToken) -> Result<WhoAmIInfo> {
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "whoami".into(),
                query_kind: None,
                query_params_json: None,
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )?;
        Ok(WhoAmIInfo {
            user_ulid: ulid::to_crockford(&token.user_ulid),
            token_kind: match token.kind {
                TokenKind::SelfSession => "self_session",
                TokenKind::Grant => "grant",
                TokenKind::Device => "device",
            }
            .to_string(),
            grant_ulid: token.grant_ulid.as_ref().map(ulid::to_crockford),
            grantee_label: token.grantee_label.clone(),
        })
    })
}

/// `OhdcService.PutEvents`.
pub fn put_events(
    storage: &Storage,
    token: &ResolvedToken,
    inputs: &[EventInput],
) -> Result<Vec<PutEventResult>> {
    auth::check_kind_for_op(token, OhdcOp::PutEvents)?;
    let require_approval = match token.kind {
        TokenKind::SelfSession => false,
        TokenKind::Grant => grant_requires_approval(storage, token.grant_id.unwrap_or(0))?,
        TokenKind::Device => false,
    };
    let mut results: Vec<PutEventResult> = vec![];
    let envelope = storage.envelope_key().cloned();
    let res = storage.with_conn_mut(|conn| {
        results = events::put_events(
            conn,
            inputs,
            token.grant_id,
            require_approval,
            envelope.as_ref(),
        )?;
        Ok(())
    });
    let outcome = match (&res, results.iter().any(is_error)) {
        (Ok(()), false) => AuditResult::Success,
        (Ok(()), true) => AuditResult::Partial,
        (Err(_), _) => AuditResult::Error,
    };
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "write".into(),
                query_kind: Some("put_events".into()),
                query_params_json: Some(serde_json::to_string(inputs)?),
                rows_returned: Some(results.len() as i64),
                rows_filtered: None,
                result: outcome,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    res.map(|_| results)
}

fn is_error(r: &PutEventResult) -> bool {
    matches!(r, PutEventResult::Error { .. })
}

fn grant_requires_approval(storage: &Storage, grant_id: i64) -> Result<bool> {
    storage.with_conn(|conn| {
        let mode: Option<String> = conn
            .query_row(
                "SELECT approval_mode FROM grants WHERE id = ?1",
                rusqlite::params![grant_id],
                |r| r.get(0),
            )
            .ok();
        Ok(matches!(mode.as_deref(), Some("always")))
    })
}

/// `OhdcService.QueryEvents`.
pub fn query_events(
    storage: &Storage,
    token: &ResolvedToken,
    filter: &EventFilter,
) -> Result<QueryEventsResponse> {
    auth::check_kind_for_op(token, OhdcOp::QueryEvents)?;
    let scope = grant_scope_for(storage, token)?;
    // aggregation_only blocks raw event reads.
    if let Some(s) = &scope {
        if s.strip_notes {
            // Materialized scope honours strip_notes; nothing to do here.
        }
    }
    if let Some(gid) = token.grant_id {
        let agg_only: bool = storage.with_conn(|conn| {
            conn.query_row(
                "SELECT aggregation_only FROM grants WHERE id = ?1",
                rusqlite::params![gid],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v != 0)
            .map_err(Error::from)
        })?;
        if agg_only {
            return Err(Error::OutOfScope);
        }
        // require_approval_per_query enforcement: check the queue first.
        let payload_json = serde_json::to_string(filter)?;
        check_or_enqueue_approval(storage, gid, "query_events", &payload_json)?;
    }
    let envelope = storage.envelope_key().cloned();
    let (events_out, filtered) = storage.with_conn(|conn| {
        events::query_events_with_key(conn, filter, scope.as_ref(), envelope.as_ref())
    })?;
    let outcome = if filtered > 0 {
        AuditResult::Partial
    } else {
        AuditResult::Success
    };
    let audit_template = AuditEntry {
        ts_ms: audit::now_ms(),
        actor_type: actor_type_for(token.kind),
        auto_granted: false,
        grant_id: token.grant_id,
        action: "read".into(),
        query_kind: Some("list_events".into()),
        query_params_json: Some(serde_json::to_string(filter)?),
        rows_returned: Some(events_out.len() as i64),
        rows_filtered: Some(filtered),
        result: outcome,
        reason: None,
        caller_ip: None,
        caller_ua: None,
        delegated_for_user_ulid: token.delegate_for_user_ulid,
    };
    if token.is_delegate() {
        // Write paired delegate + user-mirror audit rows for transparency.
        let delegate_for = token.delegate_for_user_ulid.unwrap();
        let gid = token.grant_id.unwrap_or(0);
        storage.with_conn(|conn| {
            audit::append_for_delegate(conn, gid, delegate_for, &audit_template)
        })?;
    } else {
        storage.with_conn(|conn| audit::append(conn, &audit_template))?;
    }
    Ok(QueryEventsResponse {
        events: events_out,
        rows_filtered: filtered,
    })
}

/// `OhdcService.GetEventByUlid`.
pub fn get_event_by_ulid(
    storage: &Storage,
    token: &ResolvedToken,
    ulid_str: &str,
) -> Result<Event> {
    auth::check_kind_for_op(token, OhdcOp::GetEventByUlid)?;
    let ulid_bytes = ulid::parse_crockford(ulid_str)?;
    let scope = grant_scope_for(storage, token)?;
    if let Some(gid) = token.grant_id {
        check_or_enqueue_approval(
            storage,
            gid,
            "get_event_by_ulid",
            &format!("{{\"ulid\":\"{ulid_str}\"}}"),
        )?;
    }
    let envelope = storage.envelope_key().cloned();
    let res = storage.with_conn(|conn| {
        events::get_event_by_ulid_scoped_with_key(
            conn,
            &ulid_bytes,
            scope.as_ref(),
            envelope.as_ref(),
        )
    });
    let outcome = match &res {
        Ok(_) => AuditResult::Success,
        Err(Error::NotFound) => AuditResult::Rejected,
        Err(_) => AuditResult::Error,
    };
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("get_event_by_ulid".into()),
                query_params_json: Some(format!("{{\"ulid\":\"{ulid_str}\"}}")),
                rows_returned: Some(if res.is_ok() { 1 } else { 0 }),
                rows_filtered: None,
                result: outcome,
                reason: res.as_ref().err().map(|e| e.to_string()),
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    res
}

// =============================================================================
// Pending-event RPCs
// =============================================================================

/// `OhdcService.ListPending` — list pending events under the caller's auth.
///
/// Self-session: lists every row regardless of submitter.
/// Grant token: lists only the rows submitted under that grant (introspection
/// of in-flight writes, useful for the doctor's "did the patient see my
/// submission yet" surface).
/// Device token: rejected — devices write but never list.
pub fn list_pending(
    storage: &Storage,
    token: &ResolvedToken,
    submitting_grant_ulid: Option<&Ulid>,
    status: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<PendingRow>> {
    auth::check_kind_for_op(token, OhdcOp::ListPending)?;
    let parsed_status = match status {
        None | Some("") => None,
        Some("pending") => Some(PendingStatus::Pending),
        Some("approved") => Some(PendingStatus::Approved),
        Some("rejected") => Some(PendingStatus::Rejected),
        Some("expired") => Some(PendingStatus::Expired),
        Some(other) => {
            return Err(Error::InvalidArgument(format!(
                "unknown pending status filter {other:?}"
            )))
        }
    };
    let only_grant_id = match token.kind {
        TokenKind::Grant => token.grant_id,
        TokenKind::SelfSession => match submitting_grant_ulid {
            Some(u) => Some(storage.with_conn(|conn| grants::grant_id_by_ulid(conn, u))?),
            None => None,
        },
        TokenKind::Device => unreachable!("rejected by check_kind_for_op"),
    };
    let filter = ListPendingFilter {
        submitting_grant_id: only_grant_id,
        status: parsed_status,
        limit,
    };
    let rows = storage.with_conn(|conn| pending::list_pending(conn, &filter))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("list_pending".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "status": status,
                    "submitting_grant_id": filter.submitting_grant_id,
                    "limit": filter.limit,
                }))?),
                rows_returned: Some(rows.len() as i64),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(rows)
}

/// `OhdcService.ApprovePending`. Self-session only.
///
/// Returns `(committed_at_ms, event_ulid)`.
pub fn approve_pending(
    storage: &Storage,
    token: &ResolvedToken,
    pending_ulid: &Ulid,
    also_auto_approve_this_type: bool,
) -> Result<(i64, Ulid)> {
    auth::check_kind_for_op(token, OhdcOp::ApprovePending)?;
    let envelope = storage.envelope_key().cloned();
    let result = storage.with_conn_mut(|conn| {
        pending::approve_pending(
            conn,
            pending_ulid,
            also_auto_approve_this_type,
            envelope.as_ref(),
        )
    });
    if let Err(e) = &result {
        // Approval failures still get a rejected/error audit row.
        storage.with_conn(|conn| {
            audit::append(
                conn,
                &AuditEntry {
                    ts_ms: audit::now_ms(),
                    actor_type: audit::ActorType::Self_,
                    auto_granted: false,
                    grant_id: None,
                    action: "pending_approve".into(),
                    query_kind: Some("approve_pending".into()),
                    query_params_json: Some(format!(
                        "{{\"pending_ulid\":\"{}\"}}",
                        ulid::to_crockford(pending_ulid)
                    )),
                    rows_returned: None,
                    rows_filtered: None,
                    result: match e {
                        Error::NotFound => AuditResult::Rejected,
                        _ => AuditResult::Error,
                    },
                    reason: Some(e.to_string()),
                    caller_ip: None,
                    caller_ua: None,
                    delegated_for_user_ulid: None,
                },
            )
        })?;
    }
    result
}

/// `OhdcService.RejectPending`. Self-session only.
pub fn reject_pending(
    storage: &Storage,
    token: &ResolvedToken,
    pending_ulid: &Ulid,
    reason: Option<&str>,
) -> Result<i64> {
    auth::check_kind_for_op(token, OhdcOp::RejectPending)?;
    storage.with_conn_mut(|conn| pending::reject_pending(conn, pending_ulid, reason))
}

// =============================================================================
// Grant CRUD RPCs
// =============================================================================

/// `OhdcService.CreateGrant`. Self-session only. Returns the new grant row +
/// the cleartext bearer token (shown to the caller exactly once).
pub fn create_grant(
    storage: &Storage,
    token: &ResolvedToken,
    g: &NewGrant,
) -> Result<CreateGrantOutcome> {
    auth::check_kind_for_op(token, OhdcOp::CreateGrant)?;
    let user_ulid = token.user_ulid;
    let envelope = storage.envelope_key().cloned();
    let recovery = storage.recovery_keypair().cloned();
    let (grant_id, grant_ulid) = storage.with_conn_mut(|conn| match envelope.as_ref() {
        Some(env) => grants::create_grant_with_envelope(conn, g, env, recovery.as_ref()),
        None => grants::create_grant(conn, g),
    })?;
    let ttl_ms = g.expires_at_ms.map(|exp| exp - audit::now_ms());
    let bearer = storage.with_conn(|conn| {
        auth::issue_grant_token(conn, user_ulid, grant_id, TokenKind::Grant, ttl_ms)
    })?;
    let row = storage.with_conn(|conn| grants::read_grant(conn, grant_id))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: audit::ActorType::Self_,
                auto_granted: false,
                grant_id: Some(grant_id),
                action: "grant_create".into(),
                query_kind: Some("create_grant".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "grantee_label": g.grantee_label,
                    "grantee_kind": g.grantee_kind,
                    "approval_mode": g.approval_mode,
                    "default_action": g.default_action.as_str(),
                }))?),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(CreateGrantOutcome {
        grant: row,
        token: bearer,
        share_url: format!("ohd://grant/{}", ulid::to_crockford(&grant_ulid)),
    })
}

/// `OhdcService.ListGrants`. Self-session lists all owned grants; grant tokens
/// list only their own (introspection); device tokens are rejected.
pub fn list_grants(
    storage: &Storage,
    token: &ResolvedToken,
    include_revoked: bool,
    include_expired: bool,
    grantee_kind: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<GrantRow>> {
    auth::check_kind_for_op(token, OhdcOp::ListGrants)?;
    let only = match token.kind {
        TokenKind::Grant => token.grant_id,
        TokenKind::SelfSession => None,
        TokenKind::Device => unreachable!("rejected by check_kind_for_op"),
    };
    let f = ListGrantsFilter {
        include_revoked,
        include_expired,
        grantee_kind: grantee_kind.map(str::to_string),
        only_grant_id: only,
        limit,
    };
    let out = storage.with_conn(|conn| grants::list_grants(conn, &f))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("list_grants".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "include_revoked": include_revoked,
                    "include_expired": include_expired,
                    "grantee_kind": grantee_kind,
                }))?),
                rows_returned: Some(out.len() as i64),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(out)
}

/// `OhdcService.UpdateGrant`. Self-session only.
pub fn update_grant(
    storage: &Storage,
    token: &ResolvedToken,
    grant_ulid: &Ulid,
    update: &GrantUpdate,
) -> Result<GrantRow> {
    auth::check_kind_for_op(token, OhdcOp::UpdateGrant)?;
    let grant_id = storage.with_conn(|conn| grants::grant_id_by_ulid(conn, grant_ulid))?;
    storage.with_conn_mut(|conn| grants::update_grant(conn, grant_id, update))
}

/// `OhdcService.RevokeGrant`. Self-session only. Synchronous per spec —
/// revocation is never sync-deferred.
///
/// Returns the revocation timestamp (Unix ms).
pub fn revoke_grant(
    storage: &Storage,
    token: &ResolvedToken,
    grant_ulid: &Ulid,
    reason: Option<&str>,
) -> Result<i64> {
    auth::check_kind_for_op(token, OhdcOp::RevokeGrant)?;
    let grant_id = storage.with_conn(|conn| grants::grant_id_by_ulid(conn, grant_ulid))?;
    storage.with_conn(|conn| grants::revoke_grant(conn, grant_id, reason))
}

// =============================================================================
// Case CRUD RPCs
// =============================================================================

/// `OhdcService.CreateCase`. Self-session opens directly. Grant tokens open
/// under the break-glass / care-visit pattern — the case's
/// `opening_authority_grant_id` is set to the caller's grant. The new case is
/// also bound to the calling grant via `grant_cases` so subsequent reads
/// resolve via case scope.
pub fn create_case(storage: &Storage, token: &ResolvedToken, new_case: &NewCase) -> Result<Case> {
    auth::check_kind_for_op(token, OhdcOp::CreateCase)?;
    // Inject the grant id when the caller is a grant token; preserve the
    // caller-supplied id otherwise (covers the "grant opens on behalf of
    // another authority" edge case, though v1 doesn't surface that).
    let mut nc = new_case.clone();
    if token.kind == TokenKind::Grant && nc.opening_authority_grant_id.is_none() {
        nc.opening_authority_grant_id = token.grant_id;
    }

    let (case_id, case_ulid) = storage.with_conn_mut(|conn| cases::create_case(conn, &nc))?;

    // For grant-opened cases, bind the grant to the case.
    if token.kind == TokenKind::Grant {
        if let Some(gid) = token.grant_id {
            storage.with_conn(|conn| cases::bind_grant_to_cases(conn, gid, &[case_id]))?;
        }
    }

    // Write a `std.case_started` marker event for the patient timeline.
    let _ = write_case_marker(
        storage,
        token,
        "std.case_started",
        &case_ulid,
        &[
            ("case_type".into(), nc.case_type.clone()),
            (
                "case_label".into(),
                nc.case_label.clone().unwrap_or_default(),
            ),
        ],
    );

    let row = storage.with_conn(|conn| cases::read_case(conn, case_id))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "case_create".into(),
                query_kind: Some("create_case".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "case_type": nc.case_type,
                    "case_label": nc.case_label,
                    "case_ulid": ulid::to_crockford(&case_ulid),
                }))?),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(row)
}

/// `OhdcService.UpdateCase`. Self-session, or the case's opening authority.
pub fn update_case(
    storage: &Storage,
    token: &ResolvedToken,
    case_ulid: &Ulid,
    update: &CaseUpdate,
) -> Result<Case> {
    auth::check_kind_for_op(token, OhdcOp::UpdateCase)?;
    let case_id = storage.with_conn(|conn| cases::case_id_by_ulid(conn, case_ulid))?;
    require_case_authority(storage, token, case_id)?;
    let row = storage.with_conn_mut(|conn| cases::update_case(conn, case_id, update))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "case_update".into(),
                query_kind: Some("update_case".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "case_ulid": ulid::to_crockford(case_ulid),
                }))?),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(row)
}

/// `OhdcService.CloseCase`. Self-session can close any case (force-close
/// behavior). Grant tokens can close their own opened cases. Issues a
/// reopen token to the closing authority when applicable.
pub fn close_case(
    storage: &Storage,
    token: &ResolvedToken,
    case_ulid: &Ulid,
    reason: Option<&str>,
) -> Result<(Case, Option<CaseReopenToken>)> {
    auth::check_kind_for_op(token, OhdcOp::CloseCase)?;
    let case_id = storage.with_conn(|conn| cases::case_id_by_ulid(conn, case_ulid))?;
    require_case_authority(storage, token, case_id)?;

    // For self-session "force close", we don't issue a reopen token (patient
    // explicitly revoked authority). For authority-driven close, we do.
    let issue_token = token.kind != TokenKind::SelfSession;

    let outcome = storage.with_conn_mut(|conn| {
        cases::close_case(conn, case_id, token.grant_id, issue_token, None)
    })?;

    let _ = write_case_marker(
        storage,
        token,
        "std.case_closed",
        case_ulid,
        &[("reason".into(), reason.unwrap_or("").to_string())],
    );

    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "case_close".into(),
                query_kind: Some("close_case".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "case_ulid": ulid::to_crockford(case_ulid),
                    "force_closed_by_patient": token.kind == TokenKind::SelfSession,
                }))?),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: reason.map(str::to_string),
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(outcome)
}

/// `OhdcService.ReopenCase`. Two paths:
/// - **Token reopen**: caller presents a `case_reopen_token_ulid`; valid token
///   reopens the case and is consumed. Any token kind allowed (the token
///   itself proves authority).
/// - **Patient reopen**: caller is self-session and supplies the case ULID
///   directly.
pub fn reopen_case_by_token(
    storage: &Storage,
    token: &ResolvedToken,
    reopen_token_ulid: &Ulid,
) -> Result<Case> {
    auth::check_kind_for_op(token, OhdcOp::ReopenCase)?;
    let case_id =
        storage.with_conn_mut(|conn| cases::redeem_reopen_token(conn, reopen_token_ulid))?;
    let case = storage.with_conn_mut(|conn| cases::reopen_case(conn, case_id))?;
    let _ = write_case_marker(storage, token, "std.case_reopened", &case.ulid, &[]);
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "case_reopen".into(),
                query_kind: Some("reopen_case".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "case_ulid": ulid::to_crockford(&case.ulid),
                    "method": "token",
                }))?),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(case)
}

/// Self-session-only reopen. Bypasses the token TTL (patient is the ultimate
/// authority).
pub fn reopen_case_by_patient(
    storage: &Storage,
    token: &ResolvedToken,
    case_ulid: &Ulid,
) -> Result<Case> {
    auth::check_kind_for_op(token, OhdcOp::ReopenCase)?;
    if token.kind != TokenKind::SelfSession {
        return Err(Error::WrongTokenKind(
            "patient reopen requires self-session",
        ));
    }
    let case_id = storage.with_conn(|conn| cases::case_id_by_ulid(conn, case_ulid))?;
    let case = storage.with_conn_mut(|conn| cases::reopen_case(conn, case_id))?;
    let _ = write_case_marker(storage, token, "std.case_reopened", case_ulid, &[]);
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "case_reopen".into(),
                query_kind: Some("reopen_case".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "case_ulid": ulid::to_crockford(case_ulid),
                    "method": "patient",
                }))?),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(case)
}

/// `OhdcService.ListCases`. Self-session sees all cases. Grant tokens see
/// only cases bound to them via `grant_cases`.
pub fn list_cases(
    storage: &Storage,
    token: &ResolvedToken,
    include_closed: bool,
    case_type: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<Case>> {
    auth::check_kind_for_op(token, OhdcOp::ListCases)?;
    let only_case_ids = match token.kind {
        TokenKind::SelfSession => None,
        TokenKind::Grant => {
            let gid = token.grant_id.ok_or(Error::Unauthenticated)?;
            Some(storage.with_conn(|conn| cases::grant_case_ids(conn, gid))?)
        }
        TokenKind::Device => {
            return Err(Error::WrongTokenKind("device tokens cannot list cases"));
        }
    };
    let f = ListCasesFilter {
        include_closed,
        case_type: case_type.map(str::to_string),
        only_case_ids,
        limit,
        ..Default::default()
    };
    let rows = storage.with_conn(|conn| cases::list_cases(conn, &f))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("list_cases".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "include_closed": include_closed,
                    "case_type": case_type,
                }))?),
                rows_returned: Some(rows.len() as i64),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(rows)
}

/// `OhdcService.GetCase`. Same scope rules as ListCases. Returns
/// [`Error::CaseNotFound`] for case ULIDs the caller can't see.
pub fn get_case(storage: &Storage, token: &ResolvedToken, case_ulid: &Ulid) -> Result<Case> {
    auth::check_kind_for_op(token, OhdcOp::GetCase)?;
    let case_id = storage.with_conn(|conn| cases::case_id_by_ulid(conn, case_ulid))?;
    require_case_visibility(storage, token, case_id)?;
    let case = storage.with_conn(|conn| cases::read_case(conn, case_id))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("get_case".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "case_ulid": ulid::to_crockford(case_ulid),
                }))?),
                rows_returned: Some(1),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(case)
}

/// `OhdcService.AddCaseFilter`.
pub fn add_case_filter(
    storage: &Storage,
    token: &ResolvedToken,
    case_ulid: &Ulid,
    filter: &EventFilter,
    label: Option<&str>,
) -> Result<CaseFilterRow> {
    auth::check_kind_for_op(token, OhdcOp::AddCaseFilter)?;
    let case_id = storage.with_conn(|conn| cases::case_id_by_ulid(conn, case_ulid))?;
    require_case_authority(storage, token, case_id)?;
    let added_by_grant_id = token.grant_id;
    let row = storage.with_conn_mut(|conn| {
        cases::add_case_filter(conn, case_id, filter, label, added_by_grant_id)
    })?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "case_filter_add".into(),
                query_kind: Some("add_case_filter".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "case_ulid": ulid::to_crockford(case_ulid),
                    "label": label,
                }))?),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(row)
}

/// `OhdcService.RemoveCaseFilter`.
pub fn remove_case_filter(
    storage: &Storage,
    token: &ResolvedToken,
    case_filter_ulid: &Ulid,
) -> Result<i64> {
    auth::check_kind_for_op(token, OhdcOp::RemoveCaseFilter)?;
    // Decode the filter ULID's rowid (encoded into the random tail by
    // `read_case_filter`). v1 carries the filter id in the first 8 bytes of
    // the random tail; reverse that mapping here.
    let tail = ulid::random_tail(case_filter_ulid);
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&tail[..8]);
    let filter_id = i64::from_be_bytes(buf);

    // Resolve the case via the filter row, then check authority on the case.
    let row = storage.with_conn(|conn| cases::read_case_filter(conn, filter_id))?;
    require_case_authority(storage, token, row.case_id)?;
    let removed_at = storage.with_conn(|conn| cases::remove_case_filter(conn, filter_id))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "case_filter_remove".into(),
                query_kind: Some("remove_case_filter".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "case_filter_ulid": ulid::to_crockford(case_filter_ulid),
                }))?),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(removed_at)
}

/// `OhdcService.ListCaseFilters`.
pub fn list_case_filters(
    storage: &Storage,
    token: &ResolvedToken,
    case_ulid: &Ulid,
    include_removed: bool,
) -> Result<Vec<CaseFilterRow>> {
    auth::check_kind_for_op(token, OhdcOp::ListCaseFilters)?;
    let case_id = storage.with_conn(|conn| cases::case_id_by_ulid(conn, case_ulid))?;
    require_case_visibility(storage, token, case_id)?;
    let rows =
        storage.with_conn(|conn| cases::list_case_filters(conn, case_id, include_removed))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("list_case_filters".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "case_ulid": ulid::to_crockford(case_ulid),
                    "include_removed": include_removed,
                }))?),
                rows_returned: Some(rows.len() as i64),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(rows)
}

// =============================================================================
// Helpers
// =============================================================================

/// Confirm the caller can see this case. Self-session = always; grant tokens =
/// only when bound via `grant_cases`.
fn require_case_visibility(storage: &Storage, token: &ResolvedToken, case_id: i64) -> Result<()> {
    match token.kind {
        TokenKind::SelfSession => Ok(()),
        TokenKind::Grant => {
            let gid = token.grant_id.ok_or(Error::Unauthenticated)?;
            let cases = storage.with_conn(|conn| cases::grant_case_ids(conn, gid))?;
            if cases.contains(&case_id) {
                Ok(())
            } else {
                Err(Error::CaseNotFound)
            }
        }
        TokenKind::Device => Err(Error::WrongTokenKind("device tokens cannot read cases")),
    }
}

/// Confirm the caller may mutate this case (close / update / add filter /
/// remove filter). Self-session = always; grant tokens = only when they
/// opened the case (matched via `cases.opening_authority_grant_id`).
fn require_case_authority(storage: &Storage, token: &ResolvedToken, case_id: i64) -> Result<()> {
    match token.kind {
        TokenKind::SelfSession => Ok(()),
        TokenKind::Grant => {
            let gid = token.grant_id.ok_or(Error::Unauthenticated)?;
            let opening: Option<i64> = storage.with_conn(|conn| {
                conn.query_row(
                    "SELECT opening_authority_grant_id FROM cases WHERE id = ?1",
                    rusqlite::params![case_id],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .map_err(Error::from)
            })?;
            if opening == Some(gid) {
                Ok(())
            } else {
                Err(Error::OutOfScope)
            }
        }
        TokenKind::Device => Err(Error::WrongTokenKind("device tokens cannot mutate cases")),
    }
}

/// Write a `std.case_*` marker event to the events table for the patient
/// timeline. Best-effort — failures here don't fail the lifecycle RPC.
fn write_case_marker(
    storage: &Storage,
    token: &ResolvedToken,
    event_type: &str,
    case_ulid: &Ulid,
    extra_text_channels: &[(String, String)],
) -> Result<()> {
    let now = audit::now_ms();
    let mut channels: Vec<ChannelValue> = vec![ChannelValue {
        channel_path: "case_ref_ulid".into(),
        value: ChannelScalar::Text {
            text_value: ulid::to_crockford(case_ulid),
        },
    }];
    for (path, val) in extra_text_channels {
        if val.is_empty() {
            continue;
        }
        channels.push(ChannelValue {
            channel_path: path.clone(),
            value: ChannelScalar::Text {
                text_value: val.clone(),
            },
        });
    }
    let input = EventInput {
        timestamp_ms: now,
        event_type: event_type.into(),
        channels,
        ..Default::default()
    };
    let envelope = storage.envelope_key().cloned();
    storage.with_conn_mut(|conn| {
        let _ = events::put_events(conn, &[input], token.grant_id, false, envelope.as_ref())?;
        Ok(())
    })
}

fn actor_type_for(kind: TokenKind) -> audit::ActorType {
    match kind {
        TokenKind::SelfSession => audit::ActorType::Self_,
        TokenKind::Grant | TokenKind::Device => audit::ActorType::Grant,
    }
}

/// Materialize the full grant scope for `token`. Self-session callers see
/// `None` (unrestricted). Returns `Err(Error::RateLimited)` if the grant has
/// rate limits and they're exceeded.
fn grant_scope_for(storage: &Storage, token: &ResolvedToken) -> Result<Option<GrantScope>> {
    let Some(gid) = token.grant_id else {
        return Ok(None);
    };
    let now_ms = audit::now_ms();
    let scope = storage.with_conn(|conn| {
        let row: (String, i64, Option<i32>, Option<i32>, Option<i32>) = conn
            .query_row(
                "SELECT default_action, strip_notes, max_queries_per_day,
                        max_queries_per_hour, rolling_window_days
                   FROM grants WHERE id = ?1",
                rusqlite::params![gid],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<i32>>(2)?,
                        r.get::<_, Option<i32>>(3)?,
                        r.get::<_, Option<i32>>(4)?,
                    ))
                },
            )
            .unwrap_or(("deny".into(), 1, None, None, None));
        let (default_action, strip_notes, max_per_day, max_per_hour, rolling_days) = row;

        let mut allow = vec![];
        let mut deny = vec![];
        let mut stmt = conn.prepare(
            "SELECT event_type_id, effect FROM grant_event_type_rules WHERE grant_id = ?1",
        )?;
        for r in stmt.query_map(rusqlite::params![gid], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })? {
            let (id, effect) = r?;
            if effect == "allow" {
                allow.push(id);
            } else {
                deny.push(id);
            }
        }
        let mut sens_allow = vec![];
        let mut sens_deny = vec![];
        let mut stmt2 = conn.prepare(
            "SELECT sensitivity_class, effect FROM grant_sensitivity_rules WHERE grant_id = ?1",
        )?;
        for r in stmt2.query_map(rusqlite::params![gid], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })? {
            let (cls, effect) = r?;
            if effect == "allow" {
                sens_allow.push(cls);
            } else {
                sens_deny.push(cls);
            }
        }
        let mut chan_allow = vec![];
        let mut chan_deny = vec![];
        let mut stmt3 =
            conn.prepare("SELECT channel_id, effect FROM grant_channel_rules WHERE grant_id = ?1")?;
        for r in stmt3.query_map(rusqlite::params![gid], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })? {
            let (id, effect) = r?;
            if effect == "allow" {
                chan_allow.push(id);
            } else {
                chan_deny.push(id);
            }
        }
        let absolute_window: Option<(i64, i64)> = conn
            .query_row(
                "SELECT from_ms, to_ms FROM grant_time_windows WHERE grant_id = ?1",
                rusqlite::params![gid],
                |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .ok()
            .and_then(|(f, t)| match (f, t) {
                (Some(f), Some(t)) => Some((f, t)),
                _ => None,
            });

        Ok(GrantScope {
            default_allow: default_action == "allow",
            event_type_allow: allow,
            event_type_deny: deny,
            sensitivity_allow: sens_allow,
            sensitivity_deny: sens_deny,
            channel_allow: chan_allow,
            channel_deny: chan_deny,
            strip_notes: strip_notes != 0,
            rolling_window_days: rolling_days,
            absolute_window,
            max_queries_per_day: max_per_day,
            max_queries_per_hour: max_per_hour,
            now_ms,
        })
    })?;

    // Rate-limit check (counts read RPCs under this grant in the last window).
    if let Some(per_hour) = scope.max_queries_per_hour {
        let used = grant_query_count(storage, gid, now_ms, 3600 * 1000)?;
        if used >= per_hour as i64 {
            return Err(Error::RateLimited);
        }
    }
    if let Some(per_day) = scope.max_queries_per_day {
        let used = grant_query_count(storage, gid, now_ms, 86_400 * 1000)?;
        if used >= per_day as i64 {
            return Err(Error::RateLimited);
        }
    }
    Ok(Some(scope))
}

/// Check whether the grant has `require_approval_per_query=1`, and if so
/// either short-circuit with an [`Error::PendingApproval`] (caller hasn't
/// yet been approved for this exact query) or return Ok(()) when the user
/// has already approved the same query.
///
/// Auto-runs `pending_queries` lookup and enqueue logic. The query payload
/// must be a deterministic JSON serialization of the request (we use
/// `serde_json::to_string(filter)`); the same payload re-issued by the
/// grantee produces the same hash.
///
/// Returns:
/// - `Ok(())` — query already approved; proceed.
/// - `Err(PendingApproval)` — query freshly queued or still pending.
/// - `Err(OutOfScope)` — user previously rejected.
/// - `Err(ApprovalTimeout)` — pending row auto-expired.
pub fn check_or_enqueue_approval(
    storage: &Storage,
    grant_id: i64,
    query_kind: &str,
    payload_json: &str,
) -> Result<()> {
    let needs_approval = storage.with_conn(|conn| {
        conn.query_row(
            "SELECT require_approval_per_query FROM grants WHERE id = ?1",
            rusqlite::params![grant_id],
            |r| r.get::<_, i64>(0),
        )
        .map(|v| v != 0)
        .map_err(Error::from)
    })?;
    if !needs_approval {
        return Ok(());
    }
    use crate::pending_queries::{self, QueryDecision};
    let existing = storage.with_conn(|conn| {
        pending_queries::lookup_decision(conn, grant_id, query_kind, payload_json)
    })?;
    match existing {
        Some((_ulid, QueryDecision::Approved, _)) => Ok(()),
        Some((ulid, QueryDecision::Pending, expires_at_ms)) => Err(Error::PendingApproval {
            ulid_crockford: ulid::to_crockford(&ulid),
            expires_at_ms,
        }),
        Some((_ulid, QueryDecision::Rejected, _)) => Err(Error::OutOfScope),
        Some((_ulid, QueryDecision::Expired, _)) => Err(Error::ApprovalTimeout),
        None => {
            // First time this exact query has been seen for this grant —
            // enqueue + return PendingApproval.
            let (ulid, expires_at_ms) = storage.with_conn(|conn| {
                pending_queries::enqueue(conn, grant_id, query_kind, payload_json, None)
            })?;
            // Audit the enqueue so the user sees it in their access log.
            storage.with_conn(|conn| {
                audit::append(
                    conn,
                    &AuditEntry {
                        ts_ms: audit::now_ms(),
                        actor_type: audit::ActorType::Grant,
                        auto_granted: false,
                        grant_id: Some(grant_id),
                        action: "approval_enqueue".into(),
                        query_kind: Some(query_kind.into()),
                        query_params_json: Some(payload_json.to_string()),
                        rows_returned: None,
                        rows_filtered: None,
                        result: AuditResult::Partial,
                        reason: Some(format!(
                            "require_approval_per_query: queued as {}",
                            ulid::to_crockford(&ulid)
                        )),
                        caller_ip: None,
                        caller_ua: None,
                        delegated_for_user_ulid: None,
                    },
                )
            })?;
            Err(Error::PendingApproval {
                ulid_crockford: ulid::to_crockford(&ulid),
                expires_at_ms,
            })
        }
    }
}

/// `OhdcService.IssueDelegateGrant` (proto-pending; exposed as a core
/// helper + CLI command in v1).
///
/// Wraps [`grants::create_grant`] with `grantee_kind="delegate"` +
/// `delegate_for_user_ulid` set to the storage's own `user_ulid`. The
/// resulting bearer token, when presented, lets the holder (the delegate)
/// read this user's data subject to the grant's scope. Self-session
/// callers only — grants don't chain.
///
/// `delegate_label`: display name for the delegate (e.g. "Caregiver Smith");
/// the actual caregiver identity is captured in `grants.grantee_ulid` when
/// OIDC binds it, which is a v1.x deliverable.
pub fn issue_delegate_grant(
    storage: &Storage,
    token: &ResolvedToken,
    delegate_label: &str,
    purpose: Option<String>,
    new_grant_template: &grants::NewGrant,
) -> Result<CreateGrantOutcome> {
    if token.kind != TokenKind::SelfSession {
        return Err(Error::WrongTokenKind(
            "issue_delegate_grant requires self-session",
        ));
    }
    let mut new_grant = new_grant_template.clone();
    new_grant.grantee_kind = "delegate".into();
    // Pin the delegate-for to this file's user. This is the only valid
    // value in v1 (one user per file).
    new_grant.delegate_for_user_ulid = Some(storage.user_ulid());
    new_grant.grantee_label = delegate_label.to_string();
    if purpose.is_some() {
        new_grant.purpose = purpose;
    }
    create_grant(storage, token, &new_grant)
}

/// `OhdcService.ListPendingQueries`. Returns
/// the per-query approval queue. Self-session callers see all rows;
/// grant-token callers only their own.
pub fn list_pending_queries(
    storage: &Storage,
    token: &ResolvedToken,
    grant_filter: Option<i64>,
    decision: Option<crate::pending_queries::QueryDecision>,
    since_ms: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<crate::pending_queries::PendingQueryRow>> {
    auth::check_kind_for_op(token, OhdcOp::ListPendingQueries)?;
    if token.kind != TokenKind::SelfSession && token.grant_id != grant_filter {
        return Err(Error::WrongTokenKind(
            "list_pending_queries: grant tokens may only list their own rows",
        ));
    }
    let filter = crate::pending_queries::ListPendingQueriesFilter {
        grant_id: grant_filter.or(token.grant_id),
        decision,
        since_ms,
        limit,
    };
    storage.with_conn(|conn| crate::pending_queries::list_pending_queries(conn, &filter))
}

/// `OhdcService.ApprovePendingQuery`. Self-session only.
pub fn approve_pending_query(
    storage: &Storage,
    token: &ResolvedToken,
    query_ulid: &Ulid,
) -> Result<i64> {
    auth::check_kind_for_op(token, OhdcOp::ApprovePendingQuery)?;
    let now = storage.with_conn(|conn| crate::pending_queries::approve(conn, query_ulid))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: now,
                actor_type: audit::ActorType::Self_,
                auto_granted: false,
                grant_id: None,
                action: "approve_pending_query".into(),
                query_kind: Some("approve_pending_query".into()),
                query_params_json: Some(format!(
                    "{{\"query_ulid\":\"{}\"}}",
                    ulid::to_crockford(query_ulid)
                )),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(now)
}

/// `OhdcService.RejectPendingQuery`. Self-session only.
pub fn reject_pending_query(
    storage: &Storage,
    token: &ResolvedToken,
    query_ulid: &Ulid,
    reason: Option<&str>,
) -> Result<i64> {
    auth::check_kind_for_op(token, OhdcOp::RejectPendingQuery)?;
    let now = storage.with_conn(|conn| crate::pending_queries::reject(conn, query_ulid))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: now,
                actor_type: audit::ActorType::Self_,
                auto_granted: false,
                grant_id: None,
                action: "reject_pending_query".into(),
                query_kind: Some("reject_pending_query".into()),
                query_params_json: Some(format!(
                    "{{\"query_ulid\":\"{}\"}}",
                    ulid::to_crockford(query_ulid)
                )),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Rejected,
                reason: reason.map(str::to_string),
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(now)
}

/// Count `audit_log` rows for `grant_id` with `action='read'` within the
/// trailing `window_ms` (used for rate-limit enforcement).
fn grant_query_count(storage: &Storage, grant_id: i64, now_ms: i64, window_ms: i64) -> Result<i64> {
    let cutoff = now_ms.saturating_sub(window_ms);
    storage.with_conn(|conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM audit_log
              WHERE grant_id = ?1 AND action = 'read' AND ts_ms >= ?2",
            rusqlite::params![grant_id, cutoff],
            |r| r.get::<_, i64>(0),
        )
        .map_err(Error::from)
    })
}

/// `OhdcService.WhoAmI` payload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WhoAmIInfo {
    /// User ULID (Crockford-base32).
    pub user_ulid: String,
    /// `"self_session"` / `"grant"` / `"device"`.
    pub token_kind: String,
    /// Grant ULID for grant/device tokens.
    pub grant_ulid: Option<String>,
    /// Grantee label for grant/device tokens.
    pub grantee_label: Option<String>,
}

/// Server-streaming `QueryEvents` results materialized into a vec.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryEventsResponse {
    /// Event rows in `TIME_DESC` order.
    pub events: Vec<Event>,
    /// Rows the grant scope dropped silently. Self-session always gets 0.
    pub rows_filtered: i64,
}

/// `OhdcService.CreateGrant` payload.
#[derive(Debug, Clone)]
pub struct CreateGrantOutcome {
    /// The created grant row.
    pub grant: GrantRow,
    /// Cleartext bearer token; shown to the user exactly once.
    pub token: String,
    /// Share URL (`ohd://grant/<ulid>`); the wire QR is left to the consumer.
    pub share_url: String,
}

// =============================================================================
// Sample / attachment / aggregate / correlate / audit / export / import
// =============================================================================

/// `OhdcService.ReadSamples`. Decodes every sample block on
/// `(event_ulid, channel_path)` and returns the absolute-timestamped samples.
///
/// Honours the optional `[from_ms, to_ms]` slice. `max_samples > 0` triggers
/// a server-side downsample (even spacing); 0 returns raw decoded samples.
///
/// Audit row: `action='read'`, `query_kind='read_samples'`. Grant-scope rules
/// apply via the underlying event read (out-of-scope events return
/// `NOT_FOUND`).
pub fn read_samples(
    storage: &Storage,
    token: &ResolvedToken,
    event_ulid: &Ulid,
    channel_path: &str,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    max_samples: i32,
) -> Result<Vec<crate::samples::AbsoluteSample>> {
    auth::check_kind_for_op(token, OhdcOp::ReadSamples)?;
    // aggregation_only blocks raw sample reads (per privacy-access.md table).
    if token.kind == TokenKind::Grant {
        if let Some(gid) = token.grant_id {
            let agg_only: bool = storage.with_conn(|conn| {
                conn.query_row(
                    "SELECT aggregation_only FROM grants WHERE id = ?1",
                    rusqlite::params![gid],
                    |r| r.get::<_, i64>(0),
                )
                .map(|v| v != 0)
                .map_err(Error::from)
            })?;
            if agg_only {
                return Err(Error::OutOfScope);
            }
        }
    }

    // Make sure the event itself is in scope (grant-scope intersection).
    let scope = grant_scope_for(storage, token)?;
    let _ = storage
        .with_conn(|conn| events::get_event_by_ulid_scoped(conn, event_ulid, scope.as_ref()))?;

    let raw = storage.with_conn(|conn| {
        crate::samples::read_samples_decoded(conn, event_ulid, channel_path, from_ms, to_ms)
    })?;
    let max = if max_samples > 0 {
        max_samples as usize
    } else {
        0
    };
    let out = if max > 0 {
        crate::samples::downsample(raw, max)
    } else {
        raw
    };
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("read_samples".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "event_ulid": ulid::to_crockford(event_ulid),
                    "channel_path": channel_path,
                    "from_ms": from_ms,
                    "to_ms": to_ms,
                    "max_samples": max_samples,
                }))?),
                rows_returned: Some(out.len() as i64),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(out)
}

/// `OhdcService.AttachBlob`. Persists the streamed bytes to the sidecar
/// directory and inserts an `attachments` row bound to `event_id`.
pub fn attach_blob(
    storage: &Storage,
    token: &ResolvedToken,
    event_ulid: &Ulid,
    mime_type: Option<String>,
    filename: Option<String>,
    data: &[u8],
    expected_sha256: Option<&[u8]>,
) -> Result<crate::attachments::AttachmentMetaRow> {
    auth::check_kind_for_op(token, OhdcOp::AttachBlob)?;
    let storage_path = storage.path().to_path_buf();
    let root = crate::attachments::sidecar_root_for(&storage_path);
    // Resolve event id under grant scope: callers can only attach to events
    // they're allowed to see.
    let event_id: i64 = {
        let scope = grant_scope_for(storage, token)?;
        let _e = storage
            .with_conn(|conn| events::get_event_by_ulid_scoped(conn, event_ulid, scope.as_ref()))?;
        let rand_tail = ulid::random_tail(event_ulid);
        storage.with_conn(|conn| {
            conn.query_row(
                "SELECT id FROM events WHERE ulid_random = ?1",
                rusqlite::params![rand_tail.to_vec()],
                |r| r.get::<_, i64>(0),
            )
            .map_err(Error::from)
        })?
    };
    // Default-on encryption: `K_envelope` from the storage handle wraps a
    // fresh per-attachment DEK, and the finalize step writes
    // `nonce(12) || ciphertext+tag` to disk under the sha-of-PLAINTEXT path.
    // The metadata sha256 (which the wire frame validates against) stays
    // sha-of-plaintext per the spec.
    let mut writer = match storage.envelope_key().cloned() {
        Some(env) => crate::attachments::new_writer_with_envelope(&root, mime_type, filename, env)?,
        // Testing-only no-cipher-key path — falls through to plaintext.
        None => crate::attachments::new_writer(&root, mime_type, filename)?,
    };
    writer.write_chunk(data)?;
    // Codex review #3+#6: finalize binds (event_ulid, sha256, mime,
    // filename, byte_size) into the AAD and stream-encrypts in 64 KiB
    // chunks so the full plaintext never sits in memory.
    let (_path, row) =
        storage.with_conn(|conn| writer.finalize(conn, event_id, event_ulid, expected_sha256))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "write".into(),
                query_kind: Some("attach_blob".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "event_ulid": ulid::to_crockford(event_ulid),
                    "byte_size": row.byte_size,
                    "sha256": hex::encode(row.sha256),
                }))?),
                rows_returned: Some(1),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(row)
}

/// `OhdcService.ReadAttachment`. Resolves the attachment by ULID and returns
/// the metadata + the **plaintext** bytes. The streaming layer in
/// `server.rs` chunks these for the wire.
///
/// As of the default-on encryption flip, on-disk attachment bytes are
/// `nonce(12) || ciphertext_with_tag` per the encrypted-attachment format.
/// This function unwraps the per-attachment DEK under the storage handle's
/// `K_envelope` and returns plaintext. Legacy plaintext rows
/// (`wrapped_dek IS NULL`) are returned as-is.
///
/// For callers that genuinely want the on-disk path (e.g. a future
/// streaming-decrypt reader), use [`read_attachment_meta_path`].
pub fn read_attachment_bytes(
    storage: &Storage,
    token: &ResolvedToken,
    attachment_ulid: &Ulid,
) -> Result<(crate::attachments::AttachmentMetaRow, Vec<u8>)> {
    let (row, _path) = read_attachment_meta_path(storage, token, attachment_ulid)?;
    let storage_path = storage.path().to_path_buf();
    let root = crate::attachments::sidecar_root_for(&storage_path);
    let envelope = storage.envelope_key().cloned();
    let bytes = storage.with_conn(|conn| {
        crate::attachments::read_attachment_bytes(conn, &root, attachment_ulid, envelope.as_ref())
    })?;
    Ok((row, bytes))
}

/// Internal: same as legacy `read_attachment` — metadata + on-disk path.
/// Most callers want [`read_attachment_bytes`] (which decrypts).
pub fn read_attachment_meta_path(
    storage: &Storage,
    token: &ResolvedToken,
    attachment_ulid: &Ulid,
) -> Result<(crate::attachments::AttachmentMetaRow, std::path::PathBuf)> {
    read_attachment(storage, token, attachment_ulid)
}

/// `OhdcService.ReadAttachment`. Resolves the attachment by ULID and returns
/// the metadata + on-disk path; the streaming layer pumps bytes from the file.
pub fn read_attachment(
    storage: &Storage,
    token: &ResolvedToken,
    attachment_ulid: &Ulid,
) -> Result<(crate::attachments::AttachmentMetaRow, std::path::PathBuf)> {
    auth::check_kind_for_op(token, OhdcOp::ReadAttachment)?;
    if token.kind == TokenKind::Grant {
        if let Some(gid) = token.grant_id {
            let agg_only: bool = storage.with_conn(|conn| {
                conn.query_row(
                    "SELECT aggregation_only FROM grants WHERE id = ?1",
                    rusqlite::params![gid],
                    |r| r.get::<_, i64>(0),
                )
                .map(|v| v != 0)
                .map_err(Error::from)
            })?;
            if agg_only {
                return Err(Error::OutOfScope);
            }
        }
    }
    let storage_path = storage.path().to_path_buf();
    let root = crate::attachments::sidecar_root_for(&storage_path);
    let (row, path) = storage
        .with_conn(|conn| crate::attachments::load_attachment_meta(conn, &root, attachment_ulid))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("read_attachment".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "attachment_ulid": ulid::to_crockford(attachment_ulid),
                }))?),
                rows_returned: Some(1),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok((row, path))
}

/// Aggregation operator for [`aggregate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateOp {
    /// Mean.
    Avg,
    /// Sum.
    Sum,
    /// Minimum.
    Min,
    /// Maximum.
    Max,
    /// Number of samples.
    Count,
    /// Median (P50).
    Median,
    /// 95th percentile.
    P95,
    /// 99th percentile.
    P99,
    /// Sample standard deviation (Bessel-corrected, n-1 divisor).
    StdDev,
}

impl AggregateOp {
    /// Apply over a slice of values, returning the result. Empty input returns
    /// 0 for sum / count, and is rejected for min/max/avg/median/percentile/
    /// stddev (callers should check `count > 0` first).
    pub fn apply(self, values: &[f64]) -> Option<f64> {
        if values.is_empty() {
            return match self {
                AggregateOp::Sum | AggregateOp::Count => Some(0.0),
                _ => None,
            };
        }
        let mut sorted: Vec<f64> = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        match self {
            AggregateOp::Avg => Some(values.iter().sum::<f64>() / values.len() as f64),
            AggregateOp::Sum => Some(values.iter().sum()),
            AggregateOp::Min => Some(*sorted.first().unwrap()),
            AggregateOp::Max => Some(*sorted.last().unwrap()),
            AggregateOp::Count => Some(values.len() as f64),
            AggregateOp::Median => Some(percentile(&sorted, 0.5)),
            AggregateOp::P95 => Some(percentile(&sorted, 0.95)),
            AggregateOp::P99 => Some(percentile(&sorted, 0.99)),
            AggregateOp::StdDev => {
                if values.len() < 2 {
                    return Some(0.0);
                }
                let mean = values.iter().sum::<f64>() / values.len() as f64;
                let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                    / (values.len() - 1) as f64;
                Some(var.sqrt())
            }
        }
    }
}

/// Linear-interpolated percentile on a pre-sorted slice. `q` in `[0.0, 1.0]`.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let n = sorted.len();
    if n == 1 {
        return sorted[0];
    }
    let pos = q * (n - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = pos - lo as f64;
        sorted[lo] * (1.0 - frac) + sorted[hi] * frac
    }
}

/// One bucket result from [`aggregate`].
#[derive(Debug, Clone)]
pub struct AggregateBucketResult {
    /// Inclusive lower bound (Unix ms).
    pub bucket_start_ms: i64,
    /// Exclusive upper bound (Unix ms).
    pub bucket_end_ms: i64,
    /// Number of underlying samples in the bucket.
    pub sample_count: i64,
    /// Aggregated value.
    pub value: f64,
}

/// `OhdcService.Aggregate`. Buckets matching events by `bucket_ms` (fixed
/// duration) and applies `op` over each bucket's values for `channel_path`.
///
/// `bucket_ms = 0` means "single bucket spanning the whole filter range".
///
/// Aggregations honour the grant scope: events that the grant can't see are
/// dropped from the input set (grant-scope filtering happens inside
/// `query_events`); blocked-by-aggregation_only is allowed (this is the only
/// op aggregation_only grants can call). `strip_notes` is irrelevant for
/// numeric aggregates.
pub fn aggregate(
    storage: &Storage,
    token: &ResolvedToken,
    channel_path: &str,
    filter: &EventFilter,
    op: AggregateOp,
    bucket_ms: i64,
) -> Result<Vec<AggregateBucketResult>> {
    auth::check_kind_for_op(token, OhdcOp::Aggregate)?;
    let scope = grant_scope_for(storage, token)?;
    // Aggregate is the explicitly-allowed op for aggregation_only grants —
    // no extra block here.
    let envelope = storage.envelope_key().cloned();
    let (events_out, _filtered) = storage.with_conn(|conn| {
        events::query_events_with_key(conn, filter, scope.as_ref(), envelope.as_ref())
    })?;
    let mut buckets: std::collections::BTreeMap<i64, (Vec<f64>, i64, i64)> =
        std::collections::BTreeMap::new();
    for e in &events_out {
        let value = match e.channels.iter().find(|c| c.channel_path == channel_path) {
            Some(cv) => match &cv.value {
                ChannelScalar::Real { real_value } => *real_value,
                ChannelScalar::Int { int_value } => *int_value as f64,
                ChannelScalar::Bool { bool_value } => *bool_value as i64 as f64,
                ChannelScalar::EnumOrdinal { enum_ordinal } => *enum_ordinal as f64,
                ChannelScalar::Text { .. } => continue, // text isn't numeric
            },
            None => continue,
        };
        let key = if bucket_ms <= 0 {
            0
        } else {
            (e.timestamp_ms / bucket_ms) * bucket_ms
        };
        let entry = buckets
            .entry(key)
            .or_insert_with(|| (vec![], i64::MAX, i64::MIN));
        entry.0.push(value);
        entry.1 = entry.1.min(e.timestamp_ms);
        entry.2 = entry.2.max(e.timestamp_ms);
    }
    let mut out = Vec::with_capacity(buckets.len());
    for (key, (values, _t_min, _t_max)) in buckets {
        let count = values.len() as i64;
        let agg = op.apply(&values).unwrap_or(0.0);
        let (start_ms, end_ms) = if bucket_ms <= 0 {
            (
                filter.from_ms.unwrap_or(0),
                filter.to_ms.unwrap_or(i64::MAX),
            )
        } else {
            (key, key + bucket_ms)
        };
        out.push(AggregateBucketResult {
            bucket_start_ms: start_ms,
            bucket_end_ms: end_ms,
            sample_count: count,
            value: agg,
        });
    }
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("aggregate".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "channel_path": channel_path,
                    "op": op.code(),
                    "bucket_ms": bucket_ms,
                    "from_ms": filter.from_ms,
                    "to_ms": filter.to_ms,
                }))?),
                rows_returned: Some(out.len() as i64),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(out)
}

impl AggregateOp {
    /// Wire string form (matches the proto enum names).
    pub fn code(self) -> &'static str {
        match self {
            AggregateOp::Avg => "AVG",
            AggregateOp::Sum => "SUM",
            AggregateOp::Min => "MIN",
            AggregateOp::Max => "MAX",
            AggregateOp::Count => "COUNT",
            AggregateOp::Median => "MEDIAN",
            AggregateOp::P95 => "P95",
            AggregateOp::P99 => "P99",
            AggregateOp::StdDev => "STDDEV",
        }
    }
}

/// One pair from [`correlate`].
#[derive(Debug, Clone)]
pub struct CorrelatePair {
    /// Reference (a) event ULID.
    pub a_ulid: String,
    /// Reference (a) timestamp.
    pub a_time_ms: i64,
    /// All `b`-side matches within the window.
    pub matches: Vec<CorrelateMatch>,
}

/// One b-side match in a [`CorrelatePair`].
#[derive(Debug, Clone)]
pub struct CorrelateMatch {
    /// b-side event ULID.
    pub b_ulid: String,
    /// b-side timestamp.
    pub b_time_ms: i64,
    /// b-side value (when the side specifies a channel path).
    pub b_value: Option<f64>,
}

/// Aggregate stats from [`correlate`].
#[derive(Debug, Clone)]
pub struct CorrelateStats {
    /// Count of a-side events.
    pub a_count: i64,
    /// Count of b-side events.
    pub b_count: i64,
    /// Count of pairs.
    pub paired_count: i64,
    /// Mean of paired b values, when b-side carries a numeric channel.
    pub mean_b_value: Option<f64>,
    /// Mean signed lag (b_time - a_time) in ms across pairs.
    pub mean_lag_ms: Option<f64>,
}

/// One side spec for [`correlate`]. Either an event-type name or a single
/// channel path matched against the post-query event set.
#[derive(Debug, Clone)]
pub enum CorrelateSide {
    /// Match by event type.
    EventType(String),
    /// Match by channel path on any event type.
    ChannelPath(String),
}

/// `OhdcService.Correlate`. Finds, for each event matching the `a`-side, the
/// `b`-side events whose timestamp falls within `± window_ms / 2` (i.e. the
/// `b` event is at most `window_ms / 2` from `a` in either direction).
///
/// The wire's `window` is treated as a symmetric window (half on either side
/// of `a`'s timestamp) — pinning the simple, deterministic interpretation
/// rather than introducing a directionality flag. v1.x can refine if the
/// conformance corpus surfaces a different convention.
///
/// Returned pairs are keyed by `a_ulid`; b-side values are filled when the
/// `b`-spec is a channel path.
pub fn correlate(
    storage: &Storage,
    token: &ResolvedToken,
    a: &CorrelateSide,
    b: &CorrelateSide,
    window_ms: i64,
    scope_filter: &EventFilter,
) -> Result<(Vec<CorrelatePair>, CorrelateStats)> {
    auth::check_kind_for_op(token, OhdcOp::Correlate)?;
    let scope = grant_scope_for(storage, token)?;
    let envelope = storage.envelope_key().cloned();
    let (a_events, _) = storage.with_conn(|conn| {
        events::query_events_with_key(
            conn,
            &filter_for_side(a, scope_filter),
            scope.as_ref(),
            envelope.as_ref(),
        )
    })?;
    let (b_events, _) = storage.with_conn(|conn| {
        events::query_events_with_key(
            conn,
            &filter_for_side(b, scope_filter),
            scope.as_ref(),
            envelope.as_ref(),
        )
    })?;

    let half = window_ms.max(0) / 2;
    let mut pairs: Vec<CorrelatePair> = Vec::new();
    let mut paired_count: i64 = 0;
    let mut sum_b_value: f64 = 0.0;
    let mut count_b_value: i64 = 0;
    let mut sum_lag: f64 = 0.0;
    let mut count_lag: i64 = 0;
    for ae in &a_events {
        let mut matches = Vec::new();
        for be in &b_events {
            if (be.timestamp_ms - ae.timestamp_ms).abs() <= half {
                let b_value = match b {
                    CorrelateSide::ChannelPath(path) => be
                        .channels
                        .iter()
                        .find(|c| &c.channel_path == path)
                        .and_then(|cv| match &cv.value {
                            ChannelScalar::Real { real_value } => Some(*real_value),
                            ChannelScalar::Int { int_value } => Some(*int_value as f64),
                            ChannelScalar::EnumOrdinal { enum_ordinal } => {
                                Some(*enum_ordinal as f64)
                            }
                            ChannelScalar::Bool { bool_value } => Some(*bool_value as i64 as f64),
                            ChannelScalar::Text { .. } => None,
                        }),
                    CorrelateSide::EventType(_) => None,
                };
                if let Some(v) = b_value {
                    sum_b_value += v;
                    count_b_value += 1;
                }
                sum_lag += (be.timestamp_ms - ae.timestamp_ms) as f64;
                count_lag += 1;
                paired_count += 1;
                matches.push(CorrelateMatch {
                    b_ulid: be.ulid.clone(),
                    b_time_ms: be.timestamp_ms,
                    b_value,
                });
            }
        }
        if !matches.is_empty() {
            pairs.push(CorrelatePair {
                a_ulid: ae.ulid.clone(),
                a_time_ms: ae.timestamp_ms,
                matches,
            });
        }
    }
    let stats = CorrelateStats {
        a_count: a_events.len() as i64,
        b_count: b_events.len() as i64,
        paired_count,
        mean_b_value: if count_b_value > 0 {
            Some(sum_b_value / count_b_value as f64)
        } else {
            None
        },
        mean_lag_ms: if count_lag > 0 {
            Some(sum_lag / count_lag as f64)
        } else {
            None
        },
    };
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("correlate".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "a": correlate_side_repr(a),
                    "b": correlate_side_repr(b),
                    "window_ms": window_ms,
                }))?),
                rows_returned: Some(stats.paired_count),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok((pairs, stats))
}

fn filter_for_side(side: &CorrelateSide, scope: &EventFilter) -> EventFilter {
    let mut out = scope.clone();
    match side {
        CorrelateSide::EventType(et) => {
            out.event_types_in = vec![et.clone()];
        }
        CorrelateSide::ChannelPath(_) => {
            // Channel-spec leaves the event-type filter alone; the post-query
            // pass picks the channel by name.
        }
    }
    out
}

fn correlate_side_repr(side: &CorrelateSide) -> serde_json::Value {
    match side {
        CorrelateSide::EventType(et) => serde_json::json!({"event_type": et}),
        CorrelateSide::ChannelPath(p) => serde_json::json!({"channel_path": p}),
    }
}

/// `OhdcService.AuditQuery`. Self-session sees all rows; grant tokens see
/// only their own (filtered to `grant_id = token.grant_id`).
pub fn audit_query(
    storage: &Storage,
    token: &ResolvedToken,
    q: &audit::AuditQuery,
) -> Result<Vec<audit::AuditEntry>> {
    auth::check_kind_for_op(token, OhdcOp::AuditQuery)?;
    let mut q = q.clone();
    if token.kind == TokenKind::Grant {
        // Grant tokens can only see their own audit rows. Override grant_id.
        q.grant_id = token.grant_id;
    }
    let rows = storage.with_conn(|conn| audit::query(conn, &q))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: actor_type_for(token.kind),
                auto_granted: false,
                grant_id: token.grant_id,
                action: "read".into(),
                query_kind: Some("audit_query".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "from_ms": q.from_ms,
                    "to_ms": q.to_ms,
                    "actor_type": q.actor_type,
                    "action": q.action,
                    "result": q.result,
                }))?),
                rows_returned: Some(rows.len() as i64),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(rows)
}

/// One frame in the [`export`] stream. Mirrors the proto's `ExportFrame`
/// oneof but flattened to a tagged enum for the in-process API.
///
/// Not serde-derived: serialization happens via the wire-side `ExportChunk`
/// proto which the server-side adapter constructs frame by frame.
#[derive(Debug, Clone)]
pub enum ExportFrame {
    /// Manifest header (always first).
    Init {
        /// Format version (matches `_meta.format_version`).
        format_version: String,
        /// Source instance pubkey hex (placeholder — encryption hierarchy v1.x).
        source_instance_pubkey_hex: String,
    },
    /// One event row.
    Event(Event),
    /// One grant row.
    Grant(GrantRow),
    /// One audit row.
    Audit(audit::AuditEntry),
    /// Trailer (always last; carries a deterministic signature placeholder).
    Finish {
        /// Total events emitted.
        events_emitted: i64,
    },
}

/// `OhdcService.Export`. Returns the export frames as a vec — the streaming
/// transport layer pumps these one at a time. Self-session only.
///
/// Encryption + Ed25519 signing are placeholders for v1.x; today the export
/// is unsigned and unencrypted (callers wanting encryption-at-rest can wrap
/// the frames in `age` / `gpg` themselves).
pub fn export(
    storage: &Storage,
    token: &ResolvedToken,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    include_event_types: &[String],
) -> Result<Vec<ExportFrame>> {
    auth::check_kind_for_op(token, OhdcOp::Export)?;
    if token.kind != TokenKind::SelfSession {
        return Err(Error::WrongTokenKind("export requires self-session"));
    }
    let mut out: Vec<ExportFrame> = Vec::new();
    out.push(ExportFrame::Init {
        format_version: crate::FORMAT_VERSION.to_string(),
        source_instance_pubkey_hex: String::new(),
    });
    let filter = EventFilter {
        from_ms,
        to_ms,
        event_types_in: include_event_types.to_vec(),
        include_deleted: true,
        include_superseded: true,
        ..Default::default()
    };
    let envelope = storage.envelope_key().cloned();
    let (events_out, _) = storage
        .with_conn(|conn| events::query_events_with_key(conn, &filter, None, envelope.as_ref()))?;
    for e in &events_out {
        out.push(ExportFrame::Event(e.clone()));
    }
    let grants_rows = storage.with_conn(|conn| {
        grants::list_grants(
            conn,
            &ListGrantsFilter {
                include_revoked: true,
                include_expired: true,
                grantee_kind: None,
                only_grant_id: None,
                limit: Some(10_000),
            },
        )
    })?;
    for g in &grants_rows {
        out.push(ExportFrame::Grant(g.clone()));
    }
    let audit_rows = storage.with_conn(|conn| {
        audit::query(
            conn,
            &audit::AuditQuery {
                limit: Some(100_000),
                ..Default::default()
            },
        )
    })?;
    for a in &audit_rows {
        out.push(ExportFrame::Audit(a.clone()));
    }
    out.push(ExportFrame::Finish {
        events_emitted: events_out.len() as i64,
    });
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: audit::ActorType::Self_,
                auto_granted: false,
                grant_id: None,
                action: "export".into(),
                query_kind: Some("export".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "from_ms": from_ms,
                    "to_ms": to_ms,
                    "include_event_types": include_event_types,
                }))?),
                rows_returned: Some(events_out.len() as i64),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(out)
}

/// Result of an [`import`] call.
#[derive(Debug, Clone, Default)]
pub struct ImportOutcome {
    /// Events imported (new ULIDs).
    pub events_imported: i64,
    /// Grants imported.
    pub grants_imported: i64,
    /// Audit rows imported.
    pub audit_entries_imported: i64,
    /// Non-fatal warnings.
    pub warnings: Vec<String>,
}

/// `OhdcService.Import`. Idempotent on event ULIDs (existing rows skipped).
/// Self-session only.
pub fn import(
    storage: &Storage,
    token: &ResolvedToken,
    frames: &[ExportFrame],
) -> Result<ImportOutcome> {
    auth::check_kind_for_op(token, OhdcOp::Import)?;
    if token.kind != TokenKind::SelfSession {
        return Err(Error::WrongTokenKind("import requires self-session"));
    }
    let mut outcome = ImportOutcome::default();
    storage.with_conn_mut(|conn| {
        let tx = conn.transaction()?;
        for frame in frames {
            match frame {
                ExportFrame::Init { format_version, .. } => {
                    if format_version != crate::FORMAT_VERSION {
                        outcome.warnings.push(format!(
                            "import.format_version {format_version} != {}",
                            crate::FORMAT_VERSION
                        ));
                    }
                }
                ExportFrame::Event(e) => {
                    let imported = import_one_event(&tx, e)?;
                    if imported {
                        outcome.events_imported += 1;
                    }
                }
                ExportFrame::Grant(_g) => {
                    // Grants are out-of-band per spec; export carries them for
                    // reconciliation but import doesn't recreate them (the user
                    // re-issues at the destination instance to get new tokens).
                    outcome.grants_imported += 1;
                }
                ExportFrame::Audit(a) => {
                    audit::append(&tx, a)?;
                    outcome.audit_entries_imported += 1;
                }
                ExportFrame::Finish { .. } => {}
            }
        }
        tx.commit()?;
        Ok(())
    })?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: audit::ActorType::Self_,
                auto_granted: false,
                grant_id: None,
                action: "import".into(),
                query_kind: Some("import".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "events_imported": outcome.events_imported,
                    "grants_imported": outcome.grants_imported,
                    "audit_entries_imported": outcome.audit_entries_imported,
                }))?),
                rows_returned: Some(outcome.events_imported),
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(outcome)
}

fn import_one_event(tx: &rusqlite::Transaction<'_>, e: &Event) -> Result<bool> {
    use crate::registry::EventTypeName;
    let etn = EventTypeName::parse(&e.event_type)?;
    let etype = crate::registry::resolve_event_type(tx, &etn)?;
    let parsed_ulid = ulid::parse_crockford(&e.ulid)?;
    let rand_tail = ulid::random_tail(&parsed_ulid);
    // Idempotency: skip if the ULID already exists (export is intended to
    // round-trip, so re-importing is a no-op).
    let exists: Option<i64> = tx
        .query_row(
            "SELECT id FROM events WHERE ulid_random = ?1",
            rusqlite::params![rand_tail.to_vec()],
            |r| r.get(0),
        )
        .ok();
    if exists.is_some() {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO events
            (ulid_random, timestamp_ms, tz_offset_minutes, tz_name, duration_ms,
             event_type_id, source, source_id, notes, deleted_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            rand_tail.to_vec(),
            e.timestamp_ms,
            e.tz_offset_minutes,
            e.tz_name,
            e.duration_ms,
            etype.id,
            e.source,
            e.source_id,
            e.notes,
            e.deleted_at_ms,
        ],
    )?;
    let event_rowid = tx.last_insert_rowid();
    for cv in &e.channels {
        let chan = match crate::registry::resolve_channel(tx, etype.id, &cv.channel_path) {
            Ok(c) => c,
            Err(_) => continue, // unknown channel — skip silently
        };
        let (vr, vi, vt, ve): (Option<f64>, Option<i64>, Option<String>, Option<i32>) = match &cv
            .value
        {
            ChannelScalar::Real { real_value } => (Some(*real_value), None, None, None),
            ChannelScalar::Int { int_value } => (None, Some(*int_value), None, None),
            ChannelScalar::Bool { bool_value } => (None, Some(*bool_value as i64), None, None),
            ChannelScalar::Text { text_value } => (None, None, Some(text_value.clone()), None),
            ChannelScalar::EnumOrdinal { enum_ordinal } => (None, None, None, Some(*enum_ordinal)),
        };
        tx.execute(
            "INSERT INTO event_channels
                (event_id, channel_id, value_real, value_int, value_text, value_enum)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![event_rowid, chan.id, vr, vi, vt, ve],
        )?;
    }
    Ok(true)
}

// =============================================================================
// Source signing — operator registry RPCs (self-session only).
// =============================================================================

/// `OhdcService.RegisterSigner`. Self-session only; the per-integration
/// signing key is operator-managed state.
pub fn register_signer(
    storage: &Storage,
    token: &ResolvedToken,
    signer_kid: &str,
    signer_label: &str,
    sig_alg: &str,
    public_key_pem: &str,
) -> Result<crate::source_signing::Signer> {
    if token.kind != TokenKind::SelfSession {
        return Err(Error::WrongTokenKind(
            "register_signer requires self-session",
        ));
    }
    let signer = storage.with_conn(|conn| {
        crate::source_signing::register_signer(
            conn,
            signer_kid,
            signer_label,
            sig_alg,
            public_key_pem,
        )
    })?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: audit::ActorType::Self_,
                auto_granted: false,
                grant_id: None,
                action: "signer_register".into(),
                query_kind: Some("register_signer".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "signer_kid": signer_kid,
                    "sig_alg": sig_alg,
                }))?),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(signer)
}

/// `OhdcService.ListSigners`. Self-session only.
pub fn list_signers(
    storage: &Storage,
    token: &ResolvedToken,
) -> Result<Vec<crate::source_signing::Signer>> {
    if token.kind != TokenKind::SelfSession {
        return Err(Error::WrongTokenKind("list_signers requires self-session"));
    }
    storage.with_conn(|conn| crate::source_signing::list_signers(conn))
}

/// `OhdcService.RevokeSigner`. Self-session only.
pub fn revoke_signer(storage: &Storage, token: &ResolvedToken, signer_kid: &str) -> Result<i64> {
    if token.kind != TokenKind::SelfSession {
        return Err(Error::WrongTokenKind("revoke_signer requires self-session"));
    }
    let revoked_at_ms =
        storage.with_conn(|conn| crate::source_signing::revoke_signer(conn, signer_kid))?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: audit::ActorType::Self_,
                auto_granted: false,
                grant_id: None,
                action: "signer_revoke".into(),
                query_kind: Some("revoke_signer".into()),
                query_params_json: Some(serde_json::to_string(&serde_json::json!({
                    "signer_kid": signer_kid,
                }))?),
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(revoked_at_ms)
}

// =============================================================================
// Retrospective grant: a grant created against an existing case (e.g. patient
// reviews a closed case after the fact and decides to grant a clinician
// access to the case's scope). Wraps `create_grant` + `cases::bind_grant_to_cases`.
// =============================================================================

/// `OhdcService.IssueRetrospectiveGrant`-equivalent helper. Creates a new
/// grant with the supplied policy + binds it to `case_ulid` so its read
/// scope is the case's recursive scope intersected with the grant's rules.
///
/// Self-session only — the patient is the only authority that can issue a
/// new grant after the fact.
pub fn issue_retrospective_grant(
    storage: &Storage,
    token: &ResolvedToken,
    case_ulid: &Ulid,
    g: &NewGrant,
) -> Result<CreateGrantOutcome> {
    if token.kind != TokenKind::SelfSession {
        return Err(Error::WrongTokenKind(
            "issue_retrospective_grant requires self-session",
        ));
    }
    // Resolve the case first so a typo doesn't create an orphan grant.
    let case_id = storage.with_conn(|conn| cases::case_id_by_ulid(conn, case_ulid))?;
    let outcome = create_grant(storage, token, g)?;
    storage.with_conn(|conn| cases::bind_grant_to_cases(conn, outcome.grant.id, &[case_id]))?;
    Ok(outcome)
}

// =============================================================================
// Emergency config (operator-side) — get / set wrappers, self-session only.
// =============================================================================

/// `OhdcService.GetEmergencyConfig` (operator state). Self-session only.
pub fn get_emergency_config(
    storage: &Storage,
    token: &ResolvedToken,
) -> Result<crate::emergency_config::EmergencyConfig> {
    if token.kind != TokenKind::SelfSession {
        return Err(Error::WrongTokenKind(
            "get_emergency_config requires self-session",
        ));
    }
    let user_ulid = token.effective_user_ulid();
    storage.with_conn(|conn| crate::emergency_config::get_emergency_config(conn, user_ulid))
}

/// `OhdcService.SetEmergencyConfig` (operator state). Self-session only.
pub fn set_emergency_config(
    storage: &Storage,
    token: &ResolvedToken,
    cfg: &crate::emergency_config::EmergencyConfig,
) -> Result<()> {
    if token.kind != TokenKind::SelfSession {
        return Err(Error::WrongTokenKind(
            "set_emergency_config requires self-session",
        ));
    }
    let user_ulid = token.effective_user_ulid();
    let now = audit::now_ms();
    storage.with_conn(|conn| {
        crate::emergency_config::set_emergency_config(conn, user_ulid, cfg, now)
    })?;
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: now,
                actor_type: audit::ActorType::Self_,
                auto_granted: false,
                grant_id: None,
                action: "emergency_config_set".into(),
                query_kind: Some("set_emergency_config".into()),
                query_params_json: None,
                rows_returned: None,
                rows_filtered: None,
                result: AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;
    Ok(())
}

// =============================================================================
// Export-as-bytes — a CBOR-serialized snapshot of the ExportFrame stream.
// =============================================================================

/// Serialize the full export to a single byte buffer. Self-session only.
///
/// Produces a CBOR array of `ExportFrameWire` objects (a thin serde-friendly
/// mirror of [`ExportFrame`]), suitable for writing to a portable `.ohd`
/// file. Uniffi/PyO3 callers consume the bytes opaquely; the round-trip
/// path through `import` reads the same envelope.
pub fn export_all(
    storage: &Storage,
    token: &ResolvedToken,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    include_event_types: &[String],
) -> Result<Vec<u8>> {
    let frames = export(storage, token, from_ms, to_ms, include_event_types)?;
    let wire: Vec<ExportFrameWire> = frames
        .into_iter()
        .map(ExportFrameWire::from_frame)
        .collect();
    let mut buf = Vec::with_capacity(4096);
    ciborium::ser::into_writer(&wire, &mut buf)
        .map_err(|e| Error::Internal(anyhow::anyhow!("CBOR encode export: {e}")))?;
    Ok(buf)
}

/// Wire-friendly mirror of [`ExportFrame`] for [`export_all`]. Uses serde's
/// internal tagging so the encoded byte stream is self-describing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
enum ExportFrameWire {
    #[serde(rename = "init")]
    Init {
        format_version: String,
        source_instance_pubkey_hex: String,
    },
    #[serde(rename = "event")]
    Event(Event),
    #[serde(rename = "audit")]
    Audit(audit::AuditEntry),
    #[serde(rename = "finish")]
    Finish { events_emitted: i64 },
    /// Grant rows are best-effort in the in-process `import` path; we serialize
    /// a placeholder ULID + label so the wire shape round-trips even though
    /// `import` ignores the contents.
    #[serde(rename = "grant")]
    Grant {
        ulid_crockford: String,
        label: String,
    },
}

impl ExportFrameWire {
    fn from_frame(f: ExportFrame) -> Self {
        match f {
            ExportFrame::Init {
                format_version,
                source_instance_pubkey_hex,
            } => ExportFrameWire::Init {
                format_version,
                source_instance_pubkey_hex,
            },
            ExportFrame::Event(e) => ExportFrameWire::Event(e),
            ExportFrame::Grant(g) => ExportFrameWire::Grant {
                ulid_crockford: ulid::to_crockford(&g.ulid),
                label: g.grantee_label,
            },
            ExportFrame::Audit(a) => ExportFrameWire::Audit(a),
            ExportFrame::Finish { events_emitted } => ExportFrameWire::Finish { events_emitted },
        }
    }
}

// audit::AuditEntry needs serde derives for `ExportFrameWire::Audit` — they
// already exist (see `audit.rs`).
