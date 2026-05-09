"""Per-patient grant token vault, on-disk variant.

Mirrors the in-memory vault in ``care/mcp/src/ohd_care_mcp/grant_vault.py``
but with file-backed storage suitable for a CLI: one TOML per patient under
``~/.config/ohd-care/grants/<label>.toml``, mode 0600.

TODO(kms): v0.1 ships plaintext mode-0600 + filesystem-encryption (LUKS /
FileVault / BitLocker) as the trust posture for the per-patient grant
files. v0.x should encrypt these with the same envelope abstraction the
operator-credentials vault now uses (see ``kms.py``). The shape is:

- replace `GrantVault.save` / `.load` with envelope-encrypt / envelope-
  decrypt round-trips through `select_backend()`;
- keep the file extension `.toml` (so operators can still recognize them)
  but write the JSON envelope inside;
- add a one-shot migration step that re-keys existing plaintext files.

The operator-credentials file went encrypted-at-rest in this pass per
``../spec/care-auth.md`` "Token storage on the Care server (encryption)".
The grant-file work is mechanical now that the KMS abstraction is in
place; gating on multi-grant Care use lifting the threat-model
priority.
"""

from __future__ import annotations

import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import tomli_w
from pydantic import BaseModel, Field

if sys.version_info >= (3, 11):
    import tomllib  # type: ignore[import-not-found]
else:  # pragma: no cover
    import tomli as tomllib  # type: ignore[import-not-found]

from .config import ensure_config_root, grant_path, grants_dir


class GrantVaultError(RuntimeError):
    """Base class for grant vault errors."""


class UnknownPatientError(GrantVaultError):
    def __init__(self, label: str) -> None:
        super().__init__(
            f"No grant for patient label {label!r}. Use `ohd-care patients` "
            "to list known patients."
        )
        self.label = label


class NoActivePatientError(GrantVaultError):
    def __init__(self) -> None:
        super().__init__(
            "No active patient. Run `ohd-care use <label>` to pick one "
            "(see `ohd-care patients` for available labels)."
        )


class GrantConflictError(GrantVaultError):
    """Adding a patient that already exists without --force."""


class PatientGrant(BaseModel):
    """One row of the operator's grant vault."""

    label: str = Field(..., description="Operator-typed patient label")
    grant_token: str = Field(..., description="Plaintext ohdg_… (TODO: KMS-encrypt)")
    storage_url: str | None = Field(
        default=None,
        description="Per-patient storage URL; falls back to global `storage_url` from credentials.toml",
    )
    storage_cert_pin_sha256_hex: str | None = Field(
        default=None,
        description="SHA-256 of the expected TLS cert when self-signed via relay",
    )
    grant_ulid: str | None = Field(
        default=None, description="Crockford-base32 ULID of the grant; informational"
    )
    case_ulids: list[str] = Field(
        default_factory=list,
        description="Case ULIDs the grant references (Crockford-base32). Empty = open scope.",
    )
    scope_summary: str | None = Field(default=None, description="Human-readable scope notes")
    expires_at_ms: int | None = Field(
        default=None, description="Mirror of grant's expiry (informational)"
    )
    imported_at_ms: int | None = Field(default=None)
    notes: str | None = Field(default=None)

    @classmethod
    def from_dict(cls, label: str, data: dict[str, Any]) -> "PatientGrant":
        # `label` is the file's logical name; we let the on-disk `label`
        # override (in case the operator renames the file).
        merged = {"label": label, **data}
        return cls.model_validate(merged)

    def to_toml(self) -> str:
        # Pydantic emits None for unset Optional fields; tomli_w doesn't
        # know how to serialize None — drop them.
        as_dict = {
            k: v
            for k, v in self.model_dump().items()
            if v is not None and v != []
        }
        return tomli_w.dumps(as_dict)


@dataclass
class GrantVault:
    """Filesystem-backed grant vault.

    Each patient is one TOML file under :func:`config.grants_dir`. We
    re-scan on every operation rather than keeping an in-memory cache —
    the CLI is short-lived, and freshness beats microseconds.
    """

    def list_labels(self) -> list[str]:
        d = grants_dir()
        if not d.is_dir():
            return []
        out: list[str] = []
        for path in sorted(d.iterdir()):
            if path.is_file() and path.suffix == ".toml":
                out.append(path.stem)
        return out

    def list_grants(self) -> list[PatientGrant]:
        return [self.load(label) for label in self.list_labels()]

    def has(self, label: str) -> bool:
        return grant_path(label).is_file()

    def load(self, label: str) -> PatientGrant:
        path = grant_path(label)
        if not path.is_file():
            raise UnknownPatientError(label)
        try:
            data = tomllib.loads(path.read_text())
        except (OSError, tomllib.TOMLDecodeError) as exc:
            raise GrantVaultError(f"failed to read {path}: {exc}") from exc
        return PatientGrant.from_dict(label=label, data=data)

    def save(self, grant: PatientGrant, *, force: bool = False) -> Path:
        ensure_config_root()
        path = grant_path(grant.label)
        if path.exists() and not force:
            raise GrantConflictError(
                f"grant for label {grant.label!r} already exists at {path}; "
                "pass --force to overwrite"
            )
        body = grant.to_toml().encode("utf-8")
        tmp = path.with_suffix(path.suffix + ".tmp")
        flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC
        fd = os.open(tmp, flags, 0o600)
        try:
            os.write(fd, body)
        finally:
            os.close(fd)
        os.replace(tmp, path)
        try:
            os.chmod(path, 0o600)
        except OSError:
            pass
        return path

    def remove(self, label: str) -> bool:
        path = grant_path(label)
        if not path.is_file():
            return False
        path.unlink()
        return True
