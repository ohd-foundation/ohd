"""Subcommand modules for the OHD Care CLI.

Each module exposes a `register(group)` function that adds its commands
to the top-level click group built in ``ohd_care.cli``. Splitting the
surface this way keeps ``cli.py`` short and lets tests target one group
at a time.
"""

from __future__ import annotations

from . import audit, login, patients, pending, query, submit

__all__ = ["audit", "login", "patients", "pending", "query", "submit"]
