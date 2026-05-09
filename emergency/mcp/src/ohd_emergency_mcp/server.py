"""FastMCP server bootstrap for OHD Emergency MCP.

Run with::

    uv run python -m ohd_emergency_mcp           # stdio transport
    OHD_MCP_TRANSPORT=http uv run python -m ohd_emergency_mcp

Or via the installed entry point::

    uv run ohd-emergency-mcp

When the Streamable-HTTP transport is selected and the operator
configures ``OHD_EMERGENCY_OIDC_ISSUER`` / ``OHD_EMERGENCY_OIDC_CLIENT_ID`` /
``OHD_EMERGENCY_MCP_BASE_URL``, the server fronts itself with FastMCP's
:class:`OAuthProxy` against the operator IdP. Mirrors ``connect/mcp`` and
``care/mcp``; per ``../spec/auth.md`` "MCP servers".
"""

from __future__ import annotations

from fastmcp import FastMCP

from ohd_shared.oauth_proxy import build_oauth_proxy

from .case_vault import CaseVault
from .config import EmergencyMcpConfig
from .ohdc_client import OhdcClient, OhdcClientConfig
from .tools import register_tools


def build_server(
    config: EmergencyMcpConfig | None = None,
    client: OhdcClient | None = None,
    vault: CaseVault | None = None,
) -> FastMCP:
    """Construct the FastMCP server with all tools registered."""
    cfg = config or EmergencyMcpConfig.from_env()
    ohdc = client or OhdcClient(OhdcClientConfig(storage_url=cfg.storage_url))
    case_vault = vault or CaseVault.from_list(cfg.cases)

    auth = None
    if cfg.oidc.enabled and cfg.transport == "http":
        try:
            auth = build_oauth_proxy(cfg.oidc)
        except Exception as exc:
            import logging

            logging.getLogger(__name__).warning(
                "OAuth proxy disabled — failed to discover issuer %s: %s",
                cfg.oidc.issuer_url,
                exc,
            )
            auth = None

    phi_note = (
        ""
        if cfg.allow_external_llm
        else (
            " IMPORTANT: this deployment runs with "
            "OHD_EMERGENCY_MCP_ALLOW_EXTERNAL_LLM=false (the default per "
            "emergency/SPEC.md §3.3); only self-hosted LLMs should be "
            "invoking these tools, and PHI must NOT be relayed to external "
            "providers."
        )
    )

    mcp: FastMCP = FastMCP(
        name="OHD Emergency",
        instructions=(
            "OHD Emergency is a triage-assistant OHDC consumer. The tool "
            "surface is intentionally narrow — five high-level tools plus "
            "case selection. Generic query_events/put_events are NOT exposed. "
            "Per emergency/SPEC.md §3.1, set_active_case(case_id) is the "
            "ONLY tool that changes active context; every other tool scopes "
            "to whatever case is active."
        )
        + phi_note,
        auth=auth,
    )
    register_tools(mcp, ohdc, case_vault)
    return mcp


def main() -> None:
    cfg = EmergencyMcpConfig.from_env()
    mcp = build_server(cfg)
    if cfg.transport == "http":
        mcp.run(transport="http", host=cfg.http_host, port=cfg.http_port)
    else:
        mcp.run()


if __name__ == "__main__":
    main()
