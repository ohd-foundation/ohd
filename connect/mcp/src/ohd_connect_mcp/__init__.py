"""OHD Connect MCP server.

A FastMCP server that exposes the personal-side OHDC surface (logging,
reading, grants, pending review, cases, audit) as MCP tools, intended for
use by an LLM (Claude Desktop, Claude.ai, Cursor, Continue, …) authenticated
under the user's self-session.

Per the Pinned implementation decisions in the repo root README, the MCP
servers are Python + FastMCP. The OHDC client is currently a stub — see
``STATUS.md`` for the wire-up integration point.
"""

from __future__ import annotations

__version__ = "0.1.0"
__all__ = ["__version__"]
