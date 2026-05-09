"""Small helpers shared across the CLI command modules.

- ULID Crockford-base32 encode / decode.
- ISO8601 + ``--last-day``/``--last-week``/``--last-month`` parsing.
- Event-type alias resolution (``temperature`` → ``std.body_temperature``).
- One-place table renderer.
"""

from __future__ import annotations

import datetime as dt
from collections.abc import Iterable, Iterator, Sequence
from dataclasses import dataclass

# ---------------------------------------------------------------------------
# Crockford base32 (ULID)
# ---------------------------------------------------------------------------
# https://www.crockford.com/base32.html — RFC 4648 with the substitutions
# 0/O, 1/I/L, U excluded (case-insensitive on decode).

_CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
_CROCKFORD_DECODE = {c: i for i, c in enumerate(_CROCKFORD)}
# Forgiving aliases used by the spec.
for _src, _dst in (("I", "1"), ("L", "1"), ("O", "0")):
    _CROCKFORD_DECODE[_src] = _CROCKFORD_DECODE[_dst]


def ulid_to_crockford(b: bytes) -> str:
    """Encode 16 bytes (a ULID) as 26 Crockford-base32 characters."""
    if len(b) != 16:
        raise ValueError(f"ULID must be 16 bytes, got {len(b)}")
    # ULIDs encode as 26 base32 chars where the first char only carries
    # 3 bits (the high 2 bits are zero). We left-pad to 130 bits, then
    # take 26 groups of 5 bits.
    n = int.from_bytes(b, "big")
    out: list[str] = []
    for i in range(26):
        shift = (25 - i) * 5
        out.append(_CROCKFORD[(n >> shift) & 0x1F])
    return "".join(out)


def crockford_to_ulid(s: str) -> bytes:
    """Decode a 26-char Crockford-base32 string to 16 raw ULID bytes."""
    s = s.strip().upper()
    if len(s) != 26:
        raise ValueError(f"ULID string must be 26 chars, got {len(s)}: {s!r}")
    n = 0
    for ch in s:
        try:
            n = (n << 5) | _CROCKFORD_DECODE[ch]
        except KeyError as exc:
            raise ValueError(f"invalid Crockford char {ch!r} in {s!r}") from exc
    return n.to_bytes(16, "big")


# ---------------------------------------------------------------------------
# Time
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class TimeRange:
    """Inclusive ``from_ms`` / exclusive ``to_ms``; either may be ``None``."""

    from_ms: int | None
    to_ms: int | None


def parse_iso(s: str) -> int:
    """Parse an ISO8601 datetime to Unix milliseconds (UTC)."""
    # Accept date-only as start-of-day UTC.
    if len(s) == 10 and s[4] == "-" and s[7] == "-":
        return int(
            dt.datetime.fromisoformat(s).replace(tzinfo=dt.UTC).timestamp() * 1000
        )
    # Accept trailing Z.
    s2 = s.replace("Z", "+00:00")
    parsed = dt.datetime.fromisoformat(s2)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=dt.UTC)
    return int(parsed.timestamp() * 1000)


def render_ms(ms: int) -> str:
    """Render Unix milliseconds as ``YYYY-MM-DD HH:MM:SS UTC``."""
    return dt.datetime.fromtimestamp(ms / 1000, tz=dt.UTC).strftime("%Y-%m-%d %H:%M:%S UTC")


def now_ms() -> int:
    return int(dt.datetime.now(tz=dt.UTC).timestamp() * 1000)


def build_range(
    *,
    last_day: bool = False,
    last_week: bool = False,
    last_month: bool = False,
    last_72h: bool = False,
    from_iso: str | None = None,
    to_iso: str | None = None,
) -> TimeRange:
    """Resolve mutually-exclusive ``--last-*`` / ``--from`` / ``--to`` flags."""
    flags = sum(bool(x) for x in (last_day, last_week, last_month, last_72h))
    if flags > 1:
        raise ValueError("--last-* flags are mutually exclusive")
    if flags > 0 and (from_iso or to_iso):
        raise ValueError("--last-* flags can't be combined with --from / --to")

    now = now_ms()
    if last_day:
        return TimeRange(from_ms=now - 86_400_000, to_ms=None)
    if last_week:
        return TimeRange(from_ms=now - 7 * 86_400_000, to_ms=None)
    if last_month:
        return TimeRange(from_ms=now - 30 * 86_400_000, to_ms=None)
    if last_72h:
        return TimeRange(from_ms=now - 72 * 3_600_000, to_ms=None)
    return TimeRange(
        from_ms=parse_iso(from_iso) if from_iso else None,
        to_ms=parse_iso(to_iso) if to_iso else None,
    )


