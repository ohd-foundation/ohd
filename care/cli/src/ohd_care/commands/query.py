"""Read commands: ``ohd-care query <kind>`` plus per-type convenience aliases.

The Care CLI reads through the active patient's grant token. Every read
respects the grant's read scope; rows the grant filters out are dropped
silently by storage (just as in the Rust CLI / web UI).

Usage::

    ohd-care query glucose --last-week
    ohd-care query std.body_temperature --from 2026-05-01 --to 2026-05-08
    ohd-care temperature --last-72h        # convenience alias
    ohd-care glucose --last-day            # convenience alias

Time selection is mutually exclusive: pick exactly one of ``--last-day``,
``--last-week``, ``--last-month``, ``--last-72h``, or the explicit
``--from`` / ``--to`` ISO pair.
"""

from __future__ import annotations

from typing import Any

import click

from ..credentials import get_active_label, load_credentials
from ..grant_vault import GrantVault, NoActivePatientError, UnknownPatientError
from ..ohdc_client import OhdcClient, OhdcError
from ..util import (
    EVENT_TYPE_ALIASES,
    build_range,
    join_channel_values,
    render_ms,
    render_table,
    resolve_event_type,
    short_ulid,
)


# Click decorators reused on every read subcommand.
def _time_options(f):  # type: ignore[no-untyped-def]
    decorators = [
        click.option(
            "--limit", "limit", type=int, default=100, show_default=True,
            help="Max rows to return.",
        ),
        click.option(
            "--last-day", "last_day", is_flag=True, default=False,
            help="Restrict to the last 24 hours.",
        ),
        click.option(
            "--last-week", "last_week", is_flag=True, default=False,
            help="Restrict to the last 7 days.",
        ),
        click.option(
            "--last-month", "last_month", is_flag=True, default=False,
            help="Restrict to the last 30 days.",
        ),
        click.option(
            "--last-72h", "last_72h", is_flag=True, default=False,
            help="Restrict to the last 72 hours.",
        ),
        click.option(
            "--from", "from_iso", default=None,
            help="Inclusive start ISO8601 timestamp.",
        ),
        click.option(
            "--to", "to_iso", default=None,
            help="Inclusive end ISO8601 timestamp.",
        ),
    ]
    for dec in decorators:
        f = dec(f)
    return f


def _resolve_active_grant() -> tuple[Any, str, str]:
    """Return ``(grant, storage_url, label)`` for the active patient.

    Raises :class:`click.ClickException` on any of: no credentials, no
    active patient, missing grant file. The CLI never silently falls back
    to the wrong patient — refusing is safer than guessing.
    """
    try:
        settings = load_credentials()
    except Exception as exc:
        raise click.ClickException(str(exc)) from exc
    label = get_active_label()
    if label is None:
        raise click.ClickException(
            "no active patient — run `ohd-care use <label>` "
            "(see `ohd-care patients`)."
        )
    try:
        grant = GrantVault().load(label)
    except UnknownPatientError as exc:
        raise click.ClickException(str(exc)) from exc
    storage_url = grant.storage_url or settings.storage_url
    return grant, storage_url, label


