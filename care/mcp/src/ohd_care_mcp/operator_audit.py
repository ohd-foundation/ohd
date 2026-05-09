"""Operator-side audit log for Care MCP per ``care/SPEC.md`` §7.2.

Mirrors ``care/cli/src/ohd_care/operator_audit.py`` and
``care/web/src/ohdc/operatorAudit.ts`` byte-for-byte in shape; only the
persistence layer differs:

- Care MCP runs as a per-operator-session daemon. Persist under
  ``$OHD_CARE_MCP_AUDIT_DIR/operator_audit.jsonl`` if the env var is set,
  else hold in-memory only (the MCP can be ephemeral; the patient-side
  audit is the durable log and remains the unforgeable source).
- The 1000-entry rolling buffer matches the web + cli budget so
  cross-component analysis tooling can treat all three the same way.
"""

from __future__ import annotations

import json
import os
import threading
import time
from collections.abc import Iterable
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Literal

_AUDIT_FILE_NAME = "operator_audit.jsonl"
_MAX_ENTRIES = 1000
_AUDIT_DIR_ENV = "OHD_CARE_MCP_AUDIT_DIR"

OperatorAuditResult = Literal["success", "partial", "rejected", "error", "pending"]


@dataclass
class OperatorAuditEntry:
    """One operator-side audit row. Same shape as the cli + web mirror."""

    ts_ms: int
    operator_subject: str | None
    grant_ulid: str
    ohdc_action: str
    query_hash: str | None
    query_kind: str | None
    result: OperatorAuditResult
    rows_returned: int | None
    rows_filtered: int | None
    reason: str | None


_lock = threading.Lock()
_in_memory: list[OperatorAuditEntry] = []


def _audit_path() -> Path | None:
    base = os.environ.get(_AUDIT_DIR_ENV)
    if not base:
        return None
    return Path(base).expanduser().resolve() / _AUDIT_FILE_NAME


def append_operator_audit_entry(entry: OperatorAuditEntry) -> None:
    """Append one row. Trims to the rolling 1000-entry buffer.

    Persistent path is taken if ``OHD_CARE_MCP_AUDIT_DIR`` is set;
    otherwise in-memory only. Best-effort either way.
    """
    with _lock:
        _in_memory.append(entry)
        if len(_in_memory) > _MAX_ENTRIES:
            del _in_memory[: len(_in_memory) - _MAX_ENTRIES]
    path = _audit_path()
    if path is None:
        return
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        # Same atomic-rewrite strategy as care/cli — keep the JSONL
        # tidy and trimmed even on long-running MCP sessions.
        with _lock:
            rows = [asdict(e) for e in _in_memory]
        tmp = path.with_suffix(path.suffix + ".tmp")
        with tmp.open("w", encoding="utf-8") as f:
            for r in rows:
                f.write(json.dumps(r, separators=(",", ":")))
                f.write("\n")
        os.replace(tmp, path)
    except OSError:
        # Disk full / read-only / permission denied — keep the RPC
        # path going.
        return


def read_operator_audit() -> list[OperatorAuditEntry]:
    """Read every audit row. Newest last (append order)."""
    with _lock:
        return list(_in_memory)


def clear_operator_audit() -> None:
    """Wipe the buffer + on-disk file. Test hook + ``logout`` path."""
    with _lock:
        _in_memory.clear()
    path = _audit_path()
    if path is not None:
        try:
            path.unlink()
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
    """Pre-baked record builder for read RPCs. Same shape as cli."""
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
    return list(rows)


__all__ = [
    "OperatorAuditEntry",
    "OperatorAuditResult",
    "append_operator_audit_entry",
    "build_audit_template",
    "clear_operator_audit",
    "read_operator_audit",
]