# ---------------------------------------------------------------------------
# Event-type aliases
# ---------------------------------------------------------------------------

EVENT_TYPE_ALIASES = {
    "glucose": "std.blood_glucose",
    "blood_glucose": "std.blood_glucose",
    "heart_rate": "std.heart_rate_resting",
    "heart-rate": "std.heart_rate_resting",
    "temperature": "std.body_temperature",
    "body_temperature": "std.body_temperature",
    "medication": "std.medication_dose",
    "medications": "std.medication_dose",
    "medication_taken": "std.medication_dose",
    "symptom": "std.symptom",
    "symptoms": "std.symptom",
    "note": "std.clinical_note",
    "notes": "std.clinical_note",
    "clinical_note": "std.clinical_note",
    "blood_pressure": "std.blood_pressure",
    "lab_result": "std.lab_result",
    "observation": "std.observation",
    "respiratory_rate": "std.respiratory_rate",
    "spo2": "std.spo2",
    "weight": "std.weight",
}


def resolve_event_type(name: str) -> str:
    """Resolve an alias or fully-qualified ``namespace.type`` name."""
    if "." in name:
        return name
    if name in EVENT_TYPE_ALIASES:
        return EVENT_TYPE_ALIASES[name]
    raise ValueError(
        f"unknown event-type {name!r}. Pass a fully-qualified `<ns>.<name>` or "
        f"one of: {', '.join(sorted(EVENT_TYPE_ALIASES))}."
    )


# ---------------------------------------------------------------------------
# Tables
# ---------------------------------------------------------------------------

def render_table(headers: Sequence[str], rows: Iterable[Sequence[object]]) -> str:
    """Render rows as a fixed-width table; columns auto-size to their content."""
    # Materialise rows so we can compute widths.
    materialised = [tuple(str(c) if c is not None else "" for c in r) for r in rows]
    widths = [len(h) for h in headers]
    for r in materialised:
        for i, c in enumerate(r):
            if i < len(widths):
                widths[i] = max(widths[i], len(c))
    fmt = "  ".join(f"{{:<{w}}}" for w in widths)
    out: list[str] = [fmt.format(*headers)]
    for r in materialised:
        # Pad rows that are shorter than headers.
        padded = list(r) + [""] * (len(headers) - len(r))
        out.append(fmt.format(*padded[: len(headers)]))
    return "\n".join(out)


def render_channel_value(cv: object) -> str:
    """Render a `pb.ChannelValue` as ``path=value``.

    Defensive about field presence — the ``oneof`` may be unset.
    """
    path = getattr(cv, "channel_path", "?")
    one_of = cv.WhichOneof("value") if hasattr(cv, "WhichOneof") else None  # type: ignore[attr-defined]
    if one_of is None:
        return f"{path}=<unset>"
    val = getattr(cv, one_of)
    return f"{path}={val}"


def join_channel_values(channels: Iterable[object]) -> str:
    return ", ".join(render_channel_value(c) for c in channels)


def truncate(s: str, width: int) -> str:
    """Truncate `s` to `width` chars, adding an ellipsis if shortened."""
    if len(s) <= width:
        return s
    if width <= 1:
        return s[:width]
    return s[: width - 1] + "…"


def short_ulid(b: bytes) -> str:
    """Render the first ten Crockford-base32 chars of a ULID."""
    try:
        return ulid_to_crockford(b)[:10]
    except (ValueError, TypeError):
        return "?"


__all__ = [
    "TimeRange",
    "build_range",
    "crockford_to_ulid",
    "now_ms",
    "parse_iso",
    "render_ms",
    "render_table",
    "render_channel_value",
    "join_channel_values",
    "resolve_event_type",
    "short_ulid",
    "truncate",
    "ulid_to_crockford",
    "EVENT_TYPE_ALIASES",
]


def render_iter_to_iter(
    items: Iterable[object],
) -> Iterator[object]:
    """Tiny indirection used by tests to iterate without materialising."""
    yield from items