def _do_query(
    *,
    event_type: str,
    last_day: bool,
    last_week: bool,
    last_month: bool,
    last_72h: bool,
    from_iso: str | None,
    to_iso: str | None,
    limit: int,
) -> None:
    """Shared body for `query` and the convenience subcommands."""
    try:
        canonical = resolve_event_type(event_type)
    except ValueError as exc:
        raise click.ClickException(str(exc)) from exc
    try:
        time_range = build_range(
            last_day=last_day,
            last_week=last_week,
            last_month=last_month,
            last_72h=last_72h,
            from_iso=from_iso,
            to_iso=to_iso,
        )
    except ValueError as exc:
        raise click.ClickException(str(exc)) from exc

    grant, storage_url, label = _resolve_active_grant()

    # Lazy import so `--help` doesn't pay the protobuf-codegen cost.
    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    flt_kwargs: dict[str, Any] = {
        "event_types_in": [canonical],
        "include_superseded": True,
    }
    if time_range.from_ms is not None:
        flt_kwargs["from_ms"] = time_range.from_ms
    if time_range.to_ms is not None:
        flt_kwargs["to_ms"] = time_range.to_ms
    if limit > 0:
        flt_kwargs["limit"] = limit
    flt = pb.EventFilter(**flt_kwargs)
    req = pb.QueryEventsRequest(filter=flt)

    rows: list[tuple[str, str, str, str]] = []
    count = 0
    try:
        with OhdcClient(storage_url=storage_url) as client, client.with_token(grant.grant_token):
            for event in client.query_events(req):
                count += 1
                rows.append(
                    (
                        short_ulid(event.ulid.bytes) if event.HasField("ulid") else "—",
                        render_ms(event.timestamp_ms),
                        event.event_type,
                        join_channel_values(event.channels) or "—",
                    )
                )
    except OhdcError as exc:
        raise click.ClickException(str(exc)) from exc

    click.echo(f"# active patient: {label}  •  type: {canonical}", err=True)
    if not rows:
        click.echo("(no events matched)")
        return
    click.echo(render_table(["ULID", "TIMESTAMP (UTC)", "TYPE", "CHANNELS"], rows))
    click.echo(f"({count} event{'s' if count != 1 else ''})", err=True)


# ---------------------------------------------------------------------------
# `ohd-care query <kind>` — generic.
# ---------------------------------------------------------------------------

@click.command("query", help="Query events for the active patient.")
@click.argument("event_type", metavar="EVENT_TYPE")
@_time_options
def query(
    event_type: str,
    last_day: bool,
    last_week: bool,
    last_month: bool,
    last_72h: bool,
    from_iso: str | None,
    to_iso: str | None,
    limit: int,
) -> None:
    """Query events. ``EVENT_TYPE`` is either a fully-qualified
    ``<namespace>.<name>`` or one of the recognized short forms
    (``glucose``, ``temperature``, ``heart-rate``, ``medications``,
    ``symptoms``, ``notes``).
    """
    _do_query(
        event_type=event_type,
        last_day=last_day,
        last_week=last_week,
        last_month=last_month,
        last_72h=last_72h,
        from_iso=from_iso,
        to_iso=to_iso,
        limit=limit,
    )


# ---------------------------------------------------------------------------
# Convenience commands per SPEC.md §11.
# ---------------------------------------------------------------------------

def _alias_command(name: str, alias: str, help_text: str):  # type: ignore[no-untyped-def]
    """Build a thin click command that pins `event_type` to `alias`."""

    @click.command(name, help=help_text)
    @_time_options
    def _cmd(
        last_day: bool,
        last_week: bool,
        last_month: bool,
        last_72h: bool,
        from_iso: str | None,
        to_iso: str | None,
        limit: int,
    ) -> None:
        _do_query(
            event_type=alias,
            last_day=last_day,
            last_week=last_week,
            last_month=last_month,
            last_72h=last_72h,
            from_iso=from_iso,
            to_iso=to_iso,
            limit=limit,
        )

    return _cmd


temperature = _alias_command(
    "temperature", "temperature",
    "Read body-temperature events for the active patient.",
)
glucose = _alias_command(
    "glucose", "glucose",
    "Read blood-glucose events for the active patient.",
)
heart_rate = _alias_command(
    "heart-rate", "heart-rate",
    "Read resting heart-rate events for the active patient.",
)
medications = _alias_command(
    "medications", "medications",
    "Read medication-dose events for the active patient.",
)
symptoms = _alias_command(
    "symptoms", "symptoms",
    "Read symptom events for the active patient.",
)
notes = _alias_command(
    "notes", "notes",
    "Read clinical-note events for the active patient.",
)


__all__ = [
    "query",
    "temperature",
    "glucose",
    "heart_rate",
    "medications",
    "symptoms",
    "notes",
    "EVENT_TYPE_ALIASES",
]


# Avoid unused-import warning while still re-exporting the alias dict.
_ = NoActivePatientError
