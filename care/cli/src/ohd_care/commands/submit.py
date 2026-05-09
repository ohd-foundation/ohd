"""Write-with-approval submissions: ``ohd-care submit <kind> ...``.

Every submission is a typed event written via ``OhdcService.PutEvents``
against the active patient's grant. The grant's ``approval_mode`` (set on
the patient side when the grant was issued) decides whether the event
queues for the patient's review or auto-commits — this CLI doesn't pick
that policy, only renders the result.

Two safety rules per SPEC.md §3.3 / §6.3:

1. Each submission is preceded by a confirmation step rendering the
   active patient's label as the operator typed it.
2. The submission attempt is reported back with the grant's outcome
   (``committed`` / ``pending`` / ``error``) so the operator knows what
   happened.

``--yes`` skips the prompt for scripts; the active patient is still echoed
so the script's stdout / stderr makes the trail clear.
"""

from __future__ import annotations

import sys
from typing import Any

import click

from ..credentials import get_active_label, load_credentials
from ..grant_vault import GrantVault, UnknownPatientError
from ..ohdc_client import OhdcClient, OhdcError
from ..util import now_ms, short_ulid

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _resolve_active_grant() -> tuple[Any, str, str]:
    """``(grant, storage_url, label)`` — same shape as in `query.py`."""
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
    return grant, grant.storage_url or settings.storage_url, label


def _confirm_or_abort(label: str, summary: str, yes: bool) -> None:
    """Render the SPEC §6.3 confirmation prompt and bail unless confirmed."""
    click.echo(f"Submitting to {label}: {summary}")
    if yes:
        click.echo("  --yes given; skipping interactive confirm.", err=True)
        return
    if not click.confirm("Confirm submission?", default=False):
        raise click.ClickException("aborted by operator")


def _send_event(label: str, storage_url: str, grant_token: str, event_input: object) -> int:
    """Send one event, render the per-event outcome, return exit code."""
    # Lazy proto import (so `--help` is fast).
    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    req = pb.PutEventsRequest(events=[event_input], atomic=False)  # type: ignore[arg-type]
    try:
        with OhdcClient(storage_url=storage_url) as client, client.with_token(grant_token):
            resp = client.put_events(req)
    except OhdcError as exc:
        raise click.ClickException(str(exc)) from exc

    if not resp.results:
        click.echo("warning: PutEvents returned no results", err=True)
        return 1

    rc = 0
    for r in resp.results:
        kind = r.WhichOneof("outcome")
        match kind:
            case "committed":
                click.echo(
                    f"  committed   {short_ulid(r.committed.ulid.bytes)}"
                    f"   at {r.committed.committed_at_ms} ms"
                )
            case "pending":
                click.echo(
                    f"  pending     {short_ulid(r.pending.ulid.bytes)}"
                    f"   queued for {label}'s approval (expires_at_ms={r.pending.expires_at_ms})"
                )
            case "error":
                click.echo(
                    f"  error       {r.error.code}: {r.error.message}",
                    err=True,
                )
                rc = 1
            case _:
                click.echo("  error       <empty outcome>", err=True)
                rc = 1
    return rc


def _channel_real(path: str, value: float) -> object:
    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    return pb.ChannelValue(channel_path=path, real_value=value)


def _channel_text(path: str, value: str) -> object:
    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    return pb.ChannelValue(channel_path=path, text_value=value)


# ---------------------------------------------------------------------------
# Top-level group + subcommands
# ---------------------------------------------------------------------------

@click.group("submit", help="Write-with-approval submissions for the active patient.")
def submit() -> None:
    """One submission per command. The grant's ``approval_mode`` decides
    whether the result is queued or committed; the CLI surfaces both
    outcomes verbatim. See SPEC.md §6.
    """


