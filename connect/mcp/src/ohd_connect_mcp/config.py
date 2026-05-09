"""Configuration for the Connect MCP server.

Reads from environment variables. Two modes:

- **Local stdio (Claude Desktop install)** — ``OHD_STORAGE_URL`` plus a
  pre-issued self-session token in ``OHD_ACCESS_TOKEN``. The MCP host (the
  LLM client) launches the server as a subprocess and the LLM never sees
  the token.
- **Remote Streamable HTTP** — ``OHD_STORAGE_URL`` plus an OAuth proxy
  configured by FastMCP. Token acquisition happens per-session; this
  module carries both the storage URL and the OIDC issuer/client_id so
  the proxy can be wired at startup.

The OIDC issuer for Connect is the storage AS itself (Storage acts as
the OAuth Authorization Server toward Connect). For deployments that
delegate to an upstream IdP directly (Auth0 / Keycloak / Authentik
hosting their own Connect MCP) the issuer can be pointed at that IdP —
the proxy doesn't care.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field


@dataclass(frozen=True)
class OidcProxyConfig:
    """OIDC settings for FastMCP's :class:`OAuthProxy` wiring.

    Empty issuer disables the proxy (stdio transport is local).
    """

    issuer_url: str | None = None
    client_id: str | None = None
    client_secret: str | None = None
    base_url: str | None = None
    valid_scopes: tuple[str, ...] = ("openid", "profile", "email", "offline_access")

    @property
    def enabled(self) -> bool:
        return bool(self.issuer_url and self.client_id and self.base_url)


@dataclass(frozen=True)
class ConnectMcpConfig:
    """Resolved runtime config for the Connect MCP server."""

    storage_url: str
    access_token: str | None
    transport: str  # "stdio" | "http"
    http_host: str
    http_port: int
    oidc: OidcProxyConfig = field(default_factory=OidcProxyConfig)

    @classmethod
    def from_env(cls) -> "ConnectMcpConfig":
        storage_url = os.environ.get("OHD_STORAGE_URL", "http://127.0.0.1:18443")
        access_token = os.environ.get("OHD_ACCESS_TOKEN")
        transport = os.environ.get("OHD_MCP_TRANSPORT", "stdio").lower()
        if transport not in {"stdio", "http"}:
            raise ValueError(
                f"OHD_MCP_TRANSPORT must be 'stdio' or 'http', got {transport!r}"
            )
        http_host = os.environ.get("OHD_MCP_HTTP_HOST", "127.0.0.1")
        http_port = int(os.environ.get("OHD_MCP_HTTP_PORT", "8765"))
        oidc = OidcProxyConfig(
            issuer_url=os.environ.get("OHD_CONNECT_OIDC_ISSUER")
            or os.environ.get("OHD_OIDC_ISSUER")
            or None,
            client_id=os.environ.get("OHD_CONNECT_OIDC_CLIENT_ID")
            or os.environ.get("OHD_OIDC_CLIENT_ID")
            or None,
            client_secret=os.environ.get("OHD_CONNECT_OIDC_CLIENT_SECRET")
            or os.environ.get("OHD_OIDC_CLIENT_SECRET")
            or None,
            base_url=os.environ.get("OHD_CONNECT_MCP_BASE_URL") or None,
        )
        return cls(
            storage_url=storage_url,
            access_token=access_token,
            transport=transport,
            http_host=http_host,
            http_port=http_port,
            oidc=oidc,
        )
