"""Smoke tests for ohd_shared — exercise the helpers + transport surface
without touching a real OHDC backend.

Cross-language parity for ``canonical_query_hash`` (against the
TypeScript golden vectors in ``care/web``) is asserted by the consumer
tests in ``care/cli`` and ``care/mcp``. This file just ensures the
shared package is importable and the helpers round-trip.
"""

from __future__ import annotations

import pytest

from ohd_shared.canonical_query_hash import canonical_filter_json, canonical_query_hash
from ohd_shared.connect_transport import OhdcRpcError, OhdcTransport
from ohd_shared.ohdc_helpers import (
    build_filter,
    channels_from_dict_data,
    event_input_from_dict,
    event_to_dict,
    pb,
    put_result_to_dict,
    ulid_bytes_to_crockford,
    ulid_from_crockford,
)


def test_canonical_filter_default_payload():
    payload = canonical_filter_json({})
    assert payload.startswith('{"from_ms":null,"to_ms":null')
    assert payload.endswith(',"case_ulids_in":[]}')


def test_canonical_query_hash_is_64_hex():
    h = canonical_query_hash("query_events", {})
    assert len(h) == 64
    assert all(c in "0123456789abcdef" for c in h)


def test_ulid_roundtrip():
    raw = bytes(range(16))
    s = ulid_bytes_to_crockford(raw)
    assert len(s) == 26
    assert ulid_from_crockford(s) == raw


def test_ulid_invalid_length():
    with pytest.raises(ValueError):
        ulid_bytes_to_crockford(b"\x00" * 15)
    with pytest.raises(ValueError):
        ulid_from_crockford("123")


def test_channels_from_dict_data_pivots_types():
    chans = channels_from_dict_data(
        {"glucose": 6.7, "fasting": True, "unit": "mmol/L", "count": 3}
    )
    by_path = {c.channel_path: c for c in chans}
    assert by_path["glucose"].real_value == pytest.approx(6.7)
    assert by_path["fasting"].bool_value is True
    assert by_path["unit"].text_value == "mmol/L"
    assert by_path["count"].int_value == 3


def test_channels_from_dict_data_recurses_dicts():
    chans = channels_from_dict_data({"vitals": {"hr": 60, "bp": {"sys": 120}}})
    paths = sorted(c.channel_path for c in chans)
    assert paths == ["vitals.bp.sys", "vitals.hr"]


def test_channels_from_dict_data_drops_none():
    chans = channels_from_dict_data({"a": None, "b": 1})
    assert len(chans) == 1
    assert chans[0].channel_path == "b"


def test_event_input_from_dict_basic():
    ei = event_input_from_dict(
        {
            "event_type": "std.blood_glucose",
            "timestamp_ms": 1700_000_000_000,
            "data": {"value": 5.5, "unit": "mmol/L"},
            "duration_seconds": 10,
            "notes": "before breakfast",
            "source": "manual",
        }
    )
    assert ei.event_type == "std.blood_glucose"
    assert ei.timestamp_ms == 1700_000_000_000
    assert ei.duration_ms == 10_000
    assert ei.notes == "before breakfast"
    assert ei.source == "manual"
    assert len(ei.channels) == 2


def test_event_input_from_dict_duration_ms_takes_precedence():
    ei = event_input_from_dict(
        {
            "event_type": "x",
            "timestamp_ms": 1,
            "duration_ms": 500,
            "duration_seconds": 99,
        }
    )
    assert ei.duration_ms == 500


def test_build_filter_defaults_to_desc_and_includes_superseded():
    f = build_filter(
        event_type=None, from_time_ms=None, to_time_ms=None, limit=None, order="desc"
    )
    assert f.sort == pb.TIME_DESC
    assert f.include_superseded is True


def test_build_filter_asc_order():
    f = build_filter(
        event_type="std.x", from_time_ms=1, to_time_ms=2, limit=3, order="asc"
    )
    assert f.sort == pb.TIME_ASC
    assert list(f.event_types_in) == ["std.x"]
    assert f.from_ms == 1
    assert f.to_ms == 2
    assert f.limit == 3


def test_event_to_dict_minimal():
    ev = pb.Event(
        timestamp_ms=42,
        event_type="std.x",
    )
    d = event_to_dict(ev)
    assert d["timestamp_ms"] == 42
    assert d["event_type"] == "std.x"
    assert d["channels"] == []
    assert d["ulid"] is None


def test_put_result_to_dict_committed():
    r = pb.PutEventResult(
        committed=pb.PutEventCommitted(
            ulid=pb.Ulid(bytes=bytes(range(16))),
            committed_at_ms=99,
        )
    )
    d = put_result_to_dict(r)
    assert d["outcome"] == "committed"
    assert d["committed_at_ms"] == 99
    assert len(d["ulid"]) == 26


def test_transport_constructs_and_closes():
    # No network call — just construct and aclose.
    import asyncio

    async def go() -> None:
        t = OhdcTransport(base_url="http://127.0.0.1:1/")
        await t.aclose()

    asyncio.run(go())


def test_ohdc_rpc_error_carries_fields():
    err = OhdcRpcError("permission_denied", "nope", 403)
    assert err.code == "permission_denied"
    assert err.message == "nope"
    assert err.http_status == 403
    assert "permission_denied" in str(err)
