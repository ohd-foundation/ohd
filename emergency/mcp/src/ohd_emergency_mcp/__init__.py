"""OHD Emergency MCP server.

A narrowly-scoped FastMCP server that exposes a triage-assistant OHDC
surface (find_relevant_context, summarize_vitals, flag_abnormal_vitals,
check_administered_drug, draft_handoff_summary) for emergency-response
LLMs.

Per ``emergency/SPEC.md`` §3.2: this MCP does NOT expose generic
``query_events`` / ``put_events``. The LLM is a triage assistant, not an
exploratory analytics tool. Per §3.3, external LLMs are default-deny
(``OHD_EMERGENCY_MCP_ALLOW_EXTERNAL_LLM=false``).

Per the Pinned implementation decisions in the repo root README, the MCP
servers are Python + FastMCP. The OHDC client is currently a stub — see
``STATUS.md``.
"""

from __future__ import annotations

__version__ = "0.1.0"
__all__ = ["__version__"]
