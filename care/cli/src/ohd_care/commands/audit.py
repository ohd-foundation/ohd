"""``ohd-care audit`` — server-side audit log query.

Calls ``OhdcService.AuditQuery`` with the active grant. Today the storage
implementation returns ``unimplemented`` for grant tokens — the CLI
catches this and exits with a clear "TBD" message rather than a Connect
error stack. See ``STATUS.md``.
"""

from __future__ import annotations

from typing import Any

import click

from ..credentials import get_active_label, load_credentials
from ..grant_vault import GrantVault, UnknownPatientError
from ..ohdc_client import OhdcClient, OhdcError, OhdcUnimplementedError
from ..util import build_range, render_ms, render_table, short_ulid


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


@click.command("audit", help="Query the patient-side audit log for the active grant.")
@click.option(
    "--last-day", "last_day", is_flag=True, default=False,
    help="Restrict to the last 24 hours.",
)
@click.option(
    "--last-week", "last_week", is_flag=True, default=False,
    help="Restrict to the last 7 days.",
)
@click.option(
    "--last-month", "last_month", is_flag=True, default=False,
    help="Restrict to the last 30 days.",
)
@click.option(
    "--last-72h", "last_72h", is_flag=True, default=False,
    help="Restrict to the last 72 hours.",
)
@click.option("--from", "from_iso", default=None, help="Inclusive start ISO8601 timestamp.")
@click.option("--to", "to_iso", default=None, help="Inclusive end ISO8601 timestamp.")
@click.option(
    "--action", "action", default=None,
    help="Filter by action ('read' | 'write' | 'grant_create' | …).",
)
@click.option(
    "--result", "result_filter", default=None,
    help="Filter by result ('success' | 'partial' | 'rejected' | 'error').",
)
def audit(
    last_day: bool,
    last_week: bool,
    last_month: bool,
    last_72h: bool,
    from_iso: str | None,
    to_iso: str | None,
    action: str | None,
    result_filter: str | None,
) -> None:
    grant, storage_url, label = _resolve_active_grant()
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

    from ohdc.v0 import ohdc_pb2 as pb  # type: ignore[import-not-found]

    req_kwargs: dict[str, Any] = {}
    if time_range.from_ms is not None:
        req_kwargs["from_ms"] = time_range.from_ms
    if time_range.to_ms is not None:
        req_kwargs["to_ms"] = time_range.to_ms
    if action:
        req_kwargs["action"] = action
    if result_filter:
        req_kwargs["result"] = result_filter
    req = pb.AuditQueryRequest(**req_kwargs)

    rows: list[tuple[str, str, str, str, str, str]] = []
    try:
        with OhdcClient(storage_url=storage_url) as client, client.with_token(grant.grant_token):
            for entry in client.audit_query(req):
                rows.append(
                    (
                        render_ms(entry.ts_ms),
                        entry.actor_type or "—",
                        short_ulid(entry.grant_ulid.bytes) if entry.HasField("grant_ulid") else "—",
                        entry.action or "—",
                        entry.result or "—",
                        entry.query_kind or "—",
                    )
                )
    except OhdcUnimplementedError as exc:
        click.echo(
            "ohd-care: AuditQuery is not yet wired in storage — "
            "see care/cli/STATUS.md ('Open items: AuditQuery RPC').",
            err=True,
        )
        click.echo(f"  raw error: {exc}", err=True)
        raise SystemExit(2) from exc
    except OhdcError as exc:
        raise click.ClickException(str(exc)) from exc

    click.echo(f"# active patient: {label}", err=True)
    if not rows:
        click.echo("(no audit entries matched)")
        return
    click.echo(render_table(["TS", "ACTOR", "GRANT", "ACTION", "RESULT", "KIND"], rows))


__all__ = ["audit"]
