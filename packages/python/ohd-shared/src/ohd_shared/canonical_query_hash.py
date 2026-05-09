"""Canonical OHDC query-hash — byte-identical to ``care/web/src/ohdc/canonicalQueryHash.ts``.

Single source of truth for the CLI + the three MCPs (Care, Connect,
Emergency). Previously copied byte-for-byte into ``care/cli`` and
``care/mcp``; now imported from ``ohd_shared``.

This is the operator-side mirror of storage's ``pending_queries::enqueue`` /
``lookup_decision`` algorithm. Any drift between this implementation, the
TypeScript implementation in ``care/web``, and storage's
``serde_json::to_string(filter)`` breaks the two-sided audit JOIN per
``care/SPEC.md`` §7.3 (and the per-query approval dedup path in
``storage/spec/privacy-access.md``).

Algorithm, per ``storage/crates/ohd-storage-core/src/pending_queries.rs``::

    sha256(query_kind || 0x00 || serde_json::to_string(filter))

stored as the hex-encoded 32-byte digest.

``serde_json::to_string`` for the storage ``EventFilter`` struct emits, in
**declaration order**::

    from_ms, to_ms, event_types_in, event_types_not_in, include_deleted,
    include_superseded, limit, device_id_in, source_in, event_ulids_in,
    sensitivity_classes_in, sensitivity_classes_not_in, channel_predicates,
    case_ulids_in

Notes:

* ``Option<T>`` fields with no ``#[serde(skip_serializing_if = …)]``
  serialize to ``null`` when ``None`` — they are NOT omitted. We mirror
  that by emitting ``null`` for unset numeric fields.
* ``Vec<T>`` defaults to ``[]``.
* ``include_superseded`` defaults to ``true`` (Rust ``default_true``).
* ``include_deleted`` defaults to ``false``.
* Compact JSON (no whitespace) matches ``to_string`` (vs. ``to_string_pretty``).

Cross-language byte-for-byte identicality is asserted by the golden test
vectors at ``care/web/src/ohdc/__golden__/query_hash_vectors.json``; the
CLI's ``tests/test_canonical_query_hash.py`` loads the same JSON.
"""

from __future__ import annotations

import hashlib
import json
from collections.abc import Mapping, Sequence
from typing import Any, Literal, TypedDict

# --- Public types -----------------------------------------------------------


CanonicalQueryKind = Literal[
    "query_events",
    "aggregate",
    "correlate",
    "read_samples",
    "read_attachment",
    "get_event_by_ulid",
]


class CanonicalChannelPredicate(TypedDict, total=False):
    """Mirrors the Rust ``ChannelPredicate`` field order — channel_path, op, value.

    The ``value`` shape mirrors ``ChannelScalar`` from ``events.rs``: an
    externally-tagged enum like ``{"Real": 1.0}`` / ``{"Int": 2}`` /
    ``{"Bool": true}`` / ``{"Text": "x"}`` / ``{"EnumOrdinal": 3}``.

    Either snake_case (``channel_path``) or camelCase (``channelPath``) is
    accepted on input — :func:`canonical_filter_json` normalizes.
    """

    channel_path: str
    channelPath: str
    op: str
    value: Mapping[str, Any]


class CanonicalEventFilter(TypedDict, total=False):
    """Storage-aligned ``EventFilter`` shape.

    Mirrors the Rust struct field order; only the fields that go on the
    wire are part of the canonical hash. Either snake_case or camelCase
    keys are accepted on input (camelCase mirrors the TS type for
    cross-language test parity).
    """

    # snake_case (Pythonic)
    from_ms: int | None
    to_ms: int | None
    event_types_in: Sequence[str]
    event_types_not_in: Sequence[str]
    include_deleted: bool
    include_superseded: bool
    limit: int | None
    device_id_in: Sequence[str]
    source_in: Sequence[str]
    event_ulids_in: Sequence[str]
    sensitivity_classes_in: Sequence[str]
    sensitivity_classes_not_in: Sequence[str]
    channel_predicates: Sequence[CanonicalChannelPredicate]
    case_ulids_in: Sequence[str]
    # camelCase (matches TS source-of-truth — useful for shared JSON vectors)
    fromMs: int | None
    toMs: int | None
    eventTypesIn: Sequence[str]
    eventTypesNotIn: Sequence[str]
    includeDeleted: bool
    includeSuperseded: bool
    deviceIdIn: Sequence[str]
    sourceIn: Sequence[str]
    eventUlidsIn: Sequence[str]
    sensitivityClassesIn: Sequence[str]
    sensitivityClassesNotIn: Sequence[str]
    channelPredicates: Sequence[CanonicalChannelPredicate]
    caseUlidsIn: Sequence[str]


