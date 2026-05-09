"""Connect MCP tool registrations.

One function per MCP tool. Each tool:

1. Accepts pydantic-validated input via Annotated parameters.
2. Has a real, intent-shaped docstring (the LLM's tool-selection cue).
3. Builds the OHDC request shape the underlying RPC will expect.
4. Calls the (currently stubbed) OHDC client. The client raises
   ``OhdcNotWiredError`` until the wire-up agent fills it in.

The tool surface tracks ``connect/SPEC.md`` "Connect MCP — tool list" and
``spec/docs/research/mcp-servers.md`` "Connect MCP tool definitions".
"""

from __future__ import annotations

import time
from typing import Annotated, Any

from fastmcp import FastMCP
from pydantic import Field

from .ohdc_client import OhdcClient


def _now_ms() -> int:
    return int(time.time() * 1000)


def _resolve_ts(ts: str | None) -> int:
    """Resolve an optional timestamp to Unix-ms.

    Accepts ISO 8601 strings; falls back to "now" when None. Natural-language
    parsing ("yesterday", "30 minutes ago") is on the v0.x roadmap per
    ``spec/docs/research/mcp-servers.md`` "Handling time input"; for now we
    delegate to ``datetime.fromisoformat`` and let the caller format.
    """
    if ts is None:
        return _now_ms()
    from datetime import datetime, timezone

    parsed = datetime.fromisoformat(ts)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return int(parsed.timestamp() * 1000)


