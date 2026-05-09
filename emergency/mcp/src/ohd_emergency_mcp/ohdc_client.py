"""Real Python OHDC client (Connect-RPC over HTTP/2) for Emergency MCP.

Hand-rolled on top of :class:`ohd_shared.connect_transport.OhdcTransport`
and the protobuf message types in ``ohd_shared._gen.ohdc.v0.ohdc_pb2``.
The Connect-RPC transport and the proto<->dict helpers live in the
``ohd-shared`` workspace package; this module is the Emergency-MCP-specific
client surface (case-bound; per-call grant tokens; narrow read+put surface).

Per ``emergency/SPEC.md`` §3.2, the Emergency MCP does NOT expose generic
``query_events`` / ``put_events`` to the LLM. But under the hood it still
needs OHDC primitives to implement the high-level triage tools. The narrow
real-RPC surface here is:

- ``who_am_i`` — token introspection (probe; not surfaced as a tool today).
- ``query_events`` — backs ``find_relevant_context_for_complaint``,
  ``summarize_vitals``, ``flag_abnormal_vitals``, ``draft_handoff_summary``.
- ``put_events`` — backs ``draft_handoff_summary`` writing the summary back
  to the case timeline.

``aggregate``, ``find_relevant_context``, ``check_drug_interaction`` remain
as ``OhdcNotWiredError`` because either the underlying storage handler is
``Unimplemented`` (Aggregate) or the implementation is operator-side
(drug-interaction dataset) / classifier-side (find_relevant_context).

All methods are case-bound — every call attaches a per-call grant token
from ``case_vault.get_active_case().grant_token``. The transport receives
the token as ``Authorization: Bearer {grant_token}``.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from ohd_shared.connect_transport import OhdcRpcError, OhdcTransport
from ohd_shared.ohdc_helpers import (
    build_filter,
    event_input_from_dict,
    event_to_dict,
    pb,
    put_result_to_dict,
    ulid_bytes_to_crockford,
)

# Re-export the underscore-prefixed names that lived in this module's
# original copy.
_ulid_bytes_to_crockford = ulid_bytes_to_crockford
_event_to_dict = event_to_dict
_event_input_from_dict = event_input_from_dict
_put_result_to_dict = put_result_to_dict
_build_filter = build_filter

_SERVICE = "ohdc.v0.OhdcService"


class OhdcNotWiredError(NotImplementedError):
    """Raised when the OHDC client method is not yet implemented."""

    def __init__(self, method: str, *, reason: str | None = None) -> None:
        super().__init__(
            f"OHDC client method {method!r} is not yet wired"
            + (f": {reason}" if reason else "")
            + ". See emergency/mcp/STATUS.md 'OHDC client — wire status'."
        )


@dataclass(frozen=True)
class OhdcClientConfig:
    storage_url: str


class OhdcClient:
    """Async OHDC client over Connect-RPC for Emergency MCP.

    The transport is shared across cases; each method takes a
    ``grant_token`` kwarg which is attached as ``Authorization: Bearer``
    on that call. The vault upstream (``case_vault.CaseVault``) supplies
    the token from the active case grant.
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
        """Return information about the supplied case-grant token."""
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

    # --- Read primitives (case-bound; never exposed directly to the LLM) ---

    async def query_events(
        self,
        *,
        grant_token: str,
        case_id: str | None = None,
        event_type: str | None = None,
        from_time_ms: int | None = None,
        to_time_ms: int | None = None,
        limit: int | None = None,
        order: str = "desc",
    ) -> list[dict[str, Any]]:
        """Scoped event query — case-bound; never exposed directly to LLM.

        ``case_id`` is accepted for API stability but is informational —
        case-scoping is enforced by the grant token's case_ulid binding on
        the storage side. Today this argument is unused; once storage
        supports a ``QueryEventsRequest.case_ulid`` filter it can route
        through.
        """
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
            token=grant_token,
        ):
            out.append(event_to_dict(ev))
        return out

    async def aggregate(
        self,
        *,
        grant_token: str,
        case_id: str | None = None,
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

    # --- Write primitives (case-bound; backs draft_handoff_summary) ---

    async def put_events(
        self,
        *,
        grant_token: str,
        events: list[dict[str, Any]],
    ) -> dict[str, Any]:
        """Submit case-bound events (e.g. handoff summary write-back).

        Used by ``draft_handoff_summary`` to persist the summary back to
        the case timeline. Storage may route through ``pending_events``
        per the case grant's policy.
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

    # --- Workflow helpers (storage-side classifiers not wired) ---

    async def find_relevant_context(
        self,
        *,
        grant_token: str,
        case_id: str | None,
        complaint: str,
    ) -> dict[str, Any]:
        """Triage-shaped context pull (recent vitals + meds + history slices).

        Real implementation composes ``query_events`` over the case grant
        plus a small server-side complaint classifier. The classifier side
        is not yet wired (per emergency/SPEC.md §3.2), so we surface
        OhdcNotWiredError until storage exposes it.
        """
        raise OhdcNotWiredError(
            "find_relevant_context",
            reason=(
                "storage-side complaint classifier is not wired; v0.x will "
                "compose this client-side from query_events instead."
            ),
        )

    async def check_drug_interaction(
        self,
        *,
        grant_token: str,
        case_id: str | None,
        drug_name: str,
        dose: str | None,
    ) -> dict[str, Any]:
        """Cross-reference candidate drug against patient meds + allergies.

        Real implementation: ``query_events`` over the patient's
        medication / allergy events plus an operator-provided
        drug-interaction dataset (per emergency/SPEC.md §3.2). The
        operator's dataset is a deployment-side artefact, not part of
        OHDC; v0.x will load it at server start.
        """
        raise OhdcNotWiredError(
            "check_drug_interaction",
            reason=(
                "operator-provided drug-interaction dataset loader is not "
                "yet implemented (per emergency/SPEC.md §3.2)."
            ),
        )


__all__ = [
    "OhdcClient",
    "OhdcClientConfig",
    "OhdcNotWiredError",
    "OhdcRpcError",
]
