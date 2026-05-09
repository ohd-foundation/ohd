"""Real Python OHDC client (Connect-RPC over HTTP/2) for Connect MCP.

Hand-rolled on top of :class:`ohd_shared.connect_transport.OhdcTransport`
and the protobuf message types in ``ohd_shared._gen.ohdc.v0.ohdc_pb2``.
The Connect-RPC transport and the proto<->dict helpers live in the
``ohd-shared`` workspace package; this module is the Connect-MCP-specific
client surface (self-session token, owner-side surface).

Connect MCP uses a **self-session** token (env var ``OHD_ACCESS_TOKEN``)
and is wired against the OHDC operations a personal-data owner can drive
(reads, writes, grant CRUD, pending review, audit). The ~17 OHDC RPCs
that are still ``Unimplemented`` on the storage side are surfaced as
``OhdcNotWiredError`` here too — the LLM gets a clear "not yet wired"
rather than a wire-level ``unimplemented`` error.

Tool implementations in :mod:`tools` already build dict-shaped request /
response payloads; this module is responsible for translating those dicts
to/from protobuf messages.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ohd_shared.connect_transport import OhdcRpcError, OhdcTransport
from ohd_shared.ohdc_helpers import (
    build_filter,
    event_input_from_dict,
    event_to_dict,
    grant_to_dict,
    pb,
    pending_to_dict,
    put_result_to_dict,
    ulid_bytes_to_crockford,
    ulid_msg,
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
_grant_to_dict = grant_to_dict
_put_result_to_dict = put_result_to_dict
_build_filter = build_filter

_SERVICE = "ohdc.v0.OhdcService"


class OhdcNotWiredError(NotImplementedError):
    """Raised when the OHDC client method is not yet implemented.

    Either the underlying storage RPC is still stubbed (``Aggregate``,
    ``Correlate``, cases, retrospective grants, drug interactions) or the
    client method is a v0.x deferred surface (``find_patterns``,
    ``chart``).
    """

    def __init__(self, method: str, *, reason: str | None = None) -> None:
        super().__init__(
            f"OHDC client method {method!r} is not yet wired"
            + (f": {reason}" if reason else "")
            + ". See connect/mcp/STATUS.md 'OHDC client — wire status'."
        )


@dataclass(frozen=True)
class OhdcClientConfig:
    storage_url: str
    access_token: str | None


class OhdcClient:
    """Async OHDC client over Connect-RPC for Connect MCP.

    All methods use the ``access_token`` supplied at construction time as the
    ``Authorization: Bearer`` header. The Connect MCP is the personal-data
    owner's view, so this is a self-session token (``ohds_…``).
    """

    def __init__(self, config: OhdcClientConfig) -> None:
        self._config = config
        self._transport = OhdcTransport(
            base_url=config.storage_url,
            default_token=config.access_token,
        )

    @property
    def config(self) -> OhdcClientConfig:
        return self._config

    async def aclose(self) -> None:
        await self._transport.aclose()

    # --- Diagnostics ----------------------------------------------------

    async def health(self) -> dict[str, Any]:
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="Health",
            request=pb.HealthRequest(),
            response_cls=pb.HealthResponse,
        )
        return {
            "status": resp.status,
            "server_time_ms": resp.server_time_ms,
            "server_version": resp.server_version,
            "protocol_version": resp.protocol_version,
            "subsystems": dict(resp.subsystems),
        }

    async def who_am_i(self) -> dict[str, Any]:
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="WhoAmI",
            request=pb.WhoAmIRequest(),
            response_cls=pb.WhoAmIResponse,
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
        if resp.HasField("device_label"):
            out["device_label"] = resp.device_label
        return out

    # --- Events ---------------------------------------------------------

    async def put_events(self, events: list[dict[str, Any]]) -> dict[str, Any]:
        req = pb.PutEventsRequest(
            events=[event_input_from_dict(e) for e in events],
            atomic=False,
        )
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="PutEvents",
            request=req,
            response_cls=pb.PutEventsResponse,
        )
        return {"results": [put_result_to_dict(r) for r in resp.results]}

    async def query_events(
        self,
        *,
        event_type: str | None = None,
        from_time_ms: int | None = None,
        to_time_ms: int | None = None,
        limit: int | None = None,
        order: str = "desc",
        case_ulid: str | None = None,  # accepted for API stability; unused in v1
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
        out: list[dict[str, Any]] = []
        async for ev in self._transport.call_server_streaming(
            service=_SERVICE,
            method="QueryEvents",
            request=req,
            response_cls=pb.Event,
        ):
            out.append(event_to_dict(ev))
        return out

    async def get_event_by_ulid(self, *, ulid: str) -> dict[str, Any]:
        req = pb.GetEventByUlidRequest(ulid=ulid_msg(ulid))
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="GetEventByUlid",
            request=req,
            response_cls=pb.Event,
        )
        return event_to_dict(resp)

    async def aggregate(
        self,
        *,
        event_type: str,
        period: str,
        aggregation: str,
        from_time_ms: int | None = None,
        to_time_ms: int | None = None,
    ) -> list[dict[str, Any]]:
        # Storage's Aggregate handler returns Unimplemented today; surface
        # that as OhdcNotWiredError so the LLM sees a clear "not yet" rather
        # than a Connect-RPC-level unimplemented.
        raise OhdcNotWiredError(
            "aggregate", reason="storage Aggregate handler is not wired (storage v1.x)"
        )

    async def correlate(
        self,
        *,
        event_type_a: str,
        event_type_b: str,
        window_minutes: int,
        from_time_ms: int | None = None,
        to_time_ms: int | None = None,
    ) -> dict[str, Any]:
        raise OhdcNotWiredError(
            "correlate", reason="storage Correlate handler is not wired (storage v1.x)"
        )

    # --- Grants ---------------------------------------------------------

    async def create_grant(
        self, *, template_id: str, label: str, **overrides: Any
    ) -> dict[str, Any]:
        # Templates are Connect-MCP UX sugar over CreateGrant; we map the
        # named templates onto OHDC CreateGrantRequest field defaults. The
        # exact template policies live in connect/SPEC.md "Grant templates".
        req = pb.CreateGrantRequest(
            grantee_label=label,
            grantee_kind="human",
            default_action="deny",
            approval_mode="always",
        )
        if template_id == "primary_doctor":
            req.default_action = "allow"
            req.approval_mode = "auto_for_event_types"
        elif template_id == "specialist_visit":
            req.approval_mode = "always"
        elif template_id == "spouse_family":
            req.default_action = "allow"
            req.approval_mode = "never_required"
        elif template_id == "researcher_with_study":
            req.aggregation_only = True
            req.strip_notes = True
        elif template_id == "emergency_template":
            req.default_action = "allow"
            req.approval_mode = "never_required"
        # else: unknown template id — fall through with the deny+always default.

        if overrides.get("notes"):
            req.purpose = str(overrides["notes"])

        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="CreateGrant",
            request=req,
            response_cls=pb.CreateGrantResponse,
        )
        return {
            "grant": grant_to_dict(resp.grant),
            "token": resp.token,
            "share_url": resp.share_url,
        }

    async def list_grants(self, *, include_revoked: bool = False) -> list[dict[str, Any]]:
        req = pb.ListGrantsRequest(include_revoked=include_revoked)
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="ListGrants",
            request=req,
            response_cls=pb.ListGrantsResponse,
        )
        return [grant_to_dict(g) for g in resp.grants]

    async def revoke_grant(self, *, grant_id: str) -> dict[str, Any]:
        req = pb.RevokeGrantRequest(grant_ulid=ulid_msg(grant_id))
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="RevokeGrant",
            request=req,
            response_cls=pb.RevokeGrantResponse,
        )
        return {"revoked_at_ms": resp.revoked_at_ms}

    # --- Pending --------------------------------------------------------

    async def list_pending(self, *, grant_id: str | None = None) -> list[dict[str, Any]]:
        req = pb.ListPendingRequest(status="pending")
        if grant_id:
            req.submitting_grant_ulid.CopyFrom(ulid_msg(grant_id))
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="ListPending",
            request=req,
            response_cls=pb.ListPendingResponse,
        )
        return [pending_to_dict(p) for p in resp.pending]

    async def approve_pending(
        self,
        *,
        pending_ulid: str,
        also_trust_event_type: bool = False,
    ) -> dict[str, Any]:
        req = pb.ApprovePendingRequest(
            pending_ulid=ulid_msg(pending_ulid),
            also_auto_approve_this_type=also_trust_event_type,
        )
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="ApprovePending",
            request=req,
            response_cls=pb.ApprovePendingResponse,
        )
        return {
            "event_ulid": ulid_bytes_to_crockford(resp.event_ulid.bytes),
            "committed_at_ms": resp.committed_at_ms,
        }

    async def reject_pending(
        self, *, pending_ulid: str, reason: str | None = None
    ) -> dict[str, Any]:
        req = pb.RejectPendingRequest(pending_ulid=ulid_msg(pending_ulid))
        if reason:
            req.reason = reason
        resp = await self._transport.call_unary(
            service=_SERVICE,
            method="RejectPending",
            request=req,
            response_cls=pb.RejectPendingResponse,
        )
        return {"rejected_at_ms": resp.rejected_at_ms}

    # --- Cases (storage RPCs are stubbed in v1) -------------------------

    async def list_cases(self, *, include_closed: bool = True) -> list[dict[str, Any]]:
        raise OhdcNotWiredError(
            "list_cases", reason="storage ListCases handler is not wired (storage v1.x)"
        )

    async def get_case(self, *, case_ulid: str) -> dict[str, Any]:
        raise OhdcNotWiredError(
            "get_case", reason="storage GetCase handler is not wired (storage v1.x)"
        )

    async def force_close_case(self, *, case_ulid: str) -> dict[str, Any]:
        raise OhdcNotWiredError(
            "force_close_case",
            reason="storage CloseCase handler is not wired (storage v1.x)",
        )

    async def issue_retrospective_grant(
        self, *, case_ulid: str, label: str, **overrides: Any
    ) -> dict[str, Any]:
        raise OhdcNotWiredError(
            "issue_retrospective_grant",
            reason="case-scoped CreateGrant + case binding is not wired (storage v1.x)",
        )

    # --- Audit ----------------------------------------------------------

    async def audit_query(
        self,
        *,
        grant_id: str | None = None,
        from_time_ms: int | None = None,
        to_time_ms: int | None = None,
        limit: int | None = None,
    ) -> list[dict[str, Any]]:
        # Storage AuditQuery is a server-streaming Unimplemented in v1.
        raise OhdcNotWiredError(
            "audit_query",
            reason="storage AuditQuery handler is not wired (storage v1.x)",
        )


__all__ = [
    "OhdcClient",
    "OhdcClientConfig",
    "OhdcNotWiredError",
    "OhdcRpcError",
]
