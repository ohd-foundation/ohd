"""``ohd-care login`` and ``ohd-care oidc-login`` — set up the operator session.

There are two flows here:

- ``ohd-care login --storage URL`` records the storage URL (and
  optionally a manually-supplied operator token). This was the only
  v0.1 flow.
- ``ohd-care oidc-login --issuer ISSUER --client-id ID`` runs the
  OAuth 2.0 Device Authorization Grant (RFC 8628) against the clinic's
  OIDC provider — Google Workspace, Microsoft Entra, Okta, Keycloak,
  Authentik, or storage's own OAuth AS — and persists the resulting
  ``access_token`` / ``refresh_token`` to the encrypted vault. Per
  ``../../spec/care-auth.md`` "Operator authentication into Care".

The vault is encrypted at rest via the OS keyring by default, with a
passphrase fallback for headless machines (CI, Docker without a
session bus). Pick the backend explicitly via ``--kms-backend
keyring|passphrase|none`` or via the ``OHD_CARE_KMS_BACKEND`` env var.
"""

from __future__ import annotations

import logging
import os
import sys

import click

from ..credentials import (
    CredentialsError,
    OperatorCredentials,
    load_full_credentials,
    save_credentials,
    update_credentials,
)
from ..kms import VALID_BACKENDS, select_backend


_log = logging.getLogger(__name__)
from ..oidc import (
    OidcDeviceFlowError,
    OidcDiscoveryError,
    discover,
    poll_device_token,
    start_device_flow,
)


_KMS_FLAG = click.option(
    "--kms-backend",
    "kms_backend",
    type=click.Choice(["auto", *VALID_BACKENDS]),
    default="auto",
    show_default=True,
    help="KMS backend for the credential vault. `auto` tries OS keyring then passphrase.",
)


@click.command("login", help="Configure the operator's storage URL (and stub for OIDC).")
@click.option(
    "--storage",
    "storage_url",
    default="http://localhost:8443",
    show_default=True,
    help="OHDC storage URL the operator will reach (per-patient grants override this).",
)
@click.option(
    "--operator-token",
    "operator_token",
    default=None,
    help=(
        "Operator OIDC session token (`ohdo_…`). v0.1 stub — prefer `oidc-login` "
        "for real OAuth 2.0 Device Authorization Grant flow."
    ),
)
@_KMS_FLAG
def login(storage_url: str, operator_token: str | None, kms_backend: str) -> None:
    backend = select_backend(kms_backend)
    creds = OperatorCredentials(
        storage_url=storage_url,
        operator_token=operator_token,
    )
    path = save_credentials(creds, kms=backend)
    click.echo(f"saved operator credentials to {path}")
    click.echo(f"  storage_url:    {creds.storage_url}")
    click.echo(f"  kms_backend:    {backend.name}")
    if operator_token:
        click.echo("  operator_token: <set>")
    else:
        click.echo(
            "  operator_token: <unset> "
            "(run `ohd-care oidc-login` for the real OIDC device flow)"
        )