@submit.command("observation", help="Submit a generic observation event.")
@click.option(
    "--type", "obs_type", required=True,
    help="Observation type — fully qualified `<ns>.<name>` (e.g. 'std.observation').",
)
@click.option("--value", "value", required=True, type=float, help="Numeric value.")
@click.option("--unit", "unit", default=None, help="Unit; recorded in `notes` for now.")
@click.option(
    "--at-ms",
    "at_ms",
    type=int,
    default=None,
    help="Override timestamp in epoch ms (default: now).",
)
@click.option("--yes", "yes", is_flag=True, default=False, help="Skip the confirmation prompt.")
def submit_observation(
    obs_type: str, value: float, unit: str | None, at_ms: int | None, yes: bool,
) -> None:
    grant, storage_url, label = _resolve_active_grant()
    summary = f"observation type={obs_type} value={value}" + (f" unit={unit}" if unit else "")
    _confirm_or_abort(label, summary, yes)

    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    channels = [_channel_real("value", float(value))]
    notes = f"unit={unit}" if unit else None
    event = pb.EventInput(
        timestamp_ms=at_ms if at_ms is not None else now_ms(),
        event_type=obs_type,
        channels=channels,  # type: ignore[arg-type]
        notes=notes,
        source="ohd-care",
    )
    sys.exit(_send_event(label, storage_url, grant.grant_token, event))


@submit.command(
    "clinical-note",
    help="Submit a clinical note (reads body from stdin if --text not given).",
)
@click.option("--about", default=None, help="Free-text 'about' marker (e.g. 'visit 2026-05-07').")
@click.option(
    "--text",
    "note_text",
    default=None,
    help="Body of the note. If omitted, reads from stdin until EOF.",
)
@click.option("--type", "note_type", default="std.clinical_note", show_default=True,
              help="Event type to write under.")
@click.option("--at-ms", "at_ms", type=int, default=None, help="Override timestamp in epoch ms.")
@click.option("--yes", "yes", is_flag=True, default=False, help="Skip the confirmation prompt.")
def submit_clinical_note(
    about: str | None,
    note_text: str | None,
    note_type: str,
    at_ms: int | None,
    yes: bool,
) -> None:
    grant, storage_url, label = _resolve_active_grant()
    body = note_text if note_text is not None else sys.stdin.read()
    body = body.rstrip("\n")
    if not body.strip():
        raise click.ClickException("note body is empty (pipe text or pass --text)")

    summary_preview = body[:60].replace("\n", " ")
    if len(body) > 60:
        summary_preview += "…"
    summary = f"clinical-note about={about!r} body={summary_preview!r}"
    _confirm_or_abort(label, summary, yes)

    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    channels = [_channel_text("body", body)]
    if about:
        channels.append(_channel_text("about", about))
    event = pb.EventInput(
        timestamp_ms=at_ms if at_ms is not None else now_ms(),
        event_type=note_type,
        channels=channels,  # type: ignore[arg-type]
        notes=about,
        source="ohd-care",
    )
    sys.exit(_send_event(label, storage_url, grant.grant_token, event))


@submit.command("lab-result", help="Submit a lab result event.")
@click.option(
    "--type", "lab_type", required=True,
    help="Lab type — fully qualified `<ns>.<name>` (e.g. 'std.lab_result').",
)
@click.option("--value", "value", required=True, type=float, help="Numeric result value.")
@click.option("--unit", "unit", default=None, help="Result unit (recorded in `notes`).")
@click.option("--reference-range", "reference_range", default=None, help="Reference range string.")
@click.option("--at-ms", "at_ms", type=int, default=None, help="Override timestamp in epoch ms.")
@click.option("--yes", "yes", is_flag=True, default=False, help="Skip the confirmation prompt.")
def submit_lab_result(
    lab_type: str,
    value: float,
    unit: str | None,
    reference_range: str | None,
    at_ms: int | None,
    yes: bool,
) -> None:
    grant, storage_url, label = _resolve_active_grant()
    summary = f"lab-result type={lab_type} value={value}"
    if unit:
        summary += f" unit={unit}"
    if reference_range:
        summary += f" ref={reference_range!r}"
    _confirm_or_abort(label, summary, yes)

    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    channels = [_channel_real("value", float(value))]
    if reference_range:
        channels.append(_channel_text("reference_range", reference_range))
    notes_parts = []
    if unit:
        notes_parts.append(f"unit={unit}")
    notes = "; ".join(notes_parts) or None
    event = pb.EventInput(
        timestamp_ms=at_ms if at_ms is not None else now_ms(),
        event_type=lab_type,
        channels=channels,  # type: ignore[arg-type]
        notes=notes,
        source="ohd-care",
    )
    sys.exit(_send_event(label, storage_url, grant.grant_token, event))


