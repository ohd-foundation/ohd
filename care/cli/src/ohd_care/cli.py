"""OHD Care CLI entry point.

Implements the operator-side command surface from `care/SPEC.md` §11:

    ohd-care login --storage URL
    ohd-care add-patient --label … --token …
    ohd-care patients
    ohd-care use <label>
    ohd-care current
    ohd-care remove-patient <label>

    ohd-care query <event-type> [time options]
    ohd-care temperature [time options]
    ohd-care glucose [time options]
    ohd-care heart-rate [time options]
    ohd-care medications [time options]
    ohd-care symptoms [time options]
    ohd-care notes [time options]

    ohd-care submit observation --type … --value …
    ohd-care submit clinical-note [--text … | <stdin>]
    ohd-care submit lab-result --type … --value …
    ohd-care submit measurement --type … --value …
    ohd-care submit prescription --drug … --dose …

    ohd-care pending list [--status …]
    ohd-care pending show <pending-ulid>

    ohd-care audit [time options]

Each subcommand lives in ``ohd_care.commands.*``; this module just builds
the click command tree.
"""

from __future__ import annotations

import click

from . import __version__
from .commands import audit as _audit_cmd
from .commands import login as _login_cmd
from .commands import patients as _patients_cmd
from .commands import pending as _pending_cmd
from .commands import query as _query_cmd
from .commands import submit as _submit_cmd


@click.group(help="OHD Care — terminal interface for the OHD reference clinical app.")
@click.version_option(__version__, prog_name="ohd-care")
def main() -> None:
    """Top-level command group.

    Sub-commands operate on the operator session held in
    ``$XDG_CONFIG_HOME/ohd-care`` (or ``~/.config/ohd-care``). The active
    patient (the grant in scope for read/write operations) is set via
    ``ohd-care use <label>`` per ``SPEC.md`` §3.3 — explicit operator
    action only; never inferred from arguments.
    """


# --- session / roster -----------------------------------------------------

main.add_command(_login_cmd.login)
main.add_command(_login_cmd.oidc_login)
main.add_command(_login_cmd.logout)
main.add_command(_patients_cmd.add_patient)
main.add_command(_patients_cmd.patients)
main.add_command(_patients_cmd.use)
main.add_command(_patients_cmd.current)
main.add_command(_patients_cmd.remove_patient)

# --- reads ---------------------------------------------------------------

main.add_command(_query_cmd.query)
main.add_command(_query_cmd.temperature)
main.add_command(_query_cmd.glucose)
main.add_command(_query_cmd.heart_rate)
main.add_command(_query_cmd.medications)
main.add_command(_query_cmd.symptoms)
main.add_command(_query_cmd.notes)

# --- writes (group with subcommands) -------------------------------------

main.add_command(_submit_cmd.submit)

# --- pending queue (group with subcommands) ------------------------------

main.add_command(_pending_cmd.pending)

# --- audit ---------------------------------------------------------------

main.add_command(_audit_cmd.audit)


if __name__ == "__main__":  # pragma: no cover
    main()