def register_tools(mcp: FastMCP, client: OhdcClient) -> None:
    """Register every Connect MCP tool against ``mcp`` using ``client``.

    Kept as a function (not module-level) so tests can construct an
    isolated ``FastMCP`` instance with a fake/real client.
    """

    # ------------------------------------------------------------------
    # Logging tools
    # ------------------------------------------------------------------

    @mcp.tool
    async def log_symptom(
        symptom: Annotated[str, Field(description="Symptom name, e.g. 'headache', 'nausea'.")],
        severity: Annotated[
            str | None,
            Field(description="Qualitative severity: 'mild', 'moderate', 'severe'."),
        ] = None,
        severity_scale: Annotated[
            str | None,
            Field(description="Numeric severity with scale, e.g. '1-10:7'."),
        ] = None,
        location: Annotated[
            str | None,
            Field(description="Anatomical location, e.g. 'frontal', 'left knee'."),
        ] = None,
        notes: Annotated[str | None, Field(description="Free-text notes.")] = None,
        timestamp: Annotated[
            str | None,
            Field(description="ISO 8601 timestamp; defaults to now."),
        ] = None,
    ) -> dict[str, Any]:
        """Log a symptom the user is currently experiencing."""
        event = {
            "event_type": "symptom",
            "timestamp_ms": _resolve_ts(timestamp),
            "data": {
                "symptom": symptom,
                "severity": severity,
                "severity_scale": severity_scale,
                "location": location,
                "notes": notes,
            },
            "metadata": {"source": "connect_mcp"},
        }
        return await client.put_events([event])

    @mcp.tool
    async def log_food(
        description: Annotated[str, Field(description="What was eaten or drunk.")],
        quantity: Annotated[
            str | None, Field(description="Amount, e.g. '120g', '1 cup', '2 slices'.")
        ] = None,
        started: Annotated[str | None, Field(description="When eating started (ISO 8601).")] = None,
        ended: Annotated[str | None, Field(description="When eating ended (ISO 8601).")] = None,
        barcode: Annotated[
            str | None,
            Field(description="EAN-13 / UPC barcode; enables OpenFoodFacts lookup server-side."),
        ] = None,
        notes: Annotated[str | None, Field(description="Free-text notes.")] = None,
    ) -> dict[str, Any]:
        """Log a food or drink the user consumed."""
        event = {
            "event_type": "meal",
            "timestamp_ms": _resolve_ts(started),
            "data": {
                "description": description,
                "quantity": quantity,
                "started": started,
                "ended": ended,
                "barcode": barcode,
                "notes": notes,
            },
            "metadata": {"source": "connect_mcp"},
        }
        return await client.put_events([event])

    @mcp.tool
    async def log_medication(
        name: Annotated[str, Field(description="Medication name, e.g. 'metformin'.")],
        dose: Annotated[
            str | None, Field(description="Dose, e.g. '500mg', '2 tablets'.")
        ] = None,
        status: Annotated[
            str,
            Field(description="One of 'taken', 'skipped', 'late', 'refused'."),
        ] = "taken",
        timestamp: Annotated[
            str | None, Field(description="ISO 8601; defaults to now.")
        ] = None,
        notes: Annotated[
            str | None, Field(description="Notes, e.g. 'with food'.")
        ] = None,
    ) -> dict[str, Any]:
        """Log a medication dose (taken, skipped, late, or refused)."""
        event = {
            "event_type": "medication_administered",
            "timestamp_ms": _resolve_ts(timestamp),
            "data": {
                "name": name,
                "dose": dose,
                "status": status,
                "notes": notes,
            },
            "metadata": {"source": "connect_mcp"},
        }
        return await client.put_events([event])

    @mcp.tool
    async def log_measurement(
        measurement_type: Annotated[
            str,
            Field(
                description=(
                    "Standard channel id like 'body_temperature', 'glucose', or a "
                    "namespaced custom id like 'urine.glucose'."
                )
            ),
        ],
        value: Annotated[float, Field(description="Numeric value.")],
        unit: Annotated[str, Field(description="Unit, e.g. 'C', 'mmol/L', 'mmHg'.")],
        timestamp: Annotated[
            str | None, Field(description="ISO 8601; defaults to now.")
        ] = None,
        notes: Annotated[str | None, Field(description="Free-text notes.")] = None,
    ) -> dict[str, Any]:
        """Log a generic health measurement not covered by a more specific tool."""
        event = {
            "event_type": measurement_type,
            "timestamp_ms": _resolve_ts(timestamp),
            "data": {"value": value, "unit": unit, "notes": notes},
            "metadata": {"source": "connect_mcp"},
        }
        return await client.put_events([event])

    @mcp.tool
    async def log_exercise(
        activity: Annotated[
            str, Field(description="Activity, e.g. 'running', 'cycling', 'yoga'.")
        ],
        duration_minutes: Annotated[
            int | None, Field(description="Duration in minutes.", ge=0)
        ] = None,
        intensity: Annotated[
            str | None, Field(description="'low', 'moderate', 'high'.")
        ] = None,
        started: Annotated[
            str | None, Field(description="When the session started (ISO 8601).")
        ] = None,
        notes: Annotated[str | None, Field(description="Free-text notes.")] = None,
    ) -> dict[str, Any]:
        """Log an exercise session."""
        event = {
            "event_type": "exercise",
            "timestamp_ms": _resolve_ts(started),
            "data": {
                "activity": activity,
                "duration_minutes": duration_minutes,
                "intensity": intensity,
                "notes": notes,
            },
            "metadata": {"source": "connect_mcp"},
        }
        return await client.put_events([event])

    @mcp.tool
    async def log_mood(
        mood: Annotated[
            str, Field(description="Mood description, e.g. 'anxious', 'calm', 'irritable'.")
        ],
        energy: Annotated[
            str | None, Field(description="Energy level: 'low', 'moderate', 'high'.")
        ] = None,
        notes: Annotated[str | None, Field(description="Free-text notes.")] = None,
        timestamp: Annotated[
            str | None, Field(description="ISO 8601; defaults to now.")
        ] = None,
    ) -> dict[str, Any]:
        """Log the user's current mood or emotional state."""
        event = {
            "event_type": "mood",
            "timestamp_ms": _resolve_ts(timestamp),
            "data": {"mood": mood, "energy": energy, "notes": notes},
            "metadata": {"source": "connect_mcp"},
        }
        return await client.put_events([event])

    @mcp.tool
    async def log_sleep(
        bedtime: Annotated[str, Field(description="When the user went to bed (ISO 8601).")],
        wake_time: Annotated[str, Field(description="When the user woke up (ISO 8601).")],
        quality: Annotated[
            str | None,
            Field(description="Subjective quality: 'poor', 'fair', 'good', 'great'."),
        ] = None,
        notes: Annotated[str | None, Field(description="Free-text notes.")] = None,
    ) -> dict[str, Any]:
        """Log a sleep session."""
        event = {
            "event_type": "sleep",
            "timestamp_ms": _resolve_ts(bedtime),
            "data": {
                "bedtime": bedtime,
                "wake_time": wake_time,
                "quality": quality,
                "notes": notes,
            },
            "metadata": {"source": "connect_mcp"},
        }
        return await client.put_events([event])

    @mcp.tool
    async def log_free_event(
        event_type: Annotated[
            str,
            Field(
                description=(
                    "Event type id. Use namespaced ids for custom events, e.g. "
                    "'com.user.dialysis'."
                )
            ),
        ],
        data: Annotated[
            dict[str, Any], Field(description="Event-type-specific payload.")
        ],
        timestamp: Annotated[
            str | None, Field(description="ISO 8601; defaults to now.")
        ] = None,
        duration_seconds: Annotated[
            int | None, Field(description="Duration in seconds, if applicable.", ge=0)
        ] = None,
    ) -> dict[str, Any]:
        """Fallback for event types not covered by other tools."""
        event = {
            "event_type": event_type,
            "timestamp_ms": _resolve_ts(timestamp),
            "data": data,
            "duration_seconds": duration_seconds,
            "metadata": {"source": "connect_mcp"},
        }
        return await client.put_events([event])

    # ------------------------------------------------------------------
    # Reading tools
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
        order: Annotated[
            str, Field(description="'asc' or 'desc'.")
        ] = "desc",
    ) -> list[dict[str, Any]]:
        """Iterate the user's events with optional filters."""
        return await client.query_events(
            event_type=event_type,
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
            limit=limit,
            order=order,
        )

    @mcp.tool
    async def query_latest(
        event_type: Annotated[str, Field(description="Event type to fetch.")],
        count: Annotated[
            int, Field(description="How many recent events.", ge=1, le=100)
        ] = 1,
    ) -> list[dict[str, Any]]:
        """Fetch the most recent N events of a given type."""
        return await client.query_events(event_type=event_type, limit=count, order="desc")

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
    ) -> list[dict[str, Any]]:
        """Aggregate events over time buckets."""
        return await client.aggregate(
            event_type=event_type,
            period=period,
            aggregation=aggregation,
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
        )

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
        """Find temporal relationships between two event types.

        For each event of type A, finds events of type B within
        ``window_minutes`` afterwards. Returns per-pair data plus summary
        statistics. Example:
        ``correlate(event_type_a='meal', event_type_b='glucose', window_minutes=180)``
        gives a post-meal glucose response view.
        """
        return await client.correlate(
            event_type_a=event_type_a,
            event_type_b=event_type_b,
            window_minutes=window_minutes,
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
        )

    @mcp.tool
    async def find_patterns(
        event_type: Annotated[str, Field(description="Event type to analyse.")],
        description: Annotated[
            str,
            Field(
                description=(
                    "Natural-language pattern description, e.g. 'unusually high readings' "
                    "or 'long gaps without logging'."
                )
            ),
        ],
        from_time: Annotated[str | None, Field(description="ISO 8601.")] = None,
        to_time: Annotated[str | None, Field(description="ISO 8601.")] = None,
    ) -> list[dict[str, Any]]:
        """Find events matching a natural-language pattern description."""
        # The real implementation uses statistical post-processing on top of
        # query_events; here we surface the OHDC stub.
        return await client.query_events(
            event_type=event_type,
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
        )

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
    ) -> list[dict[str, Any]]:
        """Get medication adherence data (taken / skipped / late)."""
        return await client.query_events(
            event_type="medication_administered",
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
        )

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
    ) -> list[dict[str, Any]]:
        """Get the user's food log with optional nutrition aggregates."""
        return await client.query_events(
            event_type="meal",
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
        )

    @mcp.tool
    async def chart(
        description: Annotated[
            str,
            Field(description="Natural-language chart description, e.g. 'glucose last week'."),
        ],
    ) -> dict[str, Any]:
        """Generate a chart from a natural-language description.

        Returns ``{image_base64, chart_spec, underlying_data}``. The chart
        spec can be saved as a reusable template.
        """
        # In a wired-up build this composes Aggregate + a chart renderer.
        return await client.aggregate(
            event_type="__chart_placeholder__",
            period="daily",
            aggregation="avg",
        )

    # ------------------------------------------------------------------
    # Grants
    # ------------------------------------------------------------------

    @mcp.tool
    async def create_grant(
        template_id: Annotated[
            str,
            Field(
                description=(
                    "Grant template id, e.g. 'primary_doctor', 'specialist_visit', "
                    "'spouse_family', 'researcher_with_study', 'emergency_template'."
                )
            ),
        ],
        label: Annotated[
            str, Field(description="Operator/grantee label, e.g. 'Dr. Smith'.")
        ],
        notes: Annotated[
            str | None, Field(description="Optional internal notes about this grant.")
        ] = None,
    ) -> dict[str, Any]:
        """Issue a grant from a template. Returns the share artifact (token + rendezvous URL)."""
        return await client.create_grant(template_id=template_id, label=label, notes=notes)

    @mcp.tool
    async def list_grants(
        include_revoked: Annotated[
            bool, Field(description="Include revoked grants in the list.")
        ] = False,
    ) -> list[dict[str, Any]]:
        """List active (and optionally revoked) grants."""
        return await client.list_grants(include_revoked=include_revoked)

    @mcp.tool
    async def revoke_grant(
        grant_id: Annotated[str, Field(description="Grant id to revoke.")],
    ) -> dict[str, Any]:
        """Synchronously revoke a grant. Not sync-deferred."""
        return await client.revoke_grant(grant_id=grant_id)

    # ------------------------------------------------------------------
    # Pending review
    # ------------------------------------------------------------------

    @mcp.tool
    async def list_pending(
        grant_id: Annotated[
            str | None,
            Field(description="Limit to one grant; None = all grants."),
        ] = None,
    ) -> list[dict[str, Any]]:
        """List pending writes awaiting user review."""
        return await client.list_pending(grant_id=grant_id)

    @mcp.tool
    async def approve_pending(
        pending_ulid: Annotated[
            str, Field(description="ULID of the pending event to approve.")
        ],
        also_trust_event_type: Annotated[
            bool,
            Field(
                description=(
                    "If true, add the event type to the source grant's "
                    "auto_for_event_types allowlist so future writes auto-promote."
                )
            ),
        ] = False,
    ) -> dict[str, Any]:
        """Approve a pending event; storage promotes it to ``events`` with the same ULID."""
        return await client.approve_pending(
            pending_ulid=pending_ulid,
            also_trust_event_type=also_trust_event_type,
        )

    @mcp.tool
    async def reject_pending(
        pending_ulid: Annotated[
            str, Field(description="ULID of the pending event to reject.")
        ],
        reason: Annotated[
            str | None, Field(description="Optional rejection reason (free text).")
        ] = None,
    ) -> dict[str, Any]:
        """Reject a pending event. Pending row stays with status='rejected'."""
        return await client.reject_pending(pending_ulid=pending_ulid, reason=reason)

    # ------------------------------------------------------------------
    # Cases
    # ------------------------------------------------------------------

    @mcp.tool
    async def list_cases(
        include_closed: Annotated[
            bool, Field(description="Include closed cases.")
        ] = True,
    ) -> list[dict[str, Any]]:
        """List active and (optionally) closed cases. Active first, then recent-closed."""
        return await client.list_cases(include_closed=include_closed)

    @mcp.tool
    async def get_case(
        case_ulid: Annotated[str, Field(description="Case ULID.")],
    ) -> dict[str, Any]:
        """Case detail: timeline, authorities, audit, handoff chain."""
        return await client.get_case(case_ulid=case_ulid)

    @mcp.tool
    async def force_close_case(
        case_ulid: Annotated[str, Field(description="Case ULID.")],
    ) -> dict[str, Any]:
        """User-initiated case close. Revokes the active authority's grant."""
        return await client.force_close_case(case_ulid=case_ulid)

    @mcp.tool
    async def issue_retrospective_grant(
        case_ulid: Annotated[str, Field(description="Case ULID this grant is scoped to.")],
        label: Annotated[
            str, Field(description="Operator/grantee label, e.g. 'Specialist consult'.")
        ],
        notes: Annotated[
            str | None, Field(description="Optional internal notes.")
        ] = None,
    ) -> dict[str, Any]:
        """Issue a case-scoped grant after the fact (specialist consult, billing review)."""
        return await client.issue_retrospective_grant(
            case_ulid=case_ulid, label=label, notes=notes
        )

    # ------------------------------------------------------------------
    # Audit
    # ------------------------------------------------------------------

    @mcp.tool
    async def audit_query(
        grant_id: Annotated[
            str | None,
            Field(description="Limit to one grant; None = global audit view."),
        ] = None,
        from_time: Annotated[
            str | None, Field(description="ISO 8601 start.")
        ] = None,
        to_time: Annotated[str | None, Field(description="ISO 8601 end.")] = None,
        limit: Annotated[
            int, Field(description="Max rows.", ge=1, le=10_000)
        ] = 500,
    ) -> list[dict[str, Any]]:
        """Per-grant or global audit view.

        Each row carries the ``auto_granted`` flag for emergency-timeout
        entries so the UI / LLM can surface them distinctly.
        """
        return await client.audit_query(
            grant_id=grant_id,
            from_time_ms=_resolve_ts(from_time) if from_time else None,
            to_time_ms=_resolve_ts(to_time) if to_time else None,
            limit=limit,
        )
