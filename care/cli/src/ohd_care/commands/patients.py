"""Patient-roster commands: ``add-patient`` / ``patients`` / ``use`` / ``current``.

The CLI's vault is the source of truth for "which patients does this
operator hold grants for". Each entry is one TOML file under
``~/.config/ohd-care/grants/<label>.toml`` — see ``grant_vault.py``.

Active-patient pointer is stored separately so it survives across
multiple shells / invocations without copying the grant file.
"""

from __future__ import annotations

import click

from ..credentials import get_active_label, set_active_label
from ..grant_vault import (
    GrantConflictError,
    GrantVault,
    PatientGrant,
    UnknownPatientError,
)
from ..util import now_ms, render_ms, render_table


@click.command("add-patient", help="Add a patient grant to the local vault.")
@click.option("--label", required=True, help="Operator-typed label, e.g. 'Alice (DOB 1985-04-12)'.")
@click.option("--token", "grant_token", required=True, help="Grant token (`ohdg_…`).")
@click.option(
    "--storage-url",
    default=None,
    help="Per-patient storage URL (rendezvous URL or direct). "
    "Falls back to the global URL from `ohd-care login`.",
)
@click.option(
    "--cert-pin-sha256",
    default=None,
    help="Hex-encoded SHA-256 of the expected TLS cert (relay-mediated only).",
)
@click.option(
    "--scope-summary",
    default=None,
    help="Human-readable scope notes shown in `ohd-care patients`.",
)
@click.option("--notes", default=None, help="Operator's freeform notes about this patient.")
@click.option("--force", is_flag=True, help="Overwrite an existing entry for this label.")
def add_patient(
    label: str,
    grant_token: str,
    storage_url: str | None,
    cert_pin_sha256: str | None,
    scope_summary: str | None,
    notes: str | None,
    force: bool,
) -> None:
    grant = PatientGrant(
        label=label,
        grant_token=grant_token,
        storage_url=storage_url,
        storage_cert_pin_sha256_hex=cert_pin_sha256,
        scope_summary=scope_summary,
        notes=notes,
        imported_at_ms=now_ms(),
    )
    vault = GrantVault()
    try:
        path = vault.save(grant, force=force)
    except GrantConflictError as exc:
        raise click.ClickException(str(exc)) from exc
    click.echo(f"saved grant for {label!r} to {path}")
    if get_active_label() is None:
        # Convenience: first patient added becomes the active one.
        set_active_label(label)
        click.echo(f"set active patient = {label!r}")


@click.command("patients", help="List patients in this operator's grant vault.")
def patients() -> None:
    vault = GrantVault()
    grants = vault.list_grants()
    if not grants:
        click.echo("(no patients in the vault — run `ohd-care add-patient` to add one)")
        return
    active = get_active_label()
    headers = ["ACTIVE", "LABEL", "GRANT_ULID", "EXPIRES", "SCOPE"]
    rows: list[tuple[str, str, str, str, str]] = []
    for g in grants:
        marker = "*" if g.label == active else ""
        expires = render_ms(g.expires_at_ms) if g.expires_at_ms else "—"
        rows.append(
            (
                marker,
                g.label,
                g.grant_ulid or "—",
                expires,
                g.scope_summary or "—",
            )
        )
    click.echo(render_table(headers, rows))


@click.command("use", help="Set the active patient by label.")
@click.argument("label")
def use(label: str) -> None:
    vault = GrantVault()
    try:
        vault.load(label)
    except UnknownPatientError as exc:
        raise click.ClickException(str(exc)) from exc
    set_active_label(label)
    click.echo(f"active patient = {label!r}")


@click.command("current", help="Show the active patient + scope.")
def current() -> None:
    label = get_active_label()
    if label is None:
        click.echo("(no active patient — run `ohd-care use <label>`)", err=True)
        raise SystemExit(1)
    vault = GrantVault()
    try:
        grant = vault.load(label)
    except UnknownPatientError as exc:
        raise click.ClickException(str(exc)) from exc
    click.echo(f"active patient: {grant.label}")
    click.echo(f"  storage_url:  {grant.storage_url or '(default)'}")
    click.echo(f"  grant_ulid:   {grant.grant_ulid or '—'}")
    if grant.expires_at_ms:
        click.echo(f"  expires_at:   {render_ms(grant.expires_at_ms)}")
    if grant.scope_summary:
        click.echo(f"  scope:        {grant.scope_summary}")
    if grant.case_ulids:
        click.echo(f"  cases:        {', '.join(grant.case_ulids)}")


@click.command("remove-patient", help="Drop a patient grant from the local vault.")
@click.argument("label")
def remove_patient(label: str) -> None:
    vault = GrantVault()
    if not vault.remove(label):
        raise click.ClickException(f"no grant for label {label!r}")
    if get_active_label() == label:
        set_active_label(None)
        click.echo(f"removed {label!r} (cleared active patient)")
    else:
        click.echo(f"removed {label!r}")
