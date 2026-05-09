"""Configuration for the Care MCP server.

Care MCP is run by an operator (clinician, specialist, researcher); the
operator authenticates once via OIDC, and the MCP server holds an
**operator session token** in env. Per-patient operations are routed by the
**active grant** the LLM selects via ``switch_patient(label)``.

The grant vault is a real (not stubbed) state machine for v0:

- Loaded from env var ``OHD_CARE_GRANTS_FILE`` (a JSON file with a list of
  ``{label, grant_token, scope_summary?}``).
- ``switch_patient`` updates the active label.
- Subsequent tools read the active grant token and pass it to OHDC.

This keeps the multi-patient flow real — the LLM can ``list_patients``,
``switch_patient``, ``current_patient``, and the active patient is
automatically scoped into every read/write.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class PatientGrant:
    """One row in the operator's grant vault."""

    label: str
    grant_token: str
    scope_summary: str | None = None


@dataclass
class OidcProxyConfig:
    """OIDC settings for the FastMCP OAuth proxy.

    Per ``../spec/care-auth.md`` "Operator authentication into Care",
    a remote-deployed Care MCP server (Streamable HTTP transport) must
    sit behind a clinic OIDC provider. FastMCP's :class:`OAuthProxy`
    expects upstream `(authorize, token)` URLs plus a `client_id`; we
    discover them from the issuer's
    `.well-known/oauth-authorization-server`. Empty issuer disables the
    proxy (stdio transport is operator-local — no proxy needed).
    """

    issuer_url: str | None = None
    client_id: str | None = None
    client_secret: str | None = None
    base_url: str | None = None
    valid_scopes: list[str] = field(
        default_factory=lambda: ["openid", "profile", "email"]
    )

    @property
    def enabled(self) -> bool:
        return bool(self.issuer_url and self.client_id and self.base_url)


@dataclass
class CareMcpConfig:
    """Resolved runtime config for the Care MCP server."""

    storage_url: str
    operator_token: str | None
    transport: str  # "stdio" | "http"
    http_host: str
    http_port: int
    grants: list[PatientGrant] = field(default_factory=list)
    oidc: OidcProxyConfig = field(default_factory=OidcProxyConfig)

    @classmethod
    def from_env(cls) -> "CareMcpConfig":
        storage_url = os.environ.get("OHD_STORAGE_URL", "http://127.0.0.1:18443")
        operator_token = os.environ.get("OHD_OPERATOR_TOKEN")
        transport = os.environ.get("OHD_MCP_TRANSPORT", "stdio").lower()
        if transport not in {"stdio", "http"}:
            raise ValueError(
                f"OHD_MCP_TRANSPORT must be 'stdio' or 'http', got {transport!r}"
            )
        http_host = os.environ.get("OHD_MCP_HTTP_HOST", "127.0.0.1")
        http_port = int(os.environ.get("OHD_MCP_HTTP_PORT", "8766"))

        grants: list[PatientGrant] = []
        grants_file = os.environ.get("OHD_CARE_GRANTS_FILE")
        if grants_file:
            data = json.loads(Path(grants_file).read_text())
            for row in data:
                grants.append(
                    PatientGrant(
                        label=row["label"],
                        grant_token=row["grant_token"],
                        scope_summary=row.get("scope_summary"),
                    )
                )
        oidc = OidcProxyConfig(
            issuer_url=os.environ.get("OHD_CARE_OIDC_ISSUER") or None,
            client_id=os.environ.get("OHD_CARE_OIDC_CLIENT_ID") or None,
            client_secret=os.environ.get("OHD_CARE_OIDC_CLIENT_SECRET") or None,
            base_url=os.environ.get("OHD_CARE_MCP_BASE_URL") or None,
        )
        return cls(
            storage_url=storage_url,
            operator_token=operator_token,
            transport=transport,
            http_host=http_host,
            http_port=http_port,
            grants=grants,
            oidc=oidc,
        )
