"""Proto <-> dict helpers for OHDC v0 messages.

Extracted from the per-MCP ``ohdc_client.py`` modules where the same
~600 LOC of proto<->dict translation lived in three byte-identical (or
near-identical) copies. The MCPs now import these helpers; the helpers
operate on the protobuf module shipped at ``ohd_shared._gen.ohdc.v0.ohdc_pb2``.

Public surface (per the union of the previous Care / Connect / Emergency
MCP copies):

- ULID Crockford codec: :func:`ulid_bytes_to_crockford`,
  :func:`ulid_from_crockford`, :func:`ulid_msg`.
- Event shape: :func:`event_to_dict`, :func:`event_input_from_dict`,
  :func:`channels_from_dict_data`, :func:`channel_value_to_dict`,
  :func:`build_filter`.
- Pending: :func:`pending_to_dict`.
- Cases: :func:`case_to_dict`.
- Grants: :func:`grant_to_dict`.
- Audit: :func:`audit_to_dict`.
- Put result: :func:`put_result_to_dict`.

These helpers are intentionally underscore-free — they're the public
shared API for proto<->dict translation. Existing MCP code that imports
the underscore-prefixed names re-exports them from each module's
``ohdc_client.py`` for back-compat as needed.
"""

from __future__ import annotations

import json as _json
from typing import Any

from ohd_shared._gen.ohdc.v0 import ohdc_pb2 as pb

# ---------- ULID Crockford codec ----------------------------------------


def ulid_bytes_to_crockford(b: bytes) -> str:
    """Encode 16 raw ULID bytes to the 26-char Crockford-base32 form.

    Implements RFC 4648-style base32 with Crockford alphabet (no I, L, O, U).
    Mirrors ``ohd_storage_core::ulid::to_crockford``.
    """
    if len(b) != 16:
        raise ValueError(f"ULID must be 16 bytes, got {len(b)}")
    alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
    # Pad to 130 bits (26 * 5) by left-padding with two zero bits.
    n = int.from_bytes(b, "big")
    out = []
    for i in range(26):
        shift = 5 * (25 - i)
        out.append(alphabet[(n >> shift) & 0x1F])
    return "".join(out)


def ulid_from_crockford(s: str) -> bytes:
    """Decode a 26-char Crockford-base32 ULID to 16 raw bytes."""
    if len(s) != 26:
        raise ValueError(f"ULID string must be 26 chars, got {len(s)}")
    alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
    s_norm = s.upper().replace("I", "1").replace("L", "1").replace("O", "0")
    n = 0
    for ch in s_norm:
        idx = alphabet.find(ch)
        if idx < 0:
            raise ValueError(f"Invalid Crockford-base32 char: {ch!r}")
        n = (n << 5) | idx
    return n.to_bytes(17, "big")[1:]


def ulid_msg(crockford: str) -> pb.Ulid:
    return pb.Ulid(bytes=ulid_from_crockford(crockford))


# ---------- ChannelValue / Event ---------------------------------------


def channel_value_to_dict(cv: pb.ChannelValue) -> dict[str, Any]:
    out: dict[str, Any] = {"channel_path": cv.channel_path}
    which = cv.WhichOneof("value")
    if which is not None:
        out[which] = getattr(cv, which)
    return out


def event_to_dict(ev: pb.Event) -> dict[str, Any]:
    d: dict[str, Any] = {
        "ulid": ulid_bytes_to_crockford(ev.ulid.bytes) if ev.ulid.bytes else None,
        "timestamp_ms": ev.timestamp_ms,
        "event_type": ev.event_type,
        "channels": [channel_value_to_dict(c) for c in ev.channels],
    }
    for opt in (
        "duration_ms",
        "tz_offset_minutes",
        "tz_name",
        "device_id",
        "app_name",
        "app_version",
        "source",
        "source_id",
        "notes",
        "deleted_at_ms",
    ):
        if ev.HasField(opt):
            d[opt] = getattr(ev, opt)
    if ev.HasField("metadata") and ev.metadata.entries:
        d["metadata"] = dict(ev.metadata.entries)
    return d


