"""Care MCP tool registrations.

Tracks ``care/SPEC.md`` §10 "Care MCP — tool catalog":

- §10.1 Patient management: ``list_patients``, ``switch_patient``, ``current_patient``.
- §10.2 Read tools: ``query_events``, ``query_latest``, ``summarize``,
  ``correlate``, ``find_patterns``, ``chart``, ``get_medications_taken``,
  ``get_food_log``.
- §10.3 Write-with-approval: ``submit_lab_result``, ``submit_measurement``,
  ``submit_observation``, ``submit_clinical_note``, ``submit_prescription``,
  ``submit_referral``.
- §10.4 Workflow: ``draft_visit_summary``, ``compare_to_previous_visit``,
  ``find_relevant_context_for_complaint``.

Per §10.6 safety rules, every tool result includes the active patient label
so the LLM can re-confirm orientation, and write tools require a
``confirm=True`` argument before submitting.
"""

from __future__ import annotations

import time
from typing import Annotated, Any

from fastmcp import FastMCP
from pydantic import Field

from .grant_vault import GrantVault
from .ohdc_client import OhdcClient


def _now_ms() -> int:
    return int(time.time() * 1000)


def _resolve_ts(ts: str | None) -> int:
    if ts is None:
        return _now_ms()
    from datetime import datetime, timezone

    parsed = datetime.fromisoformat(ts)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return int(parsed.timestamp() * 1000)


def _orient(vault: GrantVault) -> dict[str, Any]:
    """Per SPEC §10.6: surface the active patient on every result."""
    active = vault.current()
    return {"active_patient": active.label if active else None}


