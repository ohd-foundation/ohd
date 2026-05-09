"""Configuration for the Emergency MCP server.

Per ``emergency/SPEC.md`` §3.1 the operator authenticates via OIDC (or a
pre-shared key for stdio installs); the MCP attaches the **active case
grant** the operator selects via ``set_active_case(case_id)`` (analogous to
Care MCP's ``switch_patient``). For v0 the active case is held in-memory
on the server and seeded from a JSON file (``OHD_EMERGENCY_CASES_FILE``).

Per §3.3 ``OHD_EMERGENCY_MCP_ALLOW_EXTERNAL_LLM`` defaults to false; when
false, the MCP server marks itself as PHI-restricted in its tool
descriptions. Origin enforcement at the transport layer is a wire-up
agent's job (the FastMCP OAuth proxy / origin allowlist).

Streamable-HTTP transport supports FastMCP's :class:`OAuthProxy` against
the operator IdP. Wire it via the env vars below; mirror of
``connect/mcp`` and ``care/mcp``:

- ``OHD_EMERGENCY_OIDC_ISSUER`` — operator IdP issuer URL.
- ``OHD_EMERGENCY_OIDC_CLIENT_ID`` — OAuth client_id registered with
  the issuer for the Emergency MCP.
- ``OHD_EMERGENCY_OIDC_CLIENT_SECRET`` — confidential-client secret.
- ``OHD_EMERGENCY_MCP_BASE_URL`` — public URL the MCP is exposed at
  (the OAuth proxy needs it for redirect-URI rendering).

When any of those is missing, the OAuth proxy is disabled — useful for
``stdio`` Claude Desktop installs where the operator already issued a
pre-shared bearer.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class CaseGrant:
    """One row in the operator's case-grant vault."""

    case_id: str
    grant_token: str
    label: str | None = None  # human-readable label for the active case


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


@dataclass
class EmergencyMcpConfig:
    storage_url: str
    operator_token: str | None
    allow_external_llm: bool
    transport: str  # "stdio" | "http"
    http_host: str
    http_port: int
    cases: list[CaseGrant] = field(default_factory=list)
    oidc: OidcProxyConfig = field(default_factory=OidcProxyConfig)

    @classmethod
    def from_env(cls) -> "EmergencyMcpConfig":
        storage_url = os.environ.get("OHD_STORAGE_URL", "http://127.0.0.1:18443")
        operator_token = os.environ.get("OHD_OPERATOR_TOKEN")
        allow_external_llm = (
            os.environ.get("OHD_EMERGENCY_MCP_ALLOW_EXTERNAL_LLM", "false").lower()
            in {"1", "true", "yes"}
        )
        transport = os.environ.get("OHD_MCP_TRANSPORT", "stdio").lower()
        if transport not in {"stdio", "http"}:
            raise ValueError(
                f"OHD_MCP_TRANSPORT must be 'stdio' or 'http', got {transport!r}"
            )
        http_host = os.environ.get("OHD_MCP_HTTP_HOST", "127.0.0.1")
        http_port = int(os.environ.get("OHD_MCP_HTTP_PORT", "8767"))

        cases: list[CaseGrant] = []
        cases_file = os.environ.get("OHD_EMERGENCY_CASES_FILE")
        if cases_file:
            data = json.loads(Path(cases_file).read_text())
            for row in data:
                cases.append(
                    CaseGrant(
                        case_id=row["case_id"],
                        grant_token=row["grant_token"],
                        label=row.get("label"),
                    )
                )

        oidc = OidcProxyConfig(
            issuer_url=os.environ.get("OHD_EMERGENCY_OIDC_ISSUER")
            or os.environ.get("OHD_OIDC_ISSUER")
            or None,
            client_id=os.environ.get("OHD_EMERGENCY_OIDC_CLIENT_ID")
            or os.environ.get("OHD_OIDC_CLIENT_ID")
            or None,
            client_secret=os.environ.get("OHD_EMERGENCY_OIDC_CLIENT_SECRET")
            or os.environ.get("OHD_OIDC_CLIENT_SECRET")
            or None,
            base_url=os.environ.get("OHD_EMERGENCY_MCP_BASE_URL") or None,
        )

        return cls(
            storage_url=storage_url,
            operator_token=operator_token,
            allow_external_llm=allow_external_llm,
            transport=transport,
            http_host=http_host,
            http_port=http_port,
            cases=cases,
            oidc=oidc,
        )
