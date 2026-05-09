"""Cross-language parity test for ``canonical_query_hash``.

Loads the same JSON vectors that ``care/web/src/ohdc/canonicalQueryHash.test.ts``
asserts against. When this test passes here AND there, the TS and Python
implementations are byte-identical — the operator-side audit JOIN per
``care/SPEC.md`` §7.3 holds.

The vector file lives under ``care/web/src/ohdc/__golden__/`` and is
shared rather than duplicated.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from ohd_care.canonical_query_hash import canonical_filter_json, canonical_query_hash

# Resolve the shared vectors at import time so a missing path fails fast
# rather than mid-parametrize.
_VECTORS_PATH = (
    Path(__file__).resolve().parents[2]
    / "web"
    / "src"
    / "ohdc"
    / "__golden__"
    / "query_hash_vectors.json"
)


def _load_vectors() -> list[dict]:
    if not _VECTORS_PATH.exists():
        raise FileNotFoundError(
            f"missing shared vectors at {_VECTORS_PATH}; cross-language "
            "parity check disabled. Run `pnpm test` in care/web/ to "
            "regenerate."
        )
    return json.loads(_VECTORS_PATH.read_text(encoding="utf-8"))


@pytest.mark.parametrize("vector", _load_vectors(), ids=lambda v: v["name"])
def test_canonical_payload_matches_storage(vector: dict) -> None:
    """The canonical JSON we emit matches storage's
    ``serde_json::to_string(filter)`` byte-for-byte."""
    got = canonical_filter_json(vector["filter"])
    assert got == vector["canonical_payload"]


@pytest.mark.parametrize("vector", _load_vectors(), ids=lambda v: v["name"])
def test_canonical_query_hash_matches_ts(vector: dict) -> None:
    """The hex SHA-256 we emit matches the TS reference byte-for-byte."""
    expected = vector["expected_hex"]
    if expected == "PLACEHOLDER_FILLED_BY_TEST":
        pytest.skip(
            f"vector {vector['name']!r} has a placeholder expected_hex; "
            "fill it from `pnpm test` output before relying on parity"
        )
    got = canonical_query_hash(vector["query_kind"], vector["filter"])
    assert got == expected


def test_default_filter_canonical_payload_is_storage_compatible() -> None:
    """Sanity check: the empty filter produces the documented payload.

    The default-filter payload is literal in
    ``canonicalQueryHash.ts``'s docstring — keep this test in sync if
    storage changes its struct field order.
    """
    payload = canonical_filter_json({})
    assert "from_ms" in payload
    assert payload.startswith("{\"from_ms\":null,\"to_ms\":null")
    assert payload.endswith(",\"case_ulids_in\":[]}")


def test_camelcase_input_matches_snake_case_input() -> None:
    """The Python helper accepts both naming styles for cross-test parity."""
    a = canonical_filter_json({"fromMs": 1700, "toMs": 1701, "limit": 5})
    b = canonical_filter_json({"from_ms": 1700, "to_ms": 1701, "limit": 5})
    assert a == b