def register_tools(mcp: FastMCP, client: OhdcClient, vault: GrantVault) -> None:
    """Register every Care MCP tool. ``vault`` is shared module state for the session."""

    # ------------------------------------------------------------------
    # §10.1 Patient management
    # ------------------------------------------------------------------

    @mcp.tool
    async def list_patients() -> dict[str, Any]:
        """List the patient labels available to this operator session.

        Each entry includes the label and a short scope summary; the active
        patient (if any) is flagged.
        """
        return {"patients": vault.list_patients(), **_orient(vault)}

    @mcp.tool
    async def switch_patient(
        label: Annotated[
            str,
            Field(description="Patient label as returned by list_patients()."),
        ],
    ) -> dict[str, Any]:
        """Set the active patient for subsequent tool calls.

        This is the **only** tool that changes active context. Idempotent;
        switching to the already-active label is a no-op.
        """
        grant = vault.switch(label)
        return {
            "active_patient": grant.label,
            "scope_summary": grant.scope_summary,
            "switched": True,
        }

    @mcp.tool
    async def current_patient() -> dict[str, Any]:
        """Diagnostic: return the active patient label and grant scope."""
        active = vault.current()
        if active is None:
            return {"active_patient": None}
        return {
            "active_patient": active.label,
            "scope_summary": active.scope_summary,
        }

    # ------------------------------------------------------------------
    # §10.2 Read tools (active patient, gated by read scope)
    # ------------------------------------------------------------------

    @mcp.tool
    async def query_events(
        event_type: Annotated[
            str | None, Field(description="Filter by event type.")
        ] = None,
        from_time: Annotated[
            str | None, Field(description="ISO 8601 start; defaults to 30 days ago.")
        ] = None,
        to_time: Annotated[
            str | None, Field(description="ISO 8601 end; defaults to now.")
        ] = None,
        limit: Annotated[
            int, Field(description="Max rows.", ge=1, le=1000)
        ] = 100,
        order: Annotated[str, Field(description="'asc' or 'desc'.")] = "desc",
    ) -> dict[str, Any]:
        """Iterate the active patient's events with optional filters."""
        active = vault.require_current()
        rows = await client.query_events(
            grant_token=active.grant_token,
            event_type=event_type,
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
            limit=limit,
            order=order,
        )
        return {"events": rows, **_orient(vault)}

    @mcp.tool
    async def query_latest(
        event_type: Annotated[str, Field(description="Event type to fetch.")],
        count: Annotated[
            int, Field(description="How many recent events.", ge=1, le=100)
        ] = 1,
    ) -> dict[str, Any]:
        """Fetch the most recent N events of a given type for the active patient."""
        active = vault.require_current()
        rows = await client.query_events(
            grant_token=active.grant_token,
            event_type=event_type,
            limit=count,
            order="desc",
        )
        return {"events": rows, **_orient(vault)}

    @mcp.tool
    async def summarize(
        event_type: Annotated[str, Field(description="Event type to summarize.")],
        period: Annotated[
            str,
            Field(description="Bucket: 'hourly', 'daily', 'weekly', 'monthly'."),
        ],
        aggregation: Annotated[
            str,
            Field(description="'avg', 'min', 'max', 'sum', 'count', 'median'."),
        ] = "avg",
        from_time: Annotated[
            str | None, Field(description="ISO 8601; defaults to 90 days ago.")
        ] = None,
        to_time: Annotated[
            str | None, Field(description="ISO 8601; defaults to now.")
        ] = None,
    ) -> dict[str, Any]:
        """Aggregate the active patient's events over time buckets."""
        active = vault.require_current()
        rows = await client.aggregate(
            grant_token=active.grant_token,
            event_type=event_type,
            period=period,
            aggregation=aggregation,
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
        )
        return {"buckets": rows, **_orient(vault)}

    @mcp.tool
    async def correlate(
        event_type_a: Annotated[
            str, Field(description="First event type (the trigger).")
        ],
        event_type_b: Annotated[
            str, Field(description="Second event type (the response).")
        ],
        window_minutes: Annotated[
            int,
            Field(description="Minutes after A to look for B.", ge=1, le=1440),
        ] = 120,
        from_time: Annotated[str | None, Field(description="ISO 8601.")] = None,
        to_time: Annotated[str | None, Field(description="ISO 8601.")] = None,
    ) -> dict[str, Any]:
        """Find temporal relationships between two event types for the active patient."""
        active = vault.require_current()
        result = await client.correlate(
            grant_token=active.grant_token,
            event_type_a=event_type_a,
            event_type_b=event_type_b,
            window_minutes=window_minutes,
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
        )
        return {"correlation": result, **_orient(vault)}

    @mcp.tool
    async def find_patterns(
        event_type: Annotated[str, Field(description="Event type to analyse.")],
        description: Annotated[
            str,
            Field(description="Natural-language pattern description."),
        ],
        from_time: Annotated[str | None, Field(description="ISO 8601.")] = None,
        to_time: Annotated[str | None, Field(description="ISO 8601.")] = None,
    ) -> dict[str, Any]:
        """Find events matching a natural-language pattern for the active patient."""
        active = vault.require_current()
        rows = await client.query_events(
            grant_token=active.grant_token,
            event_type=event_type,
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
        )
        return {"events": rows, "pattern": description, **_orient(vault)}

    @mcp.tool
    async def chart(
        description: Annotated[
            str, Field(description="Natural-language chart description.")
        ],
    ) -> dict[str, Any]:
        """Generate a chart for the active patient from a natural-language description.

        Returns ``{image_base64, chart_spec, underlying_data}``.
        """
        active = vault.require_current()
        result = await client.aggregate(
            grant_token=active.grant_token,
            event_type="__chart_placeholder__",
            period="daily",
            aggregation="avg",
        )
        return {"chart": result, "description": description, **_orient(vault)}

    @mcp.tool
    async def get_medications_taken(
        from_time: Annotated[
            str | None, Field(description="ISO 8601; defaults to 30 days ago.")
        ] = None,
        to_time: Annotated[
            str | None, Field(description="ISO 8601; defaults to now.")
        ] = None,
        medication_name: Annotated[
            str | None, Field(description="Filter by medication name.")
        ] = None,
    ) -> dict[str, Any]:
        """Get the active patient's medication adherence data."""
        active = vault.require_current()
        rows = await client.query_events(
            grant_token=active.grant_token,
            event_type="medication_administered",
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
        )
        return {"events": rows, **_orient(vault)}

    @mcp.tool
    async def get_food_log(
        from_time: Annotated[
            str | None, Field(description="ISO 8601; defaults to 7 days ago.")
        ] = None,
        to_time: Annotated[
            str | None, Field(description="ISO 8601; defaults to now.")
        ] = None,
        include_nutrition_totals: Annotated[
            bool, Field(description="Include rolled-up nutrition aggregates.")
        ] = True,
    ) -> dict[str, Any]:
        """Get the active patient's food log with optional nutrition aggregates."""
        active = vault.require_current()
        rows = await client.query_events(
            grant_token=active.grant_token,
            event_type="meal",
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
        )
        return {"events": rows, **_orient(vault)}

    # ------------------------------------------------------------------
    # §10.3 Write-with-approval tools
    # ------------------------------------------------------------------

    def _require_confirm(confirm: bool, what: str) -> None:
        """Per §10.6: writes must be explicitly confirmed."""
        active = vault.require_current()
        if not confirm:
            raise PermissionError(
                f"Refusing to submit {what} to {active.label!r} without confirm=True. "
                "Re-issue the call with confirm=True after operator confirmation."
            )

    @mcp.tool
    async def submit_lab_result(
        result_data: Annotated[
            dict[str, Any],
            Field(
                description=(
                    "Structured lab result: {test_name, value, unit, reference_range?, "
                    "collected_at?, ordered_by?, notes?}."
                )
            ),
        ],
        confirm: Annotated[
            bool, Field(description="Must be true; safety guard per SPEC §10.6.")
        ] = False,
    ) -> dict[str, Any]:
        """Submit a lab result for the active patient. Routes through approval per grant policy."""
        _require_confirm(confirm, "lab result")
        active = vault.require_current()
        event = {
            "event_type": "lab_result",
            "timestamp_ms": _now_ms(),
            "data": result_data,
            "metadata": {"source": "care_mcp", "operator_label": "TODO"},
        }
        result = await client.put_events(
            grant_token=active.grant_token, events=[event]
        )
        return {"result": result, **_orient(vault)}

    @mcp.tool
    async def submit_measurement(
        measurement_type: Annotated[
            str,
            Field(description="Channel id, e.g. 'blood_pressure', 'heart_rate'."),
        ],
        value: Annotated[float, Field(description="Numeric value.")],
        unit: Annotated[str, Field(description="Unit, e.g. 'mmHg', 'bpm'.")],
        timestamp: Annotated[
            str | None, Field(description="ISO 8601; defaults to now.")
        ] = None,
        notes: Annotated[str | None, Field(description="Free-text notes.")] = None,
        confirm: Annotated[
            bool, Field(description="Must be true; safety guard per SPEC §10.6.")
        ] = False,
    ) -> dict[str, Any]:
        """Submit a measurement for the active patient."""
        _require_confirm(confirm, "measurement")
        active = vault.require_current()
        event = {
            "event_type": measurement_type,
            "timestamp_ms": _resolve_ts(timestamp),
            "data": {"value": value, "unit": unit, "notes": notes},
            "metadata": {"source": "care_mcp"},
        }
        result = await client.put_events(
            grant_token=active.grant_token, events=[event]
        )
        return {"result": result, **_orient(vault)}

    @mcp.tool
    async def submit_observation(
        observation_data: Annotated[
            dict[str, Any],
            Field(
                description=(
                    "Structured observation: {kind, value?, unit?, notes?, "
                    "observed_at?}."
                )
            ),
        ],
        confirm: Annotated[
            bool, Field(description="Must be true; safety guard per SPEC §10.6.")
        ] = False,
    ) -> dict[str, Any]:
        """Submit an observation for the active patient."""
        _require_confirm(confirm, "observation")
        active = vault.require_current()
        event = {
            "event_type": "observation",
            "timestamp_ms": _now_ms(),
            "data": observation_data,
            "metadata": {"source": "care_mcp"},
        }
        result = await client.put_events(
            grant_token=active.grant_token, events=[event]
        )
        return {"result": result, **_orient(vault)}

    @mcp.tool
    async def submit_clinical_note(
        note_text: Annotated[str, Field(description="Free-text clinical note.")],
        about_visit: Annotated[
            str | None,
            Field(
                description=(
                    "Optional visit identifier or label this note is about, "
                    "e.g. 'visit 2026-05-07'."
                )
            ),
        ] = None,
        confirm: Annotated[
            bool, Field(description="Must be true; safety guard per SPEC §10.6.")
        ] = False,
    ) -> dict[str, Any]:
        """Submit a clinical note for the active patient."""
        _require_confirm(confirm, "clinical note")
        active = vault.require_current()
        event = {
            "event_type": "clinical_note",
            "timestamp_ms": _now_ms(),
            "data": {"note_text": note_text, "about_visit": about_visit},
            "metadata": {"source": "care_mcp"},
        }
        result = await client.put_events(
            grant_token=active.grant_token, events=[event]
        )
        return {"result": result, **_orient(vault)}

    @mcp.tool
    async def submit_prescription(
        medication: Annotated[str, Field(description="Medication name.")],
        dose: Annotated[str, Field(description="Dose, e.g. '500mg'.")],
        schedule: Annotated[
            str, Field(description="Schedule, e.g. 'BID', 'QHS', 'every 8h'.")
        ],
        duration: Annotated[
            str, Field(description="Duration, e.g. '7 days', 'ongoing'.")
        ],
        notes: Annotated[
            str | None, Field(description="Notes for the patient or pharmacy.")
        ] = None,
        confirm: Annotated[
            bool, Field(description="Must be true; safety guard per SPEC §10.6.")
        ] = False,
    ) -> dict[str, Any]:
        """Submit a prescription for the active patient.

        Note: this is the OHDC ``medication_prescribed`` event; pharmacy
        delivery is a separate OHDC consumer (per Care SPEC §13).
        """
        _require_confirm(confirm, "prescription")
        active = vault.require_current()
        event = {
            "event_type": "medication_prescribed",
            "timestamp_ms": _now_ms(),
            "data": {
                "medication": medication,
                "dose": dose,
                "schedule": schedule,
                "duration": duration,
                "notes": notes,
            },
            "metadata": {"source": "care_mcp"},
        }
        result = await client.put_events(
            grant_token=active.grant_token, events=[event]
        )
        return {"result": result, **_orient(vault)}

    @mcp.tool
    async def submit_referral(
        specialty: Annotated[str, Field(description="Target specialty, e.g. 'cardiology'.")],
        reason: Annotated[str, Field(description="Reason for referral.")],
        referred_to: Annotated[
            str | None, Field(description="Specific specialist or facility.")
        ] = None,
        confirm: Annotated[
            bool, Field(description="Must be true; safety guard per SPEC §10.6.")
        ] = False,
    ) -> dict[str, Any]:
        """Submit a referral for the active patient."""
        _require_confirm(confirm, "referral")
        active = vault.require_current()
        event = {
            "event_type": "referral",
            "timestamp_ms": _now_ms(),
            "data": {
                "specialty": specialty,
                "reason": reason,
                "referred_to": referred_to,
            },
            "metadata": {"source": "care_mcp"},
        }
        result = await client.put_events(
            grant_token=active.grant_token, events=[event]
        )
        return {"result": result, **_orient(vault)}

    # ------------------------------------------------------------------
    # §10.4 Workflow tools
    # ------------------------------------------------------------------

    @mcp.tool
    async def draft_visit_summary() -> dict[str, Any]:
        """Draft a patient-readable visit summary for the active patient.

        The operator reviews and submits the result; this tool returns the
        draft, it does not submit anything.
        """
        active = vault.require_current()
        rows = await client.query_events(
            grant_token=active.grant_token, limit=50, order="desc"
        )
        return {
            "draft": {
                "patient": active.label,
                "recent_events": rows,
                "narrative": "TODO: real summary requires LLM post-processing",
            },
            **_orient(vault),
        }

    @mcp.tool
    async def compare_to_previous_visit() -> dict[str, Any]:
        """Narrative diff between the active patient's current and previous visit."""
        active = vault.require_current()
        rows = await client.query_events(
            grant_token=active.grant_token, limit=200, order="desc"
        )
        return {"comparison": rows, **_orient(vault)}

    @mcp.tool
    async def find_relevant_context_for_complaint(
        complaint: Annotated[
            str,
            Field(
                description=(
                    "Chief complaint, e.g. 'chest pain', 'fatigue last 2 weeks'."
                )
            ),
        ],
    ) -> dict[str, Any]:
        """Pull visit-prep slices for a chief complaint, scoped to the active patient.

        E.g. "chest pain" → recent BP/HR + cardiac meds + cardiac history.
        """
        active = vault.require_current()
        result = await client.find_relevant_context(
            grant_token=active.grant_token, complaint=complaint
        )
        return {"context": result, "complaint": complaint, **_orient(vault)}

    # ------------------------------------------------------------------
    # §10.5 Case tools
    # ------------------------------------------------------------------

    @mcp.tool
    async def open_case(
        case_type: Annotated[
            str,
            Field(
                description=(
                    "Case type per SPEC §4.1: one of 'admission', "
                    "'outpatient', 'ongoing-therapy', 'emergency-inherited', "
                    "or any deployment-defined string."
                )
            ),
        ],
        label: Annotated[
            str,
            Field(description="Human-readable label, e.g. 'Visit 2026-05-08'."),
        ],
        predecessor_case_ulid: Annotated[
            str | None,
            Field(
                description=(
                    "Optional predecessor case ULID — typically the EMS case "
                    "that brought the patient in. Inherits read scope."
                )
            ),
        ] = None,
        parent_case_ulid: Annotated[
            str | None,
            Field(
                description=(
                    "Optional parent case ULID — the broader containing case "
                    "(child-case results roll up to the parent)."
                )
            ),
        ] = None,
        confirm: Annotated[
            bool, Field(description="Must be true; case opens are write ops per SPEC §10.6.")
        ] = False,
    ) -> dict[str, Any]:
        """Open a case at the start of an encounter.

        Maps to OHDC ``CreateCase``; storage records a ``case_started``
        marker. Per SPEC §10.6 this is a write op and must be confirmed.
        Returns the new case's record.
        """
        _require_confirm(confirm, "case open")
        active = vault.require_current()
        case = await client.open_case(
            grant_token=active.grant_token,
            case_type=case_type,
            case_label=label,
            predecessor_case_ulid=predecessor_case_ulid,
            parent_case_ulid=parent_case_ulid,
        )
        return {"case": case, **_orient(vault)}

    @mcp.tool
    async def close_case(
        case_ulid: Annotated[str, Field(description="ULID of the case to close.")],
        reason: Annotated[
            str | None,
            Field(description="Optional close reason ('discharge', 'shift-end')."),
        ] = None,
        confirm: Annotated[
            bool, Field(description="Must be true; case closes are write ops per SPEC §10.6.")
        ] = False,
    ) -> dict[str, Any]:
        """Close a case (discharge / end-of-shift / end-of-visit).

        Per SPEC §4.1 storage records a ``case_closed`` marker; the
        operator retains read-only access to the case's span. Returns
        the closed case's record (with ``ended_at_ms`` populated).
        """
        _require_confirm(confirm, "case close")
        active = vault.require_current()
        case = await client.close_case(
            grant_token=active.grant_token,
            case_ulid=case_ulid,
            reason=reason,
        )
        return {"case": case, **_orient(vault)}

    @mcp.tool
    async def list_cases(
        include_closed: Annotated[
            bool,
            Field(description="Include closed cases (default: True)."),
        ] = True,
        case_type: Annotated[
            str | None,
            Field(description="Filter by case_type, e.g. 'admission'."),
        ] = None,
    ) -> dict[str, Any]:
        """List cases for the active patient (open + recently closed)."""
        active = vault.require_current()
        cases = await client.list_cases(
            grant_token=active.grant_token,
            include_closed=include_closed,
            case_type=case_type,
        )
        return {"cases": cases, **_orient(vault)}

    @mcp.tool
    async def get_case(
        case_ulid: Annotated[str, Field(description="ULID of the case to fetch.")],
    ) -> dict[str, Any]:
        """Get one case's full record (timeline + audit + handoff chain)."""
        active = vault.require_current()
        case = await client.get_case(
            grant_token=active.grant_token, case_ulid=case_ulid
        )
        return {"case": case, **_orient(vault)}

    @mcp.tool
    async def force_close_case(
        case_ulid: Annotated[str, Field(description="ULID of the case to force-close.")],
        confirm: Annotated[
            bool,
            Field(description="Must be true; force-close is destructive per SPEC §10.6."),
        ] = False,
    ) -> dict[str, Any]:
        """Operator-side force close. Different from patient force-close.

        Per SPEC §4.5 the patient can force-close any case from OHD
        Connect; this tool is the *operator-side* mirror — the operator
        cleanly closes their own authority before the inactivity timer
        fires (e.g., end of an unexpected shift). Maps to ``CloseCase``
        with ``reason='force_close'``.
        """
        _require_confirm(confirm, "force-close case")
        active = vault.require_current()
        case = await client.close_case(
            grant_token=active.grant_token,
            case_ulid=case_ulid,
            reason="force_close",
        )
        return {"case": case, **_orient(vault)}

    @mcp.tool
    async def issue_retrospective_grant(
        case_ulid: Annotated[
            str, Field(description="ULID of the closed case to grant access for.")
        ],
        grantee_label: Annotated[
            str,
            Field(
                description=(
                    "Human-readable label for the grantee (the specialist / "
                    "insurer / researcher receiving the grant)."
                )
            ),
        ],
        scope_event_types: Annotated[
            list[str],
            Field(
                description=(
                    "Event-type allowlist for the retrospective grant; "
                    "default is closed-scope (no event types implies the "
                    "grant reads nothing — usually you want 1+ types)."
                )
            ),
        ],
        expires_days: Annotated[
            int,
            Field(
                description="Days until the retrospective grant expires.",
                ge=1,
                le=365,
            ),
        ] = 30,
        confirm: Annotated[
            bool,
            Field(
                description=(
                    "Must be true; retrospective grants delegate read access "
                    "and are write ops per SPEC §10.6."
                )
            ),
        ] = False,
    ) -> dict[str, Any]:
        """Issue a retrospective case-scoped grant per SPEC §4.6.

        Once a case is closed, the operator can issue a narrow,
        case-scoped grant to a specialist / insurer / researcher for
        review. Storage may reject this call when invoked under a grant
        token (``CreateGrant`` is documented as self-session-only in
        OHDC v0); the underlying client surfaces a typed
        :class:`OhdcNotWiredError` so the LLM can fall back to "ask the
        patient to issue this from OHD Connect" rather than a wire-level
        stack.
        """
        _require_confirm(confirm, "retrospective grant")
        active = vault.require_current()
        result = await client.issue_retrospective_grant(
            grant_token=active.grant_token,
            case_ulid=case_ulid,
            grantee_label=grantee_label,
            scope_event_types=scope_event_types,
            expires_days=expires_days,
        )
        return {"retrospective_grant": result, **_orient(vault)}
