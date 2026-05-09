"""Emergency MCP tool registrations.

Tracks ``emergency/SPEC.md`` §3.2 "Tools (intentionally narrow — emergencies
are time-critical)":

- ``set_active_case(case_id)`` — case context selector (analogous to Care
  MCP's ``switch_patient``). Per §3.1.
- ``find_relevant_context_for_complaint(complaint)``
- ``summarize_vitals(window)``
- ``flag_abnormal_vitals()``
- ``check_administered_drug(drug_name, dose)``
- ``draft_handoff_summary()``

This MCP does NOT expose generic ``query_events`` / ``put_events``. That's
deliberate scope — per §3.2, the LLM is a triage assistant, not an
exploratory analytics tool.
"""

from __future__ import annotations

from typing import Annotated, Any

from fastmcp import FastMCP
from pydantic import Field

from .case_vault import CaseVault
from .ohdc_client import OhdcClient


def _orient(vault: CaseVault) -> dict[str, Any]:
    active = vault.current()
    return {
        "active_case_id": active.case_id if active else None,
        "active_case_label": active.label if active else None,
    }


def register_tools(mcp: FastMCP, client: OhdcClient, vault: CaseVault) -> None:
    """Register every Emergency MCP tool. ``vault`` is shared session state."""

    # ------------------------------------------------------------------
    # Case selection (per §3.1)
    # ------------------------------------------------------------------

    @mcp.tool
    async def list_active_cases() -> dict[str, Any]:
        """List the case-bound grants this operator session has access to."""
        return {"cases": vault.list_cases(), **_orient(vault)}

    @mcp.tool
    async def set_active_case(
        case_id: Annotated[
            str,
            Field(description="Case id as listed by list_active_cases()."),
        ],
    ) -> dict[str, Any]:
        """Set the active case for subsequent tool calls.

        This is the **only** tool that changes active context. Analogous to
        Care MCP's switch_patient.
        """
        grant = vault.set_active(case_id)
        return {
            "active_case_id": grant.case_id,
            "active_case_label": grant.label,
            "set": True,
        }

    # ------------------------------------------------------------------
    # Triage tools (per §3.2)
    # ------------------------------------------------------------------

    @mcp.tool
    async def find_relevant_context_for_complaint(
        complaint: Annotated[
            str,
            Field(
                description=(
                    "Chief complaint, e.g. 'chest pain', 'possible OD', "
                    "'altered mental status'."
                )
            ),
        ],
    ) -> dict[str, Any]:
        """Pull the data slices that matter for the chief complaint.

        "Chest pain" → recent BP/HR + cardiac meds + cardiac history.
        "Possible OD" → drugs taken + history + allergies.

        Returns a structured triage context object scoped to the active
        case grant.
        """
        active = vault.require_current()
        result = await client.find_relevant_context(
            grant_token=active.grant_token,
            case_id=active.case_id,
            complaint=complaint,
        )
        return {"context": result, "complaint": complaint, **_orient(vault)}

    @mcp.tool
    async def summarize_vitals(
        window: Annotated[
            str,
            Field(
                description=(
                    "Time window to summarize, e.g. 'last_15m', 'last_1h', "
                    "'last_24h'."
                )
            ),
        ] = "last_1h",
    ) -> dict[str, Any]:
        """Aggregate vitals across a recent time window.

        Returns averages and trends for HR, BP, SpO2, temperature, RR, GCS
        if available, scoped to the active case.
        """
        active = vault.require_current()
        # Real implementation will dispatch one Aggregate per channel and
        # collate; for v0 we just route a placeholder.
        result = await client.aggregate(
            grant_token=active.grant_token,
            case_id=active.case_id,
            event_type="vitals",
            period="hourly",
            aggregation="avg",
        )
        return {"summary": result, "window": window, **_orient(vault)}

    @mcp.tool
    async def flag_abnormal_vitals() -> dict[str, Any]:
        """Flag readings outside normal ranges given baseline + known conditions.

        Backed by ``query_events`` for vitals plus a small server-side
        classifier (per emergency/SPEC.md §3.2). Returns flagged readings
        with ``why`` strings the LLM can paraphrase to the responder.
        """
        active = vault.require_current()
        rows = await client.query_events(
            grant_token=active.grant_token,
            case_id=active.case_id,
            event_type="vitals",
            limit=50,
        )
        return {"flagged": rows, **_orient(vault)}

    @mcp.tool
    async def check_administered_drug(
        drug_name: Annotated[
            str, Field(description="Candidate drug name, e.g. 'naloxone', 'epinephrine'.")
        ],
        dose: Annotated[
            str | None, Field(description="Candidate dose, e.g. '0.4mg IM'.")
        ] = None,
    ) -> dict[str, Any]:
        """Check the candidate drug against current meds and allergies.

        Returns interactions, contraindications, or "no flags". Backed by
        ``query_events`` over the patient's medication / allergy events plus
        an operator-provided drug-interaction lookup.
        """
        active = vault.require_current()
        result = await client.check_drug_interaction(
            grant_token=active.grant_token,
            case_id=active.case_id,
            drug_name=drug_name,
            dose=dose,
        )
        return {
            "drug": drug_name,
            "dose": dose,
            "result": result,
            **_orient(vault),
        }

    @mcp.tool
    async def draft_handoff_summary() -> dict[str, Any]:
        """Produce a structured handoff summary for the receiving ER.

        Aggregates the case timeline (events recorded by the crew + initial
        profile) into a structured summary the responder reads on transfer.
        """
        active = vault.require_current()
        events = await client.query_events(
            grant_token=active.grant_token,
            case_id=active.case_id,
            limit=200,
        )
        return {
            "draft": {
                "case_id": active.case_id,
                "case_label": active.label,
                "timeline": events,
                "narrative": (
                    "TODO: real summary requires LLM post-processing of "
                    "the timeline against a handoff template."
                ),
            },
            **_orient(vault),
        }
