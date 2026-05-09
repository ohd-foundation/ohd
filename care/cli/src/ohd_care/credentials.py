"""Read / write ``credentials.toml`` and ``active.toml``.

Both files live under :func:`config.config_root`.

The credentials file holds the operator's storage URL plus any
session/refresh tokens minted by the OIDC device flow. With v0.x the
file is encrypted at rest — the on-disk format is a small JSON
envelope (see :mod:`ohd_care.kms`) wrapping the inner TOML payload. To
preserve backwards compatibility with v0.1 deployments that wrote
plain TOML at this path, the loader transparently parses both shapes;
the saver always emits the envelope.

What's stored:

- ``storage_url`` — required.
- ``operator_token`` — opaque ``ohdo_…`` access token (legacy field;
  same role as ``access_token`` on new files).
- ``access_token`` — OIDC device-flow access token (post-login).
- ``refresh_token`` — OIDC device-flow refresh token (post-login).
- ``access_expires_at_ms`` — when the access token stops being valid.
- ``oidc_issuer`` — the issuer URL the user logged into.
- ``oidc_client_id`` — the client_id the device flow used.
- ``oidc_subject`` — the upstream `sub` claim, recorded so the audit
  hook can attach it alongside the bearer token (see
  ``../spec/care-auth.md`` "Two-sided audit").

``active.toml`` is a tiny pointer file with the active patient label;
it stays plaintext (no secrets, just a label string).
"""

from __future__ import annotations

import os
import sys
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Any

import tomli_w

if sys.version_info >= (3, 11):
    import tomllib  # type: ignore[import-not-found]
else:  # pragma: no cover
    import tomli as tomllib  # type: ignore[import-not-found]

from .config import (
    Settings,
    active_path,
    credentials_path,
    ensure_config_root,
)
from .kms import (
    KmsBackend,
    KmsError,
    NoneBackend,
    detect_envelope_backend,
    read_envelope_file,
    select_backend,
    write_envelope_file,
)


# ---------------------------------------------------------------------------
# Data shape
# ---------------------------------------------------------------------------

class CredentialsError(RuntimeError):
    """Raised when credentials.toml can't be loaded or is malformed."""


@dataclass
class OperatorCredentials:
    """The serialized contents of the credentials vault.

    Everything except `storage_url` is optional so we can persist a
    partial state (e.g. just the URL after `ohd-care login`, before
    OIDC is configured).
    """

    storage_url: str
    operator_token: str | None = None  # legacy ohdo_… field
    access_token: str | None = None
    refresh_token: str | None = None
    access_expires_at_ms: int | None = None
    oidc_issuer: str | None = None
    oidc_client_id: str | None = None
    oidc_subject: str | None = None

    def as_settings(self) -> Settings:
        """Project to the legacy :class:`Settings` shape used by call sites."""
        return Settings(
            storage_url=self.storage_url,
            operator_token=self.access_token or self.operator_token,
        )

    @classmethod
    def from_settings(cls, settings: Settings) -> "OperatorCredentials":
        return cls(
            storage_url=settings.storage_url,
            operator_token=settings.operator_token,
        )

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "OperatorCredentials":
        storage_url = data.get("storage_url")
        if not isinstance(storage_url, str) or not storage_url:
            raise CredentialsError("credentials missing non-empty `storage_url`")

        def _opt_str(key: str) -> str | None:
            v = data.get(key)
            if v is None:
                return None
            if not isinstance(v, str):
                raise CredentialsError(f"credentials field {key!r} must be a string")
            return v or None

        def _opt_int(key: str) -> int | None:
            v = data.get(key)
            if v is None:
                return None
            if not isinstance(v, int):
                raise CredentialsError(f"credentials field {key!r} must be an int")
            return v

        return cls(
            storage_url=storage_url,
            operator_token=_opt_str("operator_token"),
            access_token=_opt_str("access_token"),
            refresh_token=_opt_str("refresh_token"),
            access_expires_at_ms=_opt_int("access_expires_at_ms"),
            oidc_issuer=_opt_str("oidc_issuer"),
            oidc_client_id=_opt_str("oidc_client_id"),
            oidc_subject=_opt_str("oidc_subject"),
        )

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {"storage_url": self.storage_url}
        for key in (
            "operator_token",
            "access_token",
            "refresh_token",
            "access_expires_at_ms",
            "oidc_issuer",
            "oidc_client_id",
            "oidc_subject",
        ):
            v = getattr(self, key)
            if v is not None:
                out[key] = v
        return out


