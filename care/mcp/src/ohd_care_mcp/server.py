"""FastMCP server bootstrap for OHD Care MCP.

Run with::

    uv run python -m ohd_care_mcp           # stdio transport
    OHD_MCP_TRANSPORT=http uv run python -m ohd_care_mcp

Or via the installed entry point::

    uv run ohd-care-mcp

When the Streamable-HTTP transport is selected and the operator
configures `OHD_CARE_OIDC_ISSUER` / `OHD_CARE_OIDC_CLIENT_ID` /
`OHD_CARE_MCP_BASE_URL`, the server fronts itself with FastMCP's
:class:`OAuthProxy` against the clinic's OIDC issuer (Hospital ADFS /
Entra, Google Workspace, Authentik, Keycloak, …). Per
``../spec/care-auth.md`` "Operator authentication into Care".
"""

from __future__ import annotations

from fastmcp import FastMCP

from ohd_shared.oauth_proxy import build_oauth_proxy

from .config import CareMcpConfig
from .grant_vault import GrantVault
from .ohdc_client import OhdcClient, OhdcClientConfig
from .tools import register_tools


def build_server(
    config: CareMcpConfig | None = None,
    client: OhdcClient | None = None,
    vault: GrantVault | None = None,
) -> FastMCP:
    """Construct the FastMCP server with all tools registered."""
    cfg = config or CareMcpConfig.from_env()
    ohdc = client or OhdcClient(OhdcClientConfig(storage_url=cfg.storage_url))
    grant_vault = vault or GrantVault.from_list(cfg.grants)

    auth = None
    if cfg.oidc.enabled and cfg.transport == "http":
        try:
            auth = build_oauth_proxy(cfg.oidc)
        except Exception as exc:  # don't crash dev usage if OIDC config is half-set
            import logging

            logging.getLogger(__name__).warning(
                "OAuth proxy disabled — failed to discover issuer %s: %s",
                cfg.oidc.issuer_url,
                exc,
            )
            auth = None

    mcp: FastMCP = FastMCP(
        name="OHD Care",
        instructions=(
            "OHD Care is the operator-side OHDC consumer for clinicians. "
            "Tools here read and write a patient's record over a per-patient "
            "grant. Per care/SPEC.md §10.6: switch_patient(label) is the "
            "ONLY tool that changes active context; every other tool scopes "
            "to whatever patient is active. Write tools require confirm=True "
            "and route through approval per the grant's policy. Surface the "
            "active_patient field from each result back to the operator for "
            "orientation."
        ),
        auth=auth,
    )
    register_tools(mcp, ohdc, grant_vault)
    return mcp


def main() -> None:
    cfg = CareMcpConfig.from_env()
    mcp = build_server(cfg)
    if cfg.transport == "http":
        mcp.run(transport="http", host=cfg.http_host, port=cfg.http_port)
    else:
        mcp.run()


if __name__ == "__main__":
    main()