@submit.command("measurement", help="Submit a generic numeric measurement.")
@click.option(
    "--type", "meas_type", required=True,
    help="Measurement type — fully qualified `<ns>.<name>` (e.g. 'std.body_temperature').",
)
@click.option("--value", "value", required=True, type=float, help="Numeric value.")
@click.option("--unit", "unit", default=None, help="Unit (recorded in `notes`).")
@click.option("--at-ms", "at_ms", type=int, default=None, help="Override timestamp in epoch ms.")
@click.option("--yes", "yes", is_flag=True, default=False, help="Skip the confirmation prompt.")
def submit_measurement(
    meas_type: str, value: float, unit: str | None, at_ms: int | None, yes: bool,
) -> None:
    grant, storage_url, label = _resolve_active_grant()
    summary = f"measurement type={meas_type} value={value}" + (f" unit={unit}" if unit else "")
    _confirm_or_abort(label, summary, yes)

    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    channels = [_channel_real("value", float(value))]
    notes = f"unit={unit}" if unit else None
    event = pb.EventInput(
        timestamp_ms=at_ms if at_ms is not None else now_ms(),
        event_type=meas_type,
        channels=channels,  # type: ignore[arg-type]
        notes=notes,
        source="ohd-care",
    )
    sys.exit(_send_event(label, storage_url, grant.grant_token, event))


@submit.command("prescription", help="Submit a prescription event.")
@click.option("--drug", "drug_name", required=True, help="Drug name.")
@click.option("--dose", "dose", required=True, type=float, help="Dose amount.")
@click.option(
    "--dose-unit", "dose_unit", required=True,
    help="Dose unit (mg, mcg, g, ml, units, tablets, puffs, drops).",
)
@click.option("--type", "rx_type", default="std.prescription", show_default=True,
              help="Event type to write under (defaults to `std.prescription`; "
                   "the std registry may not include this type — deployments may "
                   "register a custom one).")
@click.option("--at-ms", "at_ms", type=int, default=None, help="Override timestamp in epoch ms.")
@click.option("--yes", "yes", is_flag=True, default=False, help="Skip the confirmation prompt.")
def submit_prescription(
    drug_name: str,
    dose: float,
    dose_unit: str,
    rx_type: str,
    at_ms: int | None,
    yes: bool,
) -> None:
    grant, storage_url, label = _resolve_active_grant()
    summary = f"prescription drug={drug_name!r} dose={dose} {dose_unit}"
    _confirm_or_abort(label, summary, yes)

    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    channels = [
        _channel_text("name", drug_name),
        _channel_real("dose", float(dose)),
        _channel_text("dose_unit", dose_unit),
    ]
    event = pb.EventInput(
        timestamp_ms=at_ms if at_ms is not None else now_ms(),
        event_type=rx_type,
        channels=channels,  # type: ignore[arg-type]
        source="ohd-care",
    )
    sys.exit(_send_event(label, storage_url, grant.grant_token, event))


__all__ = [
    "submit",
    "submit_observation",
    "submit_clinical_note",
    "submit_lab_result",
    "submit_measurement",
    "submit_prescription",
]