# ---------------------------------------------------------------------------
# I/O — top-level (URL + tokens, encrypted envelope)
# ---------------------------------------------------------------------------

def save_credentials(
    settings_or_creds: "Settings | OperatorCredentials",
    *,
    kms: KmsBackend | None = None,
) -> Path:
    """Persist credentials to ``credentials.toml`` (encrypted envelope, mode 0600).

    Accepts the legacy :class:`Settings` shape for backwards compat;
    new call sites should pass :class:`OperatorCredentials`.
    """
    creds = (
        settings_or_creds
        if isinstance(settings_or_creds, OperatorCredentials)
        else OperatorCredentials.from_settings(settings_or_creds)
    )
    backend = kms if kms is not None else select_backend("auto")
    payload = tomli_w.dumps(creds.to_dict()).encode("utf-8")
    envelope = backend.encrypt(payload)
    ensure_config_root()
    path = credentials_path()
    write_envelope_file(path, envelope)
    return path


def load_full_credentials(*, kms: KmsBackend | None = None) -> OperatorCredentials:
    """Read the credentials file, decrypting if needed.

    Backwards compat: if the file is plain TOML (legacy v0.1 shape),
    parse it directly without going through KMS.
    """
    path = credentials_path()
    if not path.is_file():
        raise CredentialsError(
            f"no operator credentials at {path}. Run `ohd-care login --storage URL` first."
        )
    raw = path.read_text()

    # Heuristic: legacy TOML files start with a key like `storage_url = ...`
    # or a comment. Envelope files are JSON objects, so they start with `{`.
    stripped = raw.lstrip()
    if stripped.startswith("{"):
        envelope = read_envelope_file(path)
        backend_name = detect_envelope_backend(envelope)
        backend = kms if kms is not None else select_backend(backend_name)
        try:
            payload = backend.decrypt(envelope)
        except KmsError as exc:
            raise CredentialsError(f"failed to decrypt {path}: {exc}") from exc
        try:
            data = tomllib.loads(payload.decode("utf-8"))
        except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
            raise CredentialsError(f"failed to parse decrypted {path}: {exc}") from exc
        return OperatorCredentials.from_dict(data)

    # Legacy plaintext TOML.
    try:
        data = tomllib.loads(raw)
    except tomllib.TOMLDecodeError as exc:
        raise CredentialsError(f"failed to parse {path}: {exc}") from exc
    return OperatorCredentials.from_dict(data)


def load_credentials() -> Settings:
    """Read ``credentials.toml`` projecting to the legacy :class:`Settings`.

    Existing call sites use this; new OIDC-aware code calls
    :func:`load_full_credentials` directly.
    """
    return load_full_credentials().as_settings()


def update_credentials(
    *,
    kms: KmsBackend | None = None,
    **fields: Any,
) -> OperatorCredentials:
    """Read, mutate, and persist a partial update to the credentials vault.

    Used by ``ohd-care oidc-login`` to drop in tokens after the device
    flow completes without losing the storage URL the operator set up
    earlier.
    """
    current = load_full_credentials(kms=kms)
    updated = replace(current, **fields)
    save_credentials(updated, kms=kms)
    return updated


# ---------------------------------------------------------------------------
# Active patient pointer (no secrets — kept as plain TOML)
# ---------------------------------------------------------------------------

def set_active_label(label: str | None) -> Path:
    """Persist the active patient label (or clear it if ``None``)."""
    ensure_config_root()
    path = active_path()
    if label is None:
        if path.exists():
            path.unlink()
        return path
    _write_toml_0600(path, {"label": label})
    return path


def get_active_label() -> str | None:
    """Return the active patient label, or ``None`` if not set."""
    path = active_path()
    if not path.is_file():
        return None
    try:
        data = tomllib.loads(path.read_text())
    except (OSError, tomllib.TOMLDecodeError):
        return None
    label = data.get("label")
    if isinstance(label, str) and label:
        return label
    return None


# ---------------------------------------------------------------------------
# Filesystem helpers
# ---------------------------------------------------------------------------

def _write_toml_0600(path: Path, payload: dict[str, Any]) -> None:
    """Write `payload` to `path` atomically with mode 0600."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    data = tomli_w.dumps(payload).encode("utf-8")
    flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC
    fd = os.open(tmp, flags, 0o600)
    try:
        os.write(fd, data)
    finally:
        os.close(fd)
    os.replace(tmp, path)
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass
