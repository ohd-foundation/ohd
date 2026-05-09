"""FastMCP OAuthProxy bootstrap, shared by the three MCPs.

Each MCP (Care, Connect, Emergency) used to inline a near-identical
``_build_oauth_proxy`` + ``_discover`` pair. This module factors out the
discovery + proxy-construction so a single update to the OIDC bootstrap
applies everywhere.

The MCPs each have their own ``OidcProxyConfig`` dataclass shape — they
differ only in the type used for ``valid_scopes`` (Care uses a ``list``,
Connect/Emergency use a ``tuple``). :func:`build_oauth_proxy` accepts any
object that satisfies the :class:`OidcProxyConfigLike` shape (a small
typing.Protocol).

Optional dep: this module imports ``fastmcp`` lazily at call time so the
shared package can be installed by consumers (e.g. the CLI) that don't
need OAuth.
"""

from __future__ import annotations

from typing import Any, Protocol, runtime_checkable
from urllib.parse import urljoin

import httpx


@runtime_checkable
class OidcProxyConfigLike(Protocol):
    """Structural shape required by :func:`build_oauth_proxy`.

    Any dataclass with these attributes works; consumers don't need to
    import a shared base class. Care / Connect / Emergency MCP all
    define an ``OidcProxyConfig`` that satisfies this Protocol.
    """

    issuer_url: str | None
    client_id: str | None
    client_secret: str | None
    base_url: str | None
    valid_scopes: Any  # list[str] | tuple[str, ...] — coerced to list at use


def discover(issuer: str, *, timeout: float = 10.0) -> dict:
    """Fetch the OAuth AS metadata document, with OIDC fallback.

    Tries RFC 8414 ``/.well-known/oauth-authorization-server`` first and
    falls back to ``/.well-known/openid-configuration`` on 404.
    """
    primary = urljoin(issuer.rstrip("/") + "/", ".well-known/oauth-authorization-server")
    fallback = issuer.rstrip("/") + "/.well-known/openid-configuration"
    with httpx.Client(timeout=timeout) as client:
        resp = client.get(primary, headers={"accept": "application/json"})
        if resp.status_code == 404:
            resp = client.get(fallback, headers={"accept": "application/json"})
        resp.raise_for_status()
        return resp.json()


def build_oauth_proxy(oidc: OidcProxyConfigLike) -> Any:
    """Wire FastMCP's :class:`OAuthProxy` against the configured OIDC issuer.

    Synchronously discovers the upstream ``(authorize, token)`` endpoints
    via ``.well-known/oauth-authorization-server`` so the server boots
    with the right routing. Discovery is one HTTP GET; if it fails the
    caller should log and fall back to no-auth (stdio works fine without
    OIDC; Streamable HTTP without auth is dev-only).

    Raises:
        RuntimeError: if the discovery doc lacks ``jwks_uri`` (OAuthProxy
            needs it to verify upstream tokens).
        AssertionError: if any of ``issuer_url`` / ``client_id`` /
            ``base_url`` is missing on the input config.
    """
    # Lazy import so the shared package can be installed without fastmcp.
    from fastmcp.server.auth.oauth_proxy import OAuthProxy
    from fastmcp.server.auth.providers.jwt import JWTVerifier

    assert oidc.issuer_url and oidc.client_id and oidc.base_url
    discovery = discover(oidc.issuer_url)
    authorize = discovery["authorization_endpoint"]
    token_endpoint = discovery["token_endpoint"]
    jwks_uri = discovery.get("jwks_uri")
    if not jwks_uri:
        raise RuntimeError(
            f"OIDC issuer {oidc.issuer_url} discovery missing `jwks_uri`; "
            "OAuthProxy needs it to verify upstream tokens."
        )

    verifier = JWTVerifier(
        jwks_uri=jwks_uri, issuer=discovery.get("issuer", oidc.issuer_url)
    )
    return OAuthProxy(
        upstream_authorization_endpoint=authorize,
        upstream_token_endpoint=token_endpoint,
        upstream_client_id=oidc.client_id,
        upstream_client_secret=oidc.client_secret,
        token_verifier=verifier,
        base_url=oidc.base_url,
        valid_scopes=list(oidc.valid_scopes),
    )


__all__ = [
    "OidcProxyConfigLike",
    "build_oauth_proxy",
    "discover",
]
