"""OIDC / OAuth 2.0 Device Authorization Grant client.

Implements the Care operator's CLI login flow per
``../../spec/care-auth.md`` "Operator authentication into Care" and the
device-flow shape from ``../../spec/auth.md`` "CLI clients
(``ohd-connect``, ``ohd-care``)". Storage / Care's auth server speaks
standard RFC 8628 over OAuth-2 endpoints; we drive that wire here using
``httpx`` and stdlib JSON.

Design notes:

- The flow is provider-agnostic. The CLI accepts an `--issuer` URL plus
  a `--client-id`. For most setups the issuer is OHD Storage's own
  OAuth AS (Storage acts as RP toward upstream IdPs — see
  ``spec/docs/design/auth.md`` "Role split"); for clinic SSO use cases
  the issuer can also point straight at Google Workspace, Microsoft
  Entra, Okta, or Keycloak. The CLI doesn't care: it discovers the
  device endpoint via `.well-known/oauth-authorization-server` (RFC
  8414).
- For client credentials we default to "public client" semantics: the
  CLI ships no client secret. If the operator's issuer requires one
  (Azure AD's confidential apps, custom Keycloak realms), pass it via
  the `OHD_CARE_OIDC_CLIENT_SECRET` env var rather than putting it on
  the command line.
- We don't implement OIDC ID-token verification here. Storage already
  validates the upstream provider's id_token in its OIDC RP role and
  returns its own opaque ``ohds_…`` (or operator-flavor ``ohdo_…``)
  session token. The CLI just bears the opaque token.

Wire surface used:

- `GET <issuer>/.well-known/oauth-authorization-server` — discover the
  `device_authorization_endpoint` and `token_endpoint`.
- `POST <device_authorization_endpoint>` — start a device flow; receive
  `(device_code, user_code, verification_uri, expires_in, interval)`.
- `POST <token_endpoint>` (grant_type=device_code) — poll until
  authorized; receive `(access_token, refresh_token, expires_in)`.
- `POST <token_endpoint>` (grant_type=refresh_token) — silent refresh.
"""

from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Any
from urllib.parse import urljoin

import httpx


# RFC 8628 well-known polling errors. "authorization_pending" and
# "slow_down" are non-terminal — we keep polling. Everything else is
# fatal.
_PENDING_ERRORS = frozenset({"authorization_pending", "slow_down"})


class OidcError(RuntimeError):
    """Base class for OIDC / OAuth flow errors."""


class OidcDiscoveryError(OidcError):
    """`/.well-known/oauth-authorization-server` failed or omitted required fields."""


class OidcDeviceFlowError(OidcError):
    """Token endpoint returned a terminal error (other than authorization_pending/slow_down)."""

    def __init__(self, code: str, description: str | None = None) -> None:
        msg = f"{code}: {description}" if description else code
        super().__init__(msg)
        self.code = code
        self.description = description


class OidcRefreshError(OidcError):
    """Refresh-token exchange failed (revoked, expired, or rotated outside grace)."""


@dataclass(frozen=True)
class DiscoveryDocument:
    """Subset of OAuth 2.0 Authorization Server Metadata (RFC 8414) we need."""

    issuer: str
    token_endpoint: str
    device_authorization_endpoint: str
    authorization_endpoint: str | None = None
    registration_endpoint: str | None = None

    @classmethod
    def from_json(cls, doc: dict[str, Any]) -> "DiscoveryDocument":
        try:
            return cls(
                issuer=str(doc["issuer"]),
                token_endpoint=str(doc["token_endpoint"]),
                device_authorization_endpoint=str(doc["device_authorization_endpoint"]),
                authorization_endpoint=(
                    str(doc["authorization_endpoint"])
                    if "authorization_endpoint" in doc
                    else None
                ),
                registration_endpoint=(
                    str(doc["registration_endpoint"])
                    if "registration_endpoint" in doc
                    else None
                ),
            )
        except KeyError as exc:
            raise OidcDiscoveryError(
                f"discovery document missing required field {exc!s}"
            ) from exc