# --- Internals --------------------------------------------------------------


def _pick(d: Mapping[str, Any], snake: str, camel: str, default: Any) -> Any:
    if snake in d:
        return d[snake]
    if camel in d:
        return d[camel]
    return default


def _num_or_null(v: Any) -> int | None:
    """Mirror TS ``numOrNull``: None / undefined → null; bigint → int.

    The hash payload uses JSON numbers for ``from_ms`` / ``to_ms`` /
    ``limit``; storage's i64 JSON encoding is a plain integer.
    """
    if v is None:
        return None
    # bool is an int subclass in Python — preserve it as-is (canonical_filter
    # only ever sees the numeric fields here, not bools, but be defensive).
    if isinstance(v, bool):
        raise TypeError("numeric field cannot be bool")
    if isinstance(v, int):
        return int(v)
    if isinstance(v, float):
        if v.is_integer():
            return int(v)
        raise ValueError(f"non-integer float {v!r} not representable as i64")
    raise TypeError(f"unexpected numeric type {type(v).__name__}")


def _canonical_predicate(p: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "channel_path": _pick(p, "channel_path", "channelPath", ""),
        "op": p.get("op", ""),
        "value": p.get("value", {}),
    }


def canonical_filter_json(filter_: Mapping[str, Any] | None) -> str:
    """Render the canonical JSON for a filter. Pure function; no I/O.

    Useful for inspecting what the audit row will key on. Matches
    ``serde_json::to_string`` byte-for-byte: compact (no whitespace), keys
    in declaration order, default values emitted explicitly.
    """
    f: Mapping[str, Any] = filter_ or {}
    obj = {
        "from_ms": _num_or_null(_pick(f, "from_ms", "fromMs", None)),
        "to_ms": _num_or_null(_pick(f, "to_ms", "toMs", None)),
        "event_types_in": list(_pick(f, "event_types_in", "eventTypesIn", [])),
        "event_types_not_in": list(
            _pick(f, "event_types_not_in", "eventTypesNotIn", [])
        ),
        "include_deleted": bool(_pick(f, "include_deleted", "includeDeleted", False)),
        "include_superseded": bool(
            _pick(f, "include_superseded", "includeSuperseded", True)
        ),
        "limit": _num_or_null(f.get("limit", None)),
        "device_id_in": list(_pick(f, "device_id_in", "deviceIdIn", [])),
        "source_in": list(_pick(f, "source_in", "sourceIn", [])),
        "event_ulids_in": list(_pick(f, "event_ulids_in", "eventUlidsIn", [])),
        "sensitivity_classes_in": list(
            _pick(f, "sensitivity_classes_in", "sensitivityClassesIn", [])
        ),
        "sensitivity_classes_not_in": list(
            _pick(f, "sensitivity_classes_not_in", "sensitivityClassesNotIn", [])
        ),
        "channel_predicates": [
            _canonical_predicate(p)
            for p in _pick(f, "channel_predicates", "channelPredicates", [])
        ],
        "case_ulids_in": list(_pick(f, "case_ulids_in", "caseUlidsIn", [])),
    }
    # ``json.dumps`` with separators=(",", ":") matches
    # ``serde_json::to_string`` compact form: no whitespace between tokens.
    # ensure_ascii=False mirrors serde_json's UTF-8-by-default behaviour;
    # allow_nan=False matches serde_json (which emits null for NaN/Inf rather
    # than producing invalid JSON tokens).
    return json.dumps(obj, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def canonical_query_hash(
    query_kind: str, filter_: Mapping[str, Any] | None
) -> str:
    """Compute the byte-identical query hash that storage records on the
    patient side. Returns hex of the 32-byte SHA-256 digest.

    Used both to dedup pending approvals on the storage side and to JOIN the
    operator-side audit row to the patient-side audit row per
    ``care/SPEC.md`` §7.3.
    """
    payload = canonical_filter_json(filter_)
    h = hashlib.sha256()
    h.update(query_kind.encode("utf-8"))
    h.update(b"\x00")
    h.update(payload.encode("utf-8"))
    return h.hexdigest()


__all__ = [
    "CanonicalChannelPredicate",
    "CanonicalEventFilter",
    "CanonicalQueryKind",
    "canonical_filter_json",
    "canonical_query_hash",
]