@click.command(
    "oidc-login",
    help="Run OAuth 2.0 Device Authorization Grant against the clinic OIDC provider.",
)
@click.option(
    "--issuer",
    "issuer",
    required=True,
    help=(
        "OIDC / OAuth issuer URL (e.g. https://accounts.google.com, "
        "https://login.microsoftonline.com/<tenant>/v2.0, "
        "https://sso.clinic.example/realms/care). The CLI fetches the "
        "AS metadata from this URL via .well-known."
    ),
)
@click.option(
    "--client-id",
    "client_id",
    required=True,
    help="OAuth client_id registered with the issuer for this CLI.",
)
@click.option(
    "--scope",
    "scope",
    default="openid profile email offline_access",
    show_default=True,
    help="OAuth/OIDC scopes (space-separated).",
)
@click.option(
    "--storage",
    "storage_url",
    default=None,
    help="Override or set the storage URL the operator session targets.",
)
@_KMS_FLAG
def oidc_login(
    issuer: str,
    client_id: str,
    scope: str,
    storage_url: str | None,
    kms_backend: str,
) -> None:
    """Start the device flow and persist tokens to the encrypted vault."""
    backend = select_backend(kms_backend)
    client_secret = os.environ.get("OHD_CARE_OIDC_CLIENT_SECRET") or None

    click.echo(f"discovering OIDC issuer: {issuer}")
    try:
        discovery = discover(issuer)
    except OidcDiscoveryError as exc:
        click.echo(f"discovery failed: {exc}", err=True)
        sys.exit(2)

    click.echo(f"  token_endpoint:  {discovery.token_endpoint}")
    click.echo(f"  device_endpoint: {discovery.device_authorization_endpoint}")

    try:
        device = start_device_flow(
            discovery,
            client_id=client_id,
            scope=scope,
        )
    except OidcDeviceFlowError as exc:
        click.echo(f"device-flow start failed: {exc}", err=True)
        sys.exit(2)

    click.echo("")
    click.echo(f"  Open this URL on any browser: {device.verification_uri}")
    if device.verification_uri_complete:
        click.echo(f"  (or this one to skip code entry: {device.verification_uri_complete})")
    click.echo(f"  Enter user code:              {device.user_code}")
    click.echo(f"  Code expires in:              {device.expires_in}s")
    click.echo("")
    click.echo("Waiting for confirmation… (Ctrl-C to abort)")

    def _on_pending(interval: int, seconds_left: int) -> None:
        # Print at most one waiting message per minute so we don't spam.
        if seconds_left % 60 < interval:
            click.echo(f"  …still waiting ({seconds_left}s remaining)")

    try:
        token = poll_device_token(
            discovery,
            device,
            client_id=client_id,
            client_secret=client_secret,
            on_pending=_on_pending,
        )
    except OidcDeviceFlowError as exc:
        click.echo(f"device-flow poll failed: {exc}", err=True)
        sys.exit(2)

    expires_at_ms: int | None = None
    if token.expires_in:
        import time

        expires_at_ms = int(time.time() * 1000) + token.expires_in * 1000

    # Merge into existing credentials if any (preserve storage URL).
    try:
        current = load_full_credentials(kms=backend)
        target_url = storage_url or current.storage_url
    except CredentialsError as exc:
        _log.debug("no existing credentials to merge: %s", exc)
        target_url = storage_url or "http://localhost:8443"
        current = None

    creds = OperatorCredentials(
        storage_url=target_url,
        access_token=token.access_token,
        refresh_token=token.refresh_token,
        access_expires_at_ms=expires_at_ms,
        oidc_issuer=discovery.issuer,
        oidc_client_id=client_id,
        oidc_subject=token.oidc_subject,
        # Keep any pre-existing legacy operator_token so we don't drop
        # configuration the operator manually set up.
        operator_token=(current.operator_token if current is not None else None),
    )
    path = save_credentials(creds, kms=backend)
    click.echo("")
    click.echo(f"saved operator credentials to {path}")
    click.echo(f"  storage_url:    {creds.storage_url}")
    click.echo(f"  kms_backend:    {backend.name}")
    click.echo(f"  oidc_issuer:    {creds.oidc_issuer}")
    if token.oidc_subject:
        click.echo(f"  oidc_subject:   {token.oidc_subject}")
    click.echo(f"  access_token:   <set, expires_in={token.expires_in}s>")
    if token.refresh_token:
        click.echo("  refresh_token:  <set>")


@click.command("logout", help="Drop tokens from the credential vault (keep storage URL).")
@_KMS_FLAG
def logout(kms_backend: str) -> None:
    backend = select_backend(kms_backend)
    try:
        update_credentials(
            kms=backend,
            operator_token=None,
            access_token=None,
            refresh_token=None,
            access_expires_at_ms=None,
            oidc_subject=None,
        )
    except Exception as exc:
        click.echo(f"logout failed: {exc}", err=True)
        sys.exit(2)
    click.echo("operator tokens cleared from the vault.")
