"""Operator-side audit log per ``care/SPEC.md`` §7.2 (``care_operator_audit``).

One row per OHDC RPC. The ``query_hash`` is computed locally before the
call goes out (see :mod:`canonical_query_hash`); storage records the same
hash on the patient side. Joining the two by ``(grant_id, query_hash,
ts_ms)`` recovers the cross-side audit trail (§7.3).

Persistence: a JSON-lines file under ``$OHD_CARE_HOME/operator_audit.jsonl``
(rolling 1000-entry buffer to bound disk usage; the deployment-grade
audit ships the same shape via the Postgres / SQLite backend, see
``care/cli/STATUS.md``). Mirrors the v0 web audit at
``care/web/src/ohdc/operatorAudit.ts``.
"""

from __future__ import annotations

import json
import os
import time
from collections.abc import Iterable
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Literal

from .config import config_root

_AUDIT_FILE_NAME = "operator_audit.jsonl"
_MAX_ENTRIES = 1000

OperatorAuditResult = Literal["success", "partial", "rejected", "error", "pending"]


@dataclass
class OperatorAuditEntry:
    """One operator-side audit row. Shape mirrors the eventual SQL schema
    and the web client's ``OperatorAuditEntry`` interface byte-for-byte.
    """

    ts_ms: int
    """Unix ms when the call was *issued* (pre-RPC)."""

    operator_subject: str | None
    """OIDC ``sub`` of the operator who fired the call (when available)."""

    grant_ulid: str
    """The grant ULID being used (Crockford). Empty string when unknown."""

    ohdc_action: str
    """OHDC RPC name, e.g. ``query_events``, ``put_events``."""

    query_hash: str | None
    """Canonical query hash (hex). Joins to the patient-side audit row."""

    query_kind: str | None
    """One of the storage ``query_kind`` strings; ``None`` for write /
    lifecycle RPCs that don't go through the pending-query path."""

    result: OperatorAuditResult
    """Outcome — narrowed mirror of storage's ``audit_log.result``."""

    rows_returned: int | None
    """Rows returned (read RPCs); ``None`` for writes."""

    rows_filtered: int | None
    """Rows silently filtered (read RPCs); ``None`` for writes."""

    reason: str | None
    """Optional reason / error code; surfaced on ``rejected`` / ``error``."""


def _audit_path() -> Path:
    return config_root() / _AUDIT_FILE_NAME


def append_operator_audit_entry(entry: OperatorAuditEntry) -> None:
    """Append one row to the JSONL audit log. Trims to 1000 entries.

    Best-effort: if the audit dir / file is unwritable (e.g., read-only
    filesystem in tests), the call silently no-ops. The two-sided audit
    is best-effort on the operator side; the patient-side audit is the
    durable log.
    """
    try:
        path = _audit_path()
        path.parent.mkdir(parents=True, exist_ok=True)
        # Append + trim. We keep this simple: read the current rows,
        # append, slice to 1000, rewrite. For a 1000-entry budget that's
        # well under 1 MB of JSON; the cost is negligible per-call.
        rows: list[dict] = []
        if path.exists():
            with path.open("r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        rows.append(json.loads(line))
                    except json.JSONDecodeError:
                        # Skip corrupt lines rather than failing the call;
                        # the audit is best-effort.
                        continue
        rows.append(asdict(entry))
        if len(rows) > _MAX_ENTRIES:
            rows = rows[-_MAX_ENTRIES:]
        # Rewrite atomically — temp file + rename so a crash mid-write
        # doesn't truncate the log.
        tmp = path.with_suffix(path.suffix + ".tmp")
        with tmp.open("w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r, separators=(",", ":")))
                f.write("\n")
        os.replace(tmp, path)
    except OSError:
        # Disk full / read-only / permission denied — keep the RPC path
        # going, the audit row is non-essential.
        return


def read_operator_audit() -> list[OperatorAuditEntry]:
    """Read every audit row. Newest last (append order)."""
    path = _audit_path()
    if not path.exists():
        return []
    out: list[OperatorAuditEntry] = []
    try:
        with path.open("r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    d = json.loads(line)
                except json.JSONDecodeError:
                    continue
                out.append(OperatorAuditEntry(**d))
    except OSError:
        return []
    return out


def clear_operator_audit() -> None:
    """Wipe the buffer. Test hook + ``logout`` path."""
    try:
        _audit_path().unlink()
    except OSError:
        pass


def build_audit_template(
    *,
    ohdc_action: str,
    query_kind: str | None,
    query_hash_hex: str | None,
    grant_ulid: str = "",
    operator_subject: str | None = None,
) -> OperatorAuditEntry:
    """Pre-baked record builder for read RPCs.

    Caller computes the hash via :func:`canonical_query_hash` before
    issuing the call, then either passes the resolved entry on success
    or augments with ``result`` / ``rows_*`` / ``reason`` once the RPC
    returns. Mirrors ``operatorAudit.ts``'s ``buildAuditTemplate``.
    """
    return OperatorAuditEntry(
        ts_ms=int(time.time() * 1000),
        operator_subject=operator_subject,
        grant_ulid=grant_ulid,
        ohdc_action=ohdc_action,
        query_hash=query_hash_hex,
        query_kind=query_kind,
        result="success",
        rows_returned=None,
        rows_filtered=None,
        reason=None,
    )


def _coerce_iter(rows: Iterable[OperatorAuditEntry]) -> list[OperatorAuditEntry]:
    """Helper for tests: coerce an iterable into a list."""
    return list(rows)


__all__ = [
    "OperatorAuditEntry",
    "OperatorAuditResult",
    "append_operator_audit_entry",
    "build_audit_template",
    "clear_operator_audit",
    "read_operator_audit",
]