def channels_from_dict_data(data: dict[str, Any] | None) -> list[pb.ChannelValue]:
    """Best-effort flatten of a free-form ``data`` dict into ChannelValues.

    The MCP tools accept LLM-shaped or clinician-shaped payloads
    (``{"value": 6.7, "unit": "mmol/L"}``, ``{"medication": "metformin",
    "dose": "500mg"}``, etc.) which OHDC's typed-channels model wants
    pivoted into ``ChannelValue`` rows. This helper does the pivot
    heuristically:

    - bool → ``bool_value``
    - int → ``int_value``
    - float → ``real_value``
    - str → ``text_value``
    - dict → recursively walked, ``.``-joined paths
    - everything else → ``text_value`` of JSON-stringified value

    ``None`` values are dropped so callers can pass sparse payloads.
    Real production wiring should use the registry's typed schema; this is
    a v0 best-effort that lets the round-trip work for unstructured
    LLM / clinician payloads.
    """
    out: list[pb.ChannelValue] = []
    if not data:
        return out

    def _walk(prefix: str, val: Any) -> None:
        if val is None:
            return
        if isinstance(val, bool):
            out.append(pb.ChannelValue(channel_path=prefix, bool_value=val))
        elif isinstance(val, int):
            out.append(pb.ChannelValue(channel_path=prefix, int_value=val))
        elif isinstance(val, float):
            out.append(pb.ChannelValue(channel_path=prefix, real_value=val))
        elif isinstance(val, str):
            out.append(pb.ChannelValue(channel_path=prefix, text_value=val))
        elif isinstance(val, dict):
            for k, v in val.items():
                _walk(f"{prefix}.{k}" if prefix else str(k), v)
        else:
            out.append(pb.ChannelValue(channel_path=prefix, text_value=_json.dumps(val)))

    for k, v in data.items():
        _walk(str(k), v)
    return out


def event_input_from_dict(d: dict[str, Any]) -> pb.EventInput:
    """Translate the MCP-tool dict shape into ``pb.EventInput``.

    Recognized top-level keys:
        - ``event_type`` (str, required)
        - ``timestamp_ms`` (int, required)
        - ``data`` (dict) — flattened to channels via :func:`channels_from_dict_data`
        - ``duration_seconds`` (int) — multiplied to ms
        - ``duration_ms`` (int)
        - ``metadata`` (dict[str,str])
        - ``notes`` (str)
        - ``source`` / ``source_id`` / ``device_id`` / ``app_name`` / ``app_version`` (str)
    """
    ei = pb.EventInput(
        timestamp_ms=int(d["timestamp_ms"]),
        event_type=str(d["event_type"]),
        channels=channels_from_dict_data(d.get("data")),
    )
    if "duration_ms" in d and d["duration_ms"] is not None:
        ei.duration_ms = int(d["duration_ms"])
    elif "duration_seconds" in d and d["duration_seconds"] is not None:
        ei.duration_ms = int(d["duration_seconds"]) * 1000
    if d.get("notes"):
        ei.notes = str(d["notes"])
    for k in ("source", "source_id", "device_id", "app_name", "app_version"):
        if d.get(k):
            setattr(ei, k, str(d[k]))
    md = d.get("metadata")
    if md:
        for mk, mv in md.items():
            ei.metadata.entries[str(mk)] = str(mv)
    return ei


def build_filter(
    *,
    event_type: str | None,
    from_time_ms: int | None,
    to_time_ms: int | None,
    limit: int | None,
    order: str = "desc",
) -> pb.EventFilter:
    """Build a ``pb.EventFilter`` from the MCP-tool kwargs.

    Default sort is ``TIME_DESC``; ``order="asc"`` flips to ``TIME_ASC``.
    ``include_superseded`` defaults to ``True`` to match storage's Rust
    default and the canonical_query_hash payload.
    """
    f = pb.EventFilter(include_superseded=True)
    if from_time_ms is not None:
        f.from_ms = from_time_ms
    if to_time_ms is not None:
        f.to_ms = to_time_ms
    if event_type:
        f.event_types_in.append(event_type)
    if limit is not None:
        f.limit = int(limit)
    f.sort = pb.TIME_ASC if order == "asc" else pb.TIME_DESC
    return f


# ---------- Pending ----------------------------------------------------


def pending_to_dict(p: pb.PendingEvent) -> dict[str, Any]:
    return {
        "ulid": ulid_bytes_to_crockford(p.ulid.bytes) if p.ulid.bytes else None,
        "submitted_at_ms": p.submitted_at_ms,
        "submitting_grant_ulid": (
            ulid_bytes_to_crockford(p.submitting_grant_ulid.bytes)
            if p.submitting_grant_ulid.bytes
            else None
        ),
        "event": event_to_dict(p.event) if p.HasField("event") else None,
        "status": p.status,
        "expires_at_ms": p.expires_at_ms,
    }


