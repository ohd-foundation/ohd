"""Real Python OHDC client (Connect-RPC over HTTP/2) for Care MCP.

Hand-rolled on top of :class:`ohd_shared.connect_transport.OhdcTransport`
and the protobuf message types in ``ohd_shared._gen.ohdc.v0.ohdc_pb2``.
The Connect-RPC transport and the proto<->dict helpers live in the
``ohd-shared`` workspace package; this module is the Care-specific
client surface (multi-patient, per-call grant tokens, audit stamping).

Care MCP is multi-patient: every method takes ``grant_token`` as a kwarg
and the transport attaches it as ``Authorization: Bearer {grant_token}``
on that call. The token is *not* stored on the client — the active grant
changes when the operator calls ``switch_patient``, so per-call routing
keeps the client stateless w.r.t. the patient.

Care MCP does NOT need ``create_grant`` / ``revoke_grant`` (Care holds
grants, doesn't issue them) or ``audit_query`` (storage's AuditQuery is
``Unimplemented`` in v1). The aggregate / correlate / find_relevant_context
helpers are also surfaced as ``OhdcNotWiredError`` because the underlying
storage handlers are still ``Unimplemented`` — the LLM gets a clear
"not yet wired" rather than a wire-level ``unimplemented`` error.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ohd_shared.connect_transport import OhdcRpcError, OhdcTransport
from ohd_shared.ohdc_helpers import (
    build_filter,
    case_to_dict,
    event_input_from_dict,
    event_to_dict,
    pb,
    pending_to_dict,
    put_result_to_dict,
    ulid_bytes_to_crockford,
    ulid_msg,
)

from .canonical_query_hash import canonical_query_hash
from .operator_audit import (
    append_operator_audit_entry,
    build_audit_template,
)

# Re-export the underscore-prefixed names that lived in this module's
# original copy. Internal call sites in this package may use either spelling;
# external callers (tests, type-checkers) only ever touch the public class
# surface below.
_ulid_bytes_to_crockford = ulid_bytes_to_crockford
_ulid_msg = ulid_msg
_event_to_dict = event_to_dict
_event_input_from_dict = event_input_from_dict
_pending_to_dict = pending_to_dict
_case_to_dict = case_to_dict
_put_result_to_dict = put_result_to_dict
_build_filter = build_filter

_SERVICE = "ohdc.v0.OhdcService"


class OhdcNotWiredError(NotImplementedError):
    """Raised when the OHDC client method is not yet implemented.

    Either the underlying storage RPC is still stubbed (``Aggregate``,
    ``Correlate``, ``find_relevant_context``) or the client method is a
    v0.x deferred surface.
    """

    def __init__(self, method: str, *, reason: str | None = None) -> None:
        super().__init__(
            f"OHDC client method {method!r} is not yet wired"
            + (f": {reason}" if reason else "")
            + ". See care/mcp/STATUS.md 'OHDC client — wire status'."
        )


@dataclass(frozen=True)
class OhdcClientConfig:
    storage_url: str


class OhdcClient:
    """Async OHDC client over Connect-RPC for Care MCP.

    The transport is shared across patients; each method takes a
    ``grant_token`` kwarg which is attached as ``Authorization: Bearer``
    on that call. The vault upstream (``grant_vault.GrantVault``) supplies
    the token from the active patient.
    """

    def __init__(self, config: OhdcClientConfig) -> None:
        self._config = config
        self._transport = OhdcTransport(base_url=config.storage_url)

    @property
    def config(self) -> OhdcClientConfig:
        return self._config

    async def aclose(self) -> None:
        await self._transport.aclose()

    # --- Diagnostics ----------------------------------------------------

    async def who_am_i(self, *, grant_token: str) -> dict[str, Any]:
        """Return information about the supplied grant token.

        Useful as a probe — confirms the token is valid and shows the
        granted patient + scope summary. The Care MCP doesn't expose this
        as a tool today (the active patient is held in the vault), but
        callers (tests, future tools) may use it directly.
        """
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="WhoAmI",
            request=pb.WhoAmIRequest(),
            response_cls=pb.WhoAmIResponse,
            token=grant_token,
        )
        out: dict[str, Any] = {
            "user_ulid": ulid_bytes_to_crockford(resp.user_ulid.bytes)
            if resp.user_ulid.bytes
            else None,
            "token_kind": resp.token_kind,
            "caller_ip": resp.caller_ip,
        }
        if resp.HasField("grantee_label"):
            out["grantee_label"] = resp.grantee_label
        if resp.HasField("grant_ulid"):
            out["grant_ulid"] = ulid_bytes_to_crockford(resp.grant_ulid.bytes)
        return out

    # --- Read tools (active patient, gated by read scope) ---------------

    async def query_events(
        self,
        *,
        grant_token: str,
        event_type: str | None = None,
        from_time_ms: int | None = None,
        to_time_ms: int | None = None,
        limit: int | None = None,
        order: str = "desc",
    ) -> list[dict[str, Any]]:
        req = pb.QueryEventsRequest(
            filter=build_filter(
                event_type=event_type,
                from_time_ms=from_time_ms,
                to_time_ms=to_time_ms,
                limit=limit,
                order=order,
            )
        )
        # Compute the operator-side audit row's `query_hash` BEFORE the call
        # (per care/SPEC.md §7.3) so we record even rejected / errored paths.
        canonical_filter = {
            "from_ms": from_time_ms,
            "to_ms": to_time_ms,
            "event_types_in": [event_type] if event_type else [],
            "include_superseded": True,
            "limit": limit,
        }
        query_hash = canonical_query_hash("query_events", canonical_filter)
        template = build_audit_template(
            ohdc_action="query_events",
            query_kind="query_events",
            query_hash_hex=query_hash,
        )
        out: list[dict[str, Any]] = []
        try:
            async for ev in self._transport.call_server_streaming(
                service=_SERVICE,
                method="QueryEvents",
                request=req,
                response_cls=pb.Event,
                token=grant_token,
            ):
                out.append(event_to_dict(ev))
        except Exception as exc:  # OhdcRpcError, transport, etc.
            template.result = "error"  # type: ignore[assignment]
            template.reason = str(exc)
            append_operator_audit_entry(template)
            raise
        template.result = "success"  # type: ignore[assignment]
        template.rows_returned = len(out)
        append_operator_audit_entry(template)
        return out

    async def get_event_by_ulid(
        self, *, grant_token: str, ulid: str
    ) -> dict[str, Any]:
        # `get_event_by_ulid` keys on `query_kind="get_event_by_ulid"` with
        # an empty filter — mirrors storage's pending-query path.
        query_hash = canonical_query_hash("get_event_by_ulid", {})
        template = build_audit_template(
            ohdc_action="get_event_by_ulid",
            query_kind="get_event_by_ulid",
            query_hash_hex=query_hash,
        )
        req = pb.GetEventByUlidRequest(ulid=ulid_msg(ulid))
        try:
            resp = await self._transport.call_unary(
                service=_SERVICE,
                method="GetEventByUlid",
                request=req,
                response_cls=pb.Event,
                token=grant_token,
            )
        except Exception as exc:
            template.result = "error"  # type: ignore[assignment]
            template.reason = str(exc)
            append_operator_audit_entry(template)
            raise
        template.result = "success"  # type: ignore[assignment]
        template.rows_returned = 1
        append_operator_audit_entry(template)
        return event_to_dict(resp)

    async def aggregate(
        self,
        *,
        grant_token: str,
        event_type: str,
        period: str,
        aggregation: str,
        from_time_ms: int | None = None,
        to_time_ms: int | None = None,
    ) -> list[dict[str, Any]]:
        # Storage's Aggregate handler returns Unimplemented today.
        raise OhdcNotWiredError(
            "aggregate", reason="storage Aggregate handler is not wired (storage v1.x)"
        )

    async def correlate(
        self,
        *,
        grant_token: str,
        event_type_a: str,
        event_type_b: str,
        window_minutes: int,
        from_time_ms: int | None = None,
        to_time_ms: int | None = None,
    ) -> dict[str, Any]:
        raise OhdcNotWiredError(
            "correlate", reason="storage Correlate handler is not wired (storage v1.x)"
        )

    # --- Write tools (active patient, gated by write scope) -------------

    async def put_events(
        self,
        *,
        grant_token: str,
        events: list[dict[str, Any]],
    ) -> dict[str, Any]:
        """Submit clinical events for the active patient.

        Storage routes to ``pending_events`` if the grant's approval policy
        is ``always`` for this event type, or directly to ``events`` if it's
        ``auto_for_event_types`` and the type is on the allowlist.
        """
        req = pb.PutEventsRequest(
            events=[event_input_from_dict(e) for e in events],
            atomic=False,
        )
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="PutEvents",
            request=req,
            response_cls=pb.PutEventsResponse,
            token=grant_token,
        )
        return {"results": [put_result_to_dict(r) for r in resp.results]}

    # --- Cases (per care/SPEC.md §4 + §10.5) ----------------------------

    async def open_case(
        self,
        *,
        grant_token: str,
        case_type: str,
        case_label: str | None = None,
        predecessor_case_ulid: str | None = None,
        parent_case_ulid: str | None = None,
        inactivity_close_after_h: int | None = None,
    ) -> dict[str, Any]:
        """Open a new case for the active patient.

        Care SPEC's ``open_case`` maps to OHDC ``CreateCase``: storage
        records a ``case_started`` marker. The optional ``predecessor`` /
        ``parent`` ULIDs encode the inheritance lattice from §4.2.
        """
        req = pb.CreateCaseRequest(case_type=case_type)
        if case_label is not None:
            req.case_label = case_label
        if predecessor_case_ulid is not None:
            req.predecessor_case_ulid.CopyFrom(ulid_msg(predecessor_case_ulid))
        if parent_case_ulid is not None:
            req.parent_case_ulid.CopyFrom(ulid_msg(parent_case_ulid))
        if inactivity_close_after_h is not None:
            req.inactivity_close_after_h = int(inactivity_close_after_h)
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="CreateCase",
            request=req,
            response_cls=pb.Case,
            token=grant_token,
        )
        return case_to_dict(resp)

    async def close_case(
        self,
        *,
        grant_token: str,
        case_ulid: str,
        reason: str | None = None,
    ) -> dict[str, Any]:
        """Close a case for the active patient.

        Per SPEC §4.1: storage records a ``case_closed`` marker.
        Operator retains read-only access to the case's span (filters
        at close time) for records / billing / follow-up.
        """
        req = pb.CloseCaseRequest(case_ulid=ulid_msg(case_ulid))
        if reason is not None:
            req.reason = reason
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="CloseCase",
            request=req,
            response_cls=pb.Case,
            token=grant_token,
        )
        return case_to_dict(resp)

    async def list_cases(
        self,
        *,
        grant_token: str,
        include_closed: bool = True,
        case_type: str | None = None,
    ) -> list[dict[str, Any]]:
        """List cases for the active patient.

        SPEC §4.1. ``include_closed`` defaults to True so the operator
        sees both open and recently-closed cases (closed cases stay
        retrievable for the records-retention window).
        """
        req = pb.ListCasesRequest(include_closed=include_closed)
        if case_type is not None:
            req.case_type = case_type
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="ListCases",
            request=req,
            response_cls=pb.ListCasesResponse,
            token=grant_token,
        )
        return [case_to_dict(c) for c in resp.cases]

    async def get_case(
        self,
        *,
        grant_token: str,
        case_ulid: str,
    ) -> dict[str, Any]:
        """Get one case's full record (timeline + audit + handoff chain)."""
        req = pb.GetCaseRequest(case_ulid=ulid_msg(case_ulid))
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="GetCase",
            request=req,
            response_cls=pb.Case,
            token=grant_token,
        )
        return case_to_dict(resp)

    async def issue_retrospective_grant(
        self,
        *,
        grant_token: str,
        case_ulid: str,
        grantee_label: str,
        scope_event_types: list[str],
        expires_days: int,
    ) -> dict[str, Any]:
        """Issue a retrospective case-scoped grant per SPEC §4.6.

        Retrospective access lets the operator delegate read access to
        a closed case to a specialist / insurer / researcher. Mechanically
        a regular ``CreateGrant`` with ``case_ulids = [case_ulid]`` and
        ``approval_mode = always`` so the patient retains the final say
        on per-query approvals (matches the §4.6 contract).

        v0 caveat: ``CreateGrant`` is documented as "self-session only" in
        the OHDC service comment (line 252), so storage may reject this
        call when invoked under a grant token. We surface a typed
        :class:`OhdcNotWiredError` if storage refuses, so the operator UI
        can render a clear "patient must issue this from OHD Connect"
        message rather than a wire-level ``permission_denied`` stack.
        """
        from datetime import datetime, timezone

        # Build the read-only event-type allowlist for the retrospective
        # grant. Default action 'deny' + per-type 'allow' gives a closed
        # scope (the §4.6 retrospective access is intentionally narrow).
        rules = [
            pb.GrantEventTypeRule(event_type=t, effect="allow")
            for t in scope_event_types
        ]
        expires_ms = int(
            (
                datetime.now(timezone.utc).timestamp() + expires_days * 86_400
            )
            * 1000
        )
        req = pb.CreateGrantRequest(
            grantee_label=grantee_label,
            grantee_kind="other",
            default_action="deny",
            approval_mode="always",
            event_type_rules=rules,
            expires_at_ms=expires_ms,
            case_ulids=[ulid_msg(case_ulid)],
        )
        try:
            resp = await self._transport.call_unary(
                service=_SERVICE,
                method="CreateGrant",
                request=req,
                response_cls=pb.CreateGrantResponse,
                token=grant_token,
            )
        except OhdcRpcError as exc:
            # `CreateGrant` is self-session only in the OHDC service surface;
            # surface a clear typed error so the caller can route the
            # request through OHD Connect instead.
            if exc.code in ("permission_denied", "unauthenticated", "unimplemented"):
                raise OhdcNotWiredError(
                    "issue_retrospective_grant",
                    reason=(
                        f"storage rejected operator-side CreateGrant "
                        f"({exc.code}); retrospective grants must be issued "
                        "by the patient from OHD Connect in v0.x"
                    ),
                ) from exc
            raise
        return {
            "grant_ulid": (
                ulid_bytes_to_crockford(resp.grant.ulid.bytes)
                if resp.HasField("grant") and resp.grant.ulid.bytes
                else None
            ),
            "share_url": resp.share_url,
            "token": resp.token,
            "expires_at_ms": (
                int(resp.grant.expires_at_ms) if resp.HasField("grant") else None
            ),
        }

    # --- Pending --------------------------------------------------------

    async def list_pending(
        self,
        *,
        grant_token: str,
    ) -> list[dict[str, Any]]:
        """List the operator's own queued submissions for the active patient.

        Filtered by ``submitting_grant_ulid`` server-side: storage scopes
        the list to writes the calling grant submitted, so Care operators
        only see their own pending writes (not the patient's other queued
        submissions from Connect / sync).
        """
        req = pb.ListPendingRequest(status="pending")
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="ListPending",
            request=req,
            response_cls=pb.ListPendingResponse,
            token=grant_token,
        )
        return [pending_to_dict(p) for p in resp.pending]

    # --- Workflow tools -------------------------------------------------

    async def find_relevant_context(
        self,
        *,
        grant_token: str,
        complaint: str,
    ) -> dict[str, Any]:
        """Pull visit-prep slices for a chief complaint.

        Composes ``query_events`` + a server-side classifier; the classifier
        side is not yet wired, so we surface OhdcNotWiredError until storage
        exposes it. Care SPEC §10.4.
        """
        raise OhdcNotWiredError(
            "find_relevant_context",
            reason=(
                "storage-side complaint classifier is not wired; v0.x will "
                "compose this client-side from query_events instead."
            ),
        )


__all__ = [
    "OhdcClient",
    "OhdcClientConfig",
    "OhdcNotWiredError",
    "OhdcRpcError",
]
