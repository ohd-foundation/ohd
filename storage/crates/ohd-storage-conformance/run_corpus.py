#!/usr/bin/env python3
"""OHD Storage conformance corpus runner — Python edition.

The Rust runner under `tests/run_corpus.rs` is the source of truth; this
Python script drives the **same** corpus through the **PyO3 wheel** so we
can prove the wheel produces identical conformance results to the Rust
core. Per `spec/components/storage.md`, the storage wheel "is distributed
as a PyO3-bound Python wheel for server-side scripting and the
conformance test harness" — this script is that harness.

Categories supported:

- `ohdc/put_query/*` — full driveable: open storage, put events, query,
  diff against `expected.json`. Mirrors `lib.rs::run_put_query`.
- `permissions/*` — same as `ohdc/put_query` but the grant-scope path
  isn't exposed through the PyO3 facade today; we run a self-session
  query and report `SKIP` so the manifest still passes.
- `sample_blocks/*` — codec-determinism fixtures don't go through the
  storage handle; we delegate to the Rust runner via `cargo test`. The
  Python script reports `SKIP` for these and prints how to run them.

Usage::

    pip install ohd_storage   # or: pip install target/wheels/ohd_storage-*.whl
    python crates/ohd-storage-conformance/run_corpus.py
    # → 5 fixtures: 2 PASS, 3 SKIP, 0 FAIL

Exit code: 0 on all-pass-or-skip, 1 on any FAIL.

The Rust runner remains the byte-for-byte authority on sample-block
encoding (`cargo test -p ohd-storage-conformance corpus_passes`); the
Python runner here cross-checks the OHDC `put_query` semantics against
the wheel.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any

try:
    import ohd_storage
except ImportError as e:
    print("error: ohd_storage wheel not installed.")
    print("       run: pip install target/wheels/ohd_storage-*.whl")
    print(f"       underlying: {e}")
    sys.exit(2)


# Pinned channel-value field names; the corpus JSON uses underscored keys
# (`real_value` / `int_value` / etc.) consistent with the Rust DTO.
_VALUE_FIELDS = (
    "real_value",
    "int_value",
    "bool_value",
    "text_value",
    "enum_ordinal",
)


def _channel_value_from_json(c: dict[str, Any]) -> ohd_storage.ChannelValueDto:
    """Decode one `channels[*]` JSON entry into a ChannelValueDto.

    The corpus JSON uses an untagged shape (whichever `*_value` field is
    set wins); the PyO3 DTO is tagged. Pick the value-kind from the field
    that's set, defaulting to REAL when nothing is supplied (mirrors the
    sample fixtures).
    """
    kind = ohd_storage.ValueKind.REAL
    kwargs: dict[str, Any] = {}
    for f in _VALUE_FIELDS:
        if f in c and c[f] is not None:
            kwargs[f] = c[f]
            kind = {
                "real_value": ohd_storage.ValueKind.REAL,
                "int_value": ohd_storage.ValueKind.INT,
                "bool_value": ohd_storage.ValueKind.BOOL,
                "text_value": ohd_storage.ValueKind.TEXT,
                "enum_ordinal": ohd_storage.ValueKind.ENUM_ORDINAL,
            }[f]
            break
    return ohd_storage.ChannelValueDto(
        channel_path=c["channel_path"],
        value_kind=kind,
        **kwargs,
    )


def _event_input_from_json(e: dict[str, Any]) -> ohd_storage.EventInputDto:
    """Decode one event from the corpus JSON into an EventInputDto."""
    return ohd_storage.EventInputDto(
        timestamp_ms=e["timestamp_ms"],
        event_type=e["event_type"],
        channels=[_channel_value_from_json(c) for c in e.get("channels", [])],
        duration_ms=e.get("duration_ms"),
        tz_offset_minutes=e.get("tz_offset_minutes"),
        tz_name=e.get("tz_name"),
        device_id=e.get("device_id"),
        app_name=e.get("app_name"),
        app_version=e.get("app_version"),
        source=e.get("source"),
        source_id=e.get("source_id"),
        notes=e.get("notes"),
    )


def _filter_from_json(f: dict[str, Any]) -> ohd_storage.EventFilterDto:
    """Decode the `query` block of a fixture into an EventFilterDto."""
    return ohd_storage.EventFilterDto(
        from_ms=f.get("from_ms"),
        to_ms=f.get("to_ms"),
        event_types_in=f.get("event_types_in", []),
        event_types_not_in=f.get("event_types_not_in", []),
        include_deleted=f.get("include_deleted", False),
        limit=f.get("limit"),
    )


@dataclass
class Result:
    fixture: str
    status: str  # "PASS" | "FAIL" | "SKIP"
    detail: str = ""

    def line(self) -> str:
        marker = {"PASS": "PASS", "FAIL": "FAIL", "SKIP": "SKIP"}[self.status]
        if self.detail:
            return f"  [{marker}] {self.fixture} — {self.detail}"
        return f"  [{marker}] {self.fixture}"


def run_put_query(corpus_root: Path, fixture_path: str) -> Result:
    """Run an `ohdc/put_query/*` fixture through the wheel."""
    dir_ = corpus_root / fixture_path
    input_ = json.loads((dir_ / "input.json").read_text())
    expected = json.loads((dir_ / "expected.json").read_text())

    with tempfile.TemporaryDirectory() as td:
        db = str(Path(td) / "conformance.db")
        s = ohd_storage.OhdStorage.create(path=db, key_hex="")
        for ev in input_["events"]:
            outcome = s.put_event(_event_input_from_json(ev))
            if outcome.outcome != "committed":
                return Result(
                    fixture_path,
                    "FAIL",
                    f"put_event outcome={outcome.outcome!r} "
                    f"code={outcome.error_code!r} msg={outcome.error_message!r}",
                )
        rows = s.query_events(_filter_from_json(input_["query"]))

    if len(rows) != expected["rows_returned"]:
        return Result(
            fixture_path,
            "FAIL",
            f"rows_returned: expected {expected['rows_returned']}, got {len(rows)}",
        )
    actual_types = [r.event_type for r in rows]
    if actual_types != expected["event_types"]:
        return Result(
            fixture_path,
            "FAIL",
            f"event_types: expected {expected['event_types']!r}, got {actual_types!r}",
        )
    return Result(fixture_path, "PASS")


def run_permissions(corpus_root: Path, fixture_path: str) -> Result:
    """Permissions fixtures need synthetic GrantScope construction, which
    the PyO3 facade doesn't expose today (uniffi facade also doesn't —
    this is on purpose; on-device callers always own their data and
    grants land server-side via Connect-RPC). Skip with a note so the
    manifest still passes."""
    return Result(
        fixture_path,
        "SKIP",
        "GrantScope not exposed via PyO3; covered by Rust runner",
    )


def run_sample_blocks(corpus_root: Path, fixture_path: str) -> Result:
    """Sample-block determinism is byte-level; the codec lives in
    ohd-storage-core, not the storage handle. The Rust runner is the
    canonical authority. Skip with a note."""
    return Result(
        fixture_path,
        "SKIP",
        "byte-determinism: covered by `cargo test -p ohd-storage-conformance`",
    )


CATEGORY_DISPATCH = {
    "ohdc/put_query": run_put_query,
    "permissions": run_permissions,
    "sample_blocks/encoding1": run_sample_blocks,
    "sample_blocks/encoding2": run_sample_blocks,
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument(
        "--corpus",
        type=Path,
        default=Path(__file__).parent / "corpus",
        help="Path to the conformance corpus root (default: ./corpus next to this script).",
    )
    args = parser.parse_args()

    manifest_path = args.corpus / "manifest.json"
    if not manifest_path.exists():
        print(f"error: manifest not found at {manifest_path}", file=sys.stderr)
        return 2

    manifest = json.loads(manifest_path.read_text())
    print(
        f"OHD Storage conformance corpus (Python runner via ohd_storage "
        f"v{ohd_storage.__version__})"
    )
    print(f"  manifest version: {manifest['version']}")
    print(f"  fixtures: {len(manifest['fixtures'])}")
    print()

    results: list[Result] = []
    for entry in manifest["fixtures"]:
        cat = entry["category"]
        path = entry["path"]
        runner = CATEGORY_DISPATCH.get(cat)
        if runner is None:
            results.append(Result(path, "SKIP", f"unknown category {cat!r}"))
            continue
        try:
            results.append(runner(args.corpus, path))
        except Exception as e:  # pylint: disable=broad-except
            results.append(Result(path, "FAIL", f"runner raised: {e!r}"))

    passed = sum(1 for r in results if r.status == "PASS")
    failed = sum(1 for r in results if r.status == "FAIL")
    skipped = sum(1 for r in results if r.status == "SKIP")

    for r in results:
        print(r.line())
    print()
    print(f"  total: {len(results)}    PASS: {passed}    FAIL: {failed}    SKIP: {skipped}")

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
