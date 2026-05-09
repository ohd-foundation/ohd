"""Cross-language parity test for ``canonical_query_hash`` (Care MCP).

Loads the same JSON vectors that ``care/web/src/ohdc/canonicalQueryHash.test.ts``
and ``care/cli/tests/test_canonical_query_hash.py`` assert against. When all
three pass, the TypeScript (web), Python-cli, and Python-mcp implementations
are byte-identical — the operator-side audit JOIN per ``care/SPEC.md`` §7.3
holds across every Care surface.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from ohd_care_mcp.canonical_query_hash import (
    canonical_filter_json,
    canonical_query_hash,
)

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
            f"missing shared vectors at {_VECTORS_PATH}; "
            "cross-language parity check disabled. Run `pnpm test` in "
            "care/web/ to regenerate."
        )
    return json.loads(_VECTORS_PATH.read_text(encoding="utf-8"))


@pytest.mark.parametrize("vector", _load_vectors(), ids=lambda v: v["name"])
def test_canonical_payload_matches_storage(vector: dict) -> None:
    got = canonical_filter_json(vector["filter"])
    assert got == vector["canonical_payload"]


@pytest.mark.parametrize("vector", _load_vectors(), ids=lambda v: v["name"])
def test_canonical_query_hash_matches_ts(vector: dict) -> None:
    expected = vector["expected_hex"]
    if expected == "PLACEHOLDER_FILLED_BY_TEST":
        pytest.skip(
            f"vector {vector['name']!r} has a placeholder expected_hex; "
            "fill it from `pnpm test` output before relying on parity"
        )
    got = canonical_query_hash(vector["query_kind"], vector["filter"])
    assert got == expected


def test_default_filter_canonical_payload_is_storage_compatible() -> None:
    payload = canonical_filter_json({})
    assert payload.startswith("{\"from_ms\":null,\"to_ms\":null")
    assert payload.endswith(",\"case_ulids_in\":[]}")


def test_camelcase_input_matches_snake_case_input() -> None:
    a = canonical_filter_json({"fromMs": 1700, "toMs": 1701, "limit": 5})
    b = canonical_filter_json({"from_ms": 1700, "to_ms": 1701, "limit": 5})
    assert a == b


def test_cli_and_mcp_implementations_agree() -> None:
    """Ensure the cli and mcp implementations are byte-identical.

    Both modules are independently distributed and could drift if a future
    edit lands in only one. We assert both produce the same hash for a
    representative non-trivial filter so any drift fails loudly.
    """
    # Import lazily — care/cli/ might not be on the path in some CI shapes.
    import importlib.util

    cli_path = (
        Path(__file__).resolve().parents[2]
        / "cli"
        / "src"
        / "ohd_care"
        / "canonical_query_hash.py"
    )
    if not cli_path.exists():
        pytest.skip(f"sister cli module not present at {cli_path}")
    spec = importlib.util.spec_from_file_location("ohd_care_canonical", cli_path)
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)

    filter_ = {
        "fromMs": 1_700_000_000_000,
        "toMs": 1_700_001_000_000,
        "eventTypesIn": ["std.blood_glucose", "std.heart_rate_resting"],
        "limit": 100,
    }
    cli_hash = mod.canonical_query_hash("query_events", filter_)
    mcp_hash = canonical_query_hash("query_events", filter_)
    assert cli_hash == mcp_hash
