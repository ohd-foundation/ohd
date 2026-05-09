"""Operator-side config + filesystem layout.

A self-hosted Care CLI keeps per-operator state under
``$XDG_CONFIG_HOME/ohd-care`` (default ``~/.config/ohd-care``):

::

    ~/.config/ohd-care/
    ├── credentials.toml         # operator session: storage URL + (future) ohdo_… token
    ├── active.toml              # the current active patient label
    └── grants/
        └── <label>.toml         # one file per patient grant (mode 0600)

This module is purely pathing + a tiny ``Settings`` accessor. Token I/O
lives in ``credentials.py`` and ``grant_vault.py`` so the auth surface
stays narrow.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


def config_root() -> Path:
    """Return ``$XDG_CONFIG_HOME/ohd-care`` (or ``~/.config/ohd-care``).

    Honors the ``OHD_CARE_HOME`` override for tests.
    """
    override = os.environ.get("OHD_CARE_HOME")
    if override:
        return Path(override).expanduser().resolve()
    base = os.environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")
    return Path(base) / "ohd-care"


def credentials_path() -> Path:
    return config_root() / "credentials.toml"


def active_path() -> Path:
    return config_root() / "active.toml"


def grants_dir() -> Path:
    return config_root() / "grants"


def grant_path(label: str) -> Path:
    """Path to the grant file for one patient label.

    The label is sanitized for filesystem use — slashes / NULs / leading
    dots are stripped; everything else passes through. Grant files are
    addressable by the operator-typed label so an operator can find them
    by hand if they need to.
    """
    safe = sanitize_label_for_filename(label)
    return grants_dir() / f"{safe}.toml"


def sanitize_label_for_filename(label: str) -> str:
    """Sanitize a patient label for use as a filename component.

    The transform is intentionally light: the operator should be able to
    eyeball the file in ``~/.config/ohd-care/grants/`` and recognise it.
    We replace the path separator characters and NUL with ``_``; we strip
    leading dots / whitespace; we cap length at 200.
    """
    if not label:
        raise ValueError("patient label must be non-empty")
    cleaned = "".join(
        "_" if c in "/\\\x00" else c
        for c in label
    ).strip()
    cleaned = cleaned.lstrip(".")
    if not cleaned:
        raise ValueError(f"label {label!r} is empty after sanitization")
    return cleaned[:200]


def ensure_config_root() -> Path:
    """Create the config dir and grants/ subdir with mode 0700."""
    root = config_root()
    root.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(root, 0o700)
    except OSError:
        pass
    grants = grants_dir()
    grants.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(grants, 0o700)
    except OSError:
        pass
    return root


@dataclass(frozen=True)
class Settings:
    """Operator-level settings — currently just the storage URL."""

    storage_url: str
    operator_token: str | None = None  # ohdo_… session, populated by `login`; v0.1 stub