@dataclass(frozen=True)
class DeviceCodeResponse:
    """Result of POSTing to ``device_authorization_endpoint`` (RFC 8628 §3.2)."""

    device_code: str
    user_code: str
    verification_uri: str
    verification_uri_complete: str | None
    expires_in: int
    interval: int


@dataclass(frozen=True)
class TokenResponse:
    """Result of a successful ``token_endpoint`` exchange."""

    access_token: str
    token_type: str
    expires_in: int | None = None
    refresh_token: str | None = None
    id_token: str | None = None
    scope: str | None = None
    # `oidc_subject` is OHD-specific: when storage acts as the RP it
    # echoes the upstream `sub` claim alongside the opaque token so the
    # CLI can attach it to its audit hook (per care-auth.md "Two-sided
    # audit"). This is a non-standard extension; storage emits it only
    # for the OHD Storage AS, not for arbitrary upstream issuers.
    oidc_subject: str | None = None
    oidc_issuer: str | None = None

    @classmethod
    def from_json(cls, doc: dict[str, Any]) -> "TokenResponse":
        return cls(
            access_token=str(doc["access_token"]),
            token_type=str(doc.get("token_type", "Bearer")),
            expires_in=int(doc["expires_in"]) if "expires_in" in doc else None,
            refresh_token=(str(doc["refresh_token"]) if "refresh_token" in doc else None),
            id_token=(str(doc["id_token"]) if "id_token" in doc else None),
            scope=(str(doc["scope"]) if "scope" in doc else None),
            oidc_subject=(str(doc["oidc_subject"]) if "oidc_subject" in doc else None),
            oidc_issuer=(str(doc["oidc_issuer"]) if "oidc_issuer" in doc else None),
        )


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------

def _well_known_url(issuer: str) -> str:
    """Compute the AS-Metadata URL per RFC 8414 §3."""
    if not issuer.endswith("/"):
        issuer = issuer + "/"
    return urljoin(issuer, ".well-known/oauth-authorization-server")


def discover(issuer: str, *, timeout: float = 10.0, verify: bool | str = True) -> DiscoveryDocument:
    """Fetch the AS metadata for ``issuer``.

    If the issuer is OHD Storage / OHD Care's own OAuth AS the document
    will already include the device-flow endpoint. For stricter
    OpenID-Connect-only issuers (like Google) the device-flow endpoint
    is in the OIDC discovery doc (`/.well-known/openid-configuration`)
    instead — we fall back to that on 404.
    """
    url = _well_known_url(issuer)
    with httpx.Client(timeout=timeout, verify=verify) as client:
        try:
            resp = client.get(url, headers={"accept": "application/json"})
        except httpx.HTTPError as exc:
            raise OidcDiscoveryError(f"discovery: {exc}") from exc
        if resp.status_code == 404:
            # Fall back to OIDC discovery doc (Google / Auth0 / Keycloak).
            base = issuer.rstrip("/")
            alt = base + "/.well-known/openid-configuration"
            try:
                resp = client.get(alt, headers={"accept": "application/json"})
            except httpx.HTTPError as exc:
                raise OidcDiscoveryError(f"discovery: {exc}") from exc
        if resp.status_code != 200:
            raise OidcDiscoveryError(
                f"discovery returned HTTP {resp.status_code} from {resp.request.url}"
            )
        try:
            doc = resp.json()
        except ValueError as exc:
            raise OidcDiscoveryError(f"discovery returned invalid JSON: {exc}") from exc
    return DiscoveryDocument.from_json(doc)


# ---------------------------------------------------------------------------
# Device flow
# ---------------------------------------------------------------------------