# ---------- Cases ------------------------------------------------------


def case_to_dict(c: pb.Case) -> dict[str, Any]:
    """Translate ``pb.Case`` to a dict; mirrors event/pending shape."""
    out: dict[str, Any] = {
        "ulid": ulid_bytes_to_crockford(c.ulid.bytes) if c.ulid.bytes else None,
        "case_type": c.case_type,
        "started_at_ms": int(c.started_at_ms),
        "last_activity_at_ms": int(c.last_activity_at_ms),
    }
    if c.HasField("case_label"):
        out["case_label"] = c.case_label
    if c.HasField("ended_at_ms"):
        out["ended_at_ms"] = int(c.ended_at_ms)
    if c.HasField("parent_case_ulid") and c.parent_case_ulid.bytes:
        out["parent_case_ulid"] = ulid_bytes_to_crockford(c.parent_case_ulid.bytes)
    if c.HasField("predecessor_case_ulid") and c.predecessor_case_ulid.bytes:
        out["predecessor_case_ulid"] = ulid_bytes_to_crockford(
            c.predecessor_case_ulid.bytes
        )
    if (
        c.HasField("opening_authority_grant_ulid")
        and c.opening_authority_grant_ulid.bytes
    ):
        out["opening_authority_grant_ulid"] = ulid_bytes_to_crockford(
            c.opening_authority_grant_ulid.bytes
        )
    if c.HasField("inactivity_close_after_h"):
        out["inactivity_close_after_h"] = int(c.inactivity_close_after_h)
    return out


# ---------- Grants -----------------------------------------------------


def grant_to_dict(g: pb.Grant) -> dict[str, Any]:
    d: dict[str, Any] = {
        "ulid": ulid_bytes_to_crockford(g.ulid.bytes) if g.ulid.bytes else None,
        "grantee_label": g.grantee_label,
        "grantee_kind": g.grantee_kind,
        "created_at_ms": g.created_at_ms,
        "default_action": g.default_action,
        "approval_mode": g.approval_mode,
        "aggregation_only": g.aggregation_only,
        "strip_notes": g.strip_notes,
        "use_count": g.use_count,
    }
    for opt in ("purpose", "expires_at_ms", "revoked_at_ms", "last_used_ms"):
        if g.HasField(opt):
            d[opt] = getattr(g, opt)
    return d


# ---------- Audit ------------------------------------------------------


def audit_to_dict(a: pb.AuditEntry) -> dict[str, Any]:
    d: dict[str, Any] = {
        "ts_ms": a.ts_ms,
        "actor_type": a.actor_type,
        "action": a.action,
        "query_kind": a.query_kind,
        "query_params_json": a.query_params_json,
        "result": a.result,
    }
    if a.HasField("grant_ulid"):
        d["grant_ulid"] = ulid_bytes_to_crockford(a.grant_ulid.bytes)
    for opt in ("rows_returned", "rows_filtered", "reason", "caller_ip", "caller_ua"):
        if a.HasField(opt):
            d[opt] = getattr(a, opt)
    return d


# ---------- Put result -------------------------------------------------


def put_result_to_dict(r: pb.PutEventResult) -> dict[str, Any]:
    which = r.WhichOneof("outcome")
    match which:
        case "committed":
            return {
                "outcome": "committed",
                "ulid": ulid_bytes_to_crockford(r.committed.ulid.bytes),
                "committed_at_ms": r.committed.committed_at_ms,
            }
        case "pending":
            return {
                "outcome": "pending",
                "ulid": ulid_bytes_to_crockford(r.pending.ulid.bytes),
                "expires_at_ms": r.pending.expires_at_ms,
            }
        case "error":
            return {
                "outcome": "error",
                "code": r.error.code,
                "message": r.error.message,
                "metadata": dict(r.error.metadata),
            }
        case _:
            return {"outcome": None}


__all__ = [
    "audit_to_dict",
    "build_filter",
    "case_to_dict",
    "channel_value_to_dict",
    "channels_from_dict_data",
    "event_input_from_dict",
    "event_to_dict",
    "grant_to_dict",
    "pb",
    "pending_to_dict",
    "put_result_to_dict",
    "ulid_bytes_to_crockford",
    "ulid_from_crockford",
    "ulid_msg",
]
