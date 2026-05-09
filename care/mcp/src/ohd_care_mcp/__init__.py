"""OHD Care MCP server.

A FastMCP server that exposes the multi-patient clinical OHDC surface to
an LLM (operator-side: clinicians, specialists, researchers).

The operator authenticates via OIDC (a session token in env or via FastMCP's
OAuth proxy). Per-patient operations are routed through the **active grant**
selected via ``switch_patient(label)`` — analogous to a chart selector in a
traditional EHR but with explicit OHDC grants instead of role-based DB
access.

Per the Pinned implementation decisions in the repo root README, the MCP
servers are Python + FastMCP. The OHDC client is currently a stub — see
``STATUS.md`` for the wire-up integration point.
"""

from __future__ import annotations

__version__ = "0.1.0"
__all__ = ["__version__"]