def start_device_flow(
    discovery: DiscoveryDocument,
    *,
    client_id: str,
    scope: str = "openid profile email offline_access",
    extra_params: dict[str, str] | None = None,
    timeout: float = 10.0,
    verify: bool | str = True,
) -> DeviceCodeResponse:
    """Initiate the device flow (RFC 8628 §3.1)."""
    body: dict[str, str] = {"client_id": client_id, "scope": scope}
    if extra_params:
        body.update(extra_params)
    with httpx.Client(timeout=timeout, verify=verify) as client:
        try:
            resp = client.post(
                discovery.device_authorization_endpoint,
                data=body,
                headers={"accept": "application/json"},
            )
        except httpx.HTTPError as exc:
            raise OidcDeviceFlowError("transport_error", str(exc)) from exc
    if resp.status_code != 200:
        try:
            payload = resp.json()
            code = str(payload.get("error", f"http_{resp.status_code}"))
            desc = payload.get("error_description")
        except ValueError:
            code = f"http_{resp.status_code}"
            desc = resp.text[:200]
        raise OidcDeviceFlowError(code, desc)
    payload = resp.json()
    return DeviceCodeResponse(
        device_code=str(payload["device_code"]),
        user_code=str(payload["user_code"]),
        verification_uri=str(payload["verification_uri"]),
        verification_uri_complete=(
            str(payload["verification_uri_complete"])
            if "verification_uri_complete" in payload
            else None
        ),
        expires_in=int(payload["expires_in"]),
        interval=int(payload.get("interval", 5)),
    )


def poll_device_token(
    discovery: DiscoveryDocument,
    device_code: DeviceCodeResponse,
    *,
    client_id: str,
    client_secret: str | None = None,
    timeout: float = 10.0,
    verify: bool | str = True,
    on_pending: "callable[[int, int], None] | None" = None,
    sleep: "callable[[float], None]" = time.sleep,
    now: "callable[[], float]" = time.monotonic,
) -> TokenResponse:
    """Poll the token endpoint until success or timeout (RFC 8628 §3.4-§3.5).

    `on_pending(interval, seconds_left)` is invoked once per polling
    iteration — the CLI uses it to print a "still waiting…" line.
    `sleep` and `now` are injectable for tests.
    """
    deadline = now() + device_code.expires_in
    interval = max(1, device_code.interval)
    while True:
        if now() >= deadline:
            raise OidcDeviceFlowError("expired_token", "device code expired before user confirmed")
        body = {
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            "device_code": device_code.device_code,
            "client_id": client_id,
        }
        auth = (client_id, client_secret) if client_secret else None
        with httpx.Client(timeout=timeout, verify=verify) as client:
            try:
                resp = client.post(
                    discovery.token_endpoint,
                    data=body,
                    auth=auth,
                    headers={"accept": "application/json"},
                )
            except httpx.HTTPError as exc:
                raise OidcDeviceFlowError("transport_error", str(exc)) from exc
        if resp.status_code == 200:
            return TokenResponse.from_json(resp.json())
        # RFC 8628 §3.5: 4xx on pending. Parse error body.
        try:
            payload = resp.json()
            err = str(payload.get("error", f"http_{resp.status_code}"))
            desc = payload.get("error_description")
        except ValueError:
            err = f"http_{resp.status_code}"
            desc = resp.text[:200]
        if err == "slow_down":
            interval = interval + 5
        if err in _PENDING_ERRORS:
            if on_pending is not None:
                on_pending(interval, max(0, int(deadline - now())))
            sleep(interval)
            continue
        # Terminal failure.
        raise OidcDeviceFlowError(err, desc)


def refresh_access_token(
    discovery: DiscoveryDocument,
    refresh_token: str,
    *,
    client_id: str,
    client_secret: str | None = None,
    timeout: float = 10.0,
    verify: bool | str = True,
) -> TokenResponse:
    """Exchange a refresh token for a new access token (RFC 6749 §6)."""
    body = {
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": client_id,
    }
    auth = (client_id, client_secret) if client_secret else None
    with httpx.Client(timeout=timeout, verify=verify) as client:
        try:
            resp = client.post(
                discovery.token_endpoint,
                data=body,
                auth=auth,
                headers={"accept": "application/json"},
            )
        except httpx.HTTPError as exc:
            raise OidcRefreshError(f"transport_error: {exc}") from exc
    if resp.status_code != 200:
        try:
            payload = resp.json()
            err = str(payload.get("error", f"http_{resp.status_code}"))
            desc = payload.get("error_description") or ""
        except ValueError:
            err = f"http_{resp.status_code}"
            desc = resp.text[:200]
        raise OidcRefreshError(f"{err}: {desc}".rstrip(": "))
    return TokenResponse.from_json(resp.json())
