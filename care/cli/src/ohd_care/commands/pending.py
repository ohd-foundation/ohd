"""Pending-queue commands: ``ohd-care pending list/show``.

Submissions made under ``approval_mode=always`` (or queued types under
``auto_for_event_types``) live in the patient's pending queue until the
patient approves them via OHD Connect. The operator can inspect the
queue, but **only** entries submitted under their own grant token. The
patient's storage scopes ``ListPending`` to ``submitting_grant_ulid =
caller's grant`` for grant tokens.
"""

from __future__ import annotations

from typing import Any

import click

from ..credentials import get_active_label, load_credentials
from ..grant_vault import GrantVault, UnknownPatientError
from ..ohdc_client import OhdcClient, OhdcError
from ..util import (
    crockford_to_ulid,
    join_channel_values,
    render_ms,
    render_table,
    short_ulid,
    ulid_to_crockford,
)


def _resolve_active_grant() -> tuple[Any, str, str]:
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


@click.group("pending", help="Inspect the patient-side pending queue for the active patient.")
def pending() -> None:
    """Submissions made under the operator's grant that are awaiting
    patient review (or auto-committed under the grant's policy). The
    patient sees the same queue from their side in OHD Connect.
    """


@pending.command("list", help="List pending submissions for the active patient.")
@click.option(
    "--status", "status",
    type=click.Choice(["pending", "approved", "rejected", "expired"]),
    default=None,
    help="Filter by review status (default: all statuses).",
)
@click.option("--limit", "limit", type=int, default=50, show_default=True, help="Max rows.")
def pending_list(status: str | None, limit: int) -> None:
    grant, storage_url, label = _resolve_active_grant()
    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    page_kwargs: dict[str, Any] = {}
    if limit > 0:
        page_kwargs["limit"] = limit
    page = pb.PageRequest(**page_kwargs) if page_kwargs else pb.PageRequest()
    req_kwargs: dict[str, Any] = {"page": page}
    if status:
        req_kwargs["status"] = status
    req = pb.ListPendingRequest(**req_kwargs)

    try:
        with OhdcClient(storage_url=storage_url) as client, client.with_token(grant.grant_token):
            resp = client.list_pending(req)
    except OhdcError as exc:
        raise click.ClickException(str(exc)) from exc

    click.echo(f"# active patient: {label}", err=True)
    if not resp.pending:
        click.echo("(no pending submissions)")
        return

    headers = ["PENDING_ULID", "STATUS", "TYPE", "SUBMITTED", "EXPIRES", "CHANNELS"]
    rows: list[tuple[str, str, str, str, str, str]] = []
    for p in resp.pending:
        rows.append(
            (
                short_ulid(p.ulid.bytes) if p.HasField("ulid") else "—",
                p.status,
                p.event.event_type if p.HasField("event") else "—",
                render_ms(p.submitted_at_ms),
                render_ms(p.expires_at_ms) if p.expires_at_ms else "—",
                (join_channel_values(p.event.channels) if p.HasField("event") else "—") or "—",
            )
        )
    click.echo(render_table(headers, rows))
    click.echo(f"({len(resp.pending)} submission{'s' if len(resp.pending) != 1 else ''})", err=True)


@pending.command("show", help="Show one pending submission's full content + audit metadata.")
@click.argument("pending_ulid", metavar="PENDING_ULID")
def pending_show(pending_ulid: str) -> None:
    grant, storage_url, label = _resolve_active_grant()
    # Validate the operator-supplied ULID early — clearer error than waiting
    # for storage to reject a bad shape.
    try:
        crockford_to_ulid(pending_ulid)
    except ValueError as exc:
        raise click.ClickException(f"invalid ULID: {exc}") from exc

    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    # No `GetPending` RPC; we list and locate. Storage scopes the listing
    # to the caller's grant, so this only finds the operator's own
    # submissions.
    req = pb.ListPendingRequest(page=pb.PageRequest(limit=1000))
    try:
        with OhdcClient(storage_url=storage_url) as client, client.with_token(grant.grant_token):
            resp = client.list_pending(req)
    except OhdcError as exc:
        raise click.ClickException(str(exc)) from exc

    target = pending_ulid.upper()
    found = None
    for p in resp.pending:
        if p.HasField("ulid") and ulid_to_crockford(p.ulid.bytes) == target:
            found = p
            break
    if found is None:
        raise click.ClickException(
            f"no pending submission matches {pending_ulid!r} for active patient {label!r}"
        )

    click.echo(f"# active patient: {label}")
    click.echo(f"pending_ulid:        {ulid_to_crockford(found.ulid.bytes)}")
    click.echo(f"status:              {found.status}")
    click.echo(f"submitted_at:        {render_ms(found.submitted_at_ms)}")
    if found.HasField("submitting_grant_ulid"):
        click.echo(f"submitting_grant:    {ulid_to_crockford(found.submitting_grant_ulid.bytes)}")
    expires_at_str = render_ms(found.expires_at_ms) if found.expires_at_ms else "—"
    click.echo(f"expires_at:          {expires_at_str}")
    if found.reviewed_at_ms:
        click.echo(f"reviewed_at:         {render_ms(found.reviewed_at_ms)}")
    if found.rejection_reason:
        click.echo(f"rejection_reason:    {found.rejection_reason}")
    if found.HasField("approved_event_ulid"):
        click.echo(
            f"approved_event_ulid: {ulid_to_crockford(found.approved_event_ulid.bytes)}"
        )

    if found.HasField("event"):
        e = found.event
        click.echo("")
        click.echo("event:")
        click.echo(f"  event_type:        {e.event_type}")
        click.echo(f"  timestamp_ms:      {e.timestamp_ms}  ({render_ms(e.timestamp_ms)})")
        if e.channels:
            click.echo("  channels:")
            for ch in e.channels:
                click.echo(f"    - {ch.channel_path} = {_channel_value_repr(ch)}")
        if e.source:
            click.echo(f"  source:            {e.source}")
        if e.notes:
            click.echo(f"  notes:             {e.notes}")


def _channel_value_repr(cv: object) -> str:
    """Render a `ChannelValue`'s active oneof in a small-table-friendly way."""
    one_of = cv.WhichOneof("value") if hasattr(cv, "WhichOneof") else None  # type: ignore[attr-defined]
    if one_of is None:
        return "<unset>"
    return f"{getattr(cv, one_of)!r} ({one_of})"


__all__ = ["pending", "pending_list", "pending_show"]
