"""FastMCP server bootstrap for OHD Connect MCP.

Run with::

    uv run python -m ohd_connect_mcp           # stdio transport
    OHD_MCP_TRANSPORT=http uv run python -m ohd_connect_mcp

Or via the installed entry point::

    uv run ohd-connect-mcp

When the Streamable-HTTP transport is selected and the operator
configures `OHD_CONNECT_OIDC_ISSUER` / `OHD_CONNECT_OIDC_CLIENT_ID` /
`OHD_CONNECT_MCP_BASE_URL`, the server fronts itself with FastMCP's
:class:`OAuthProxy` against the storage AS (or a third-party IdP).
Per ``../spec/auth.md`` "MCP servers".
"""

from __future__ import annotations

from fastmcp import FastMCP

from ohd_shared.oauth_proxy import build_oauth_proxy

from .config import ConnectMcpConfig
from .ohdc_client import OhdcClient, OhdcClientConfig
from .tools import register_tools


def build_server(
    config: ConnectMcpConfig | None = None,
    client: OhdcClient | None = None,
) -> FastMCP:
    """Construct the FastMCP server with all tools registered."""
    cfg = config or ConnectMcpConfig.from_env()
    ohdc = client or OhdcClient(
        OhdcClientConfig(storage_url=cfg.storage_url, access_token=cfg.access_token)
    )

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

    mcp: FastMCP = FastMCP(
        name="OHD Connect",
        instructions=(
            "OHD Connect is the personal-side OHDC consumer. Tools here log "
            "the user's own health events, read back their data, and manage "
            "grants / pending writes / cases / audit on their own OHD instance. "
            "Authentication is the user's self-session token; never echo the "
            "token in tool output."
        ),
        auth=auth,
    )
    register_tools(mcp, ohdc)
    return mcp


def main() -> None:
    """Entry point used by ``python -m ohd_connect_mcp`` and the console script."""
    cfg = ConnectMcpConfig.from_env()
    mcp = build_server(cfg)
    if cfg.transport == "http":
        mcp.run(transport="http", host=cfg.http_host, port=cfg.http_port)
    else:
        mcp.run()  # stdio default


if __name__ == "__main__":
    main()
