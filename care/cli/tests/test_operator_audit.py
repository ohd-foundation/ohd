"""Tests for ``ohd_care.operator_audit``.

Mirrors ``care/web/src/ohdc/operatorAudit.ts``: append → read → trim
semantics + best-effort persistence + clearing. Per
``care/SPEC.md`` §7.2.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from ohd_care.operator_audit import (
    OperatorAuditEntry,
    append_operator_audit_entry,
    build_audit_template,
    clear_operator_audit,
    read_operator_audit,
)


@pytest.fixture
def audit_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Point ``OHD_CARE_HOME`` at a tmp dir so audit writes are isolated."""
    monkeypatch.setenv("OHD_CARE_HOME", str(tmp_path))
    return tmp_path


def test_build_audit_template_defaults_match_web_shape() -> None:
    e = build_audit_template(
        ohdc_action="query_events",
        query_kind="query_events",
        query_hash_hex="abcd",
    )
    assert e.ohdc_action == "query_events"
    assert e.query_kind == "query_events"
    assert e.query_hash == "abcd"
    assert e.result == "success"  # the template is "presumed success" until finished
    assert e.rows_returned is None
    assert e.rows_filtered is None
    assert e.reason is None
    assert e.grant_ulid == ""


def test_append_and_read_round_trips(audit_home: Path) -> None:
    e = build_audit_template(
        ohdc_action="put_events",
        query_kind=None,
        query_hash_hex=None,
        operator_subject="oidc-sub-123",
    )
    e.result = "success"
    append_operator_audit_entry(e)

    rows = read_operator_audit()
    assert len(rows) == 1
    assert rows[0].ohdc_action == "put_events"
    assert rows[0].operator_subject == "oidc-sub-123"
    assert rows[0].result == "success"


def test_append_trims_to_max_entries(audit_home: Path) -> None:
    for i in range(1100):
        e = OperatorAuditEntry(
            ts_ms=i,
            operator_subject=None,
            grant_ulid="",
            ohdc_action="query_events",
            query_hash="00",
            query_kind="query_events",
            result="success",
            rows_returned=0,
            rows_filtered=0,
            reason=None,
        )
        append_operator_audit_entry(e)
    rows = read_operator_audit()
    # Exactly the 1000 most-recent entries (ts_ms 100..1099).
    assert len(rows) == 1000
    assert rows[0].ts_ms == 100
    assert rows[-1].ts_ms == 1099


def test_clear_operator_audit_removes_file(audit_home: Path) -> None:
    e = build_audit_template(
        ohdc_action="query_events", query_kind="query_events", query_hash_hex="ab"
    )
    append_operator_audit_entry(e)
    assert (audit_home / "operator_audit.jsonl").exists()
    clear_operator_audit()
    assert not (audit_home / "operator_audit.jsonl").exists()
    assert read_operator_audit() == []


def test_corrupt_lines_are_skipped(audit_home: Path) -> None:
    """A corrupt line shouldn't break the reader — best-effort."""
    e = build_audit_template(
        ohdc_action="query_events", query_kind="query_events", query_hash_hex="ab"
    )
    append_operator_audit_entry(e)
    path = audit_home / "operator_audit.jsonl"
    # Inject a corrupt line + a valid one.
    with path.open("a", encoding="utf-8") as f:
        f.write("not-json\n")
        f.write(
            json.dumps(
                {
                    "ts_ms": 9999,
                    "operator_subject": None,
                    "grant_ulid": "",
                    "ohdc_action": "query_events",
                    "query_hash": "cd",
                    "query_kind": "query_events",
                    "result": "success",
                    "rows_returned": 1,
                    "rows_filtered": 0,
                    "reason": None,
                }
            )
            + "\n"
        )
    rows = read_operator_audit()
    # Original + the appended valid line; corrupt line silently skipped.
    assert len(rows) == 2
    assert rows[-1].ts_ms == 9999
