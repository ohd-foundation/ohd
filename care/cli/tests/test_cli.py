"""CLI behavior tests — no real OHDC server required.

We test what the CLI does without a storage server reachable: every
subcommand exposes ``--help``, the "no active patient" path returns a
clean error, and ``add-patient`` + ``patients`` + ``use`` round-trip
through the on-disk grant vault.
"""

from __future__ import annotations

import os
from pathlib import Path

import pytest
from click.testing import CliRunner

from ohd_care.cli import main


@pytest.fixture
def isolated_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Point ``OHD_CARE_HOME`` at a tmp dir so tests don't touch the real config."""
    home = tmp_path / "ohd-care"
    home.mkdir()
    monkeypatch.setenv("OHD_CARE_HOME", str(home))
    # Also clear XDG so the override is the only source.
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    return home


# ---------------------------------------------------------------------------
# --help on every command (regression net)
# ---------------------------------------------------------------------------

@pytest.mark.parametrize(
    "argv",
    [
        ["--help"],
        ["login", "--help"],
        ["add-patient", "--help"],
        ["patients", "--help"],
        ["use", "--help"],
        ["current", "--help"],
        ["remove-patient", "--help"],
        ["query", "--help"],
        ["temperature", "--help"],
        ["glucose", "--help"],
        ["heart-rate", "--help"],
        ["medications", "--help"],
        ["symptoms", "--help"],
        ["notes", "--help"],
        ["submit", "--help"],
        ["submit", "observation", "--help"],
        ["submit", "clinical-note", "--help"],
        ["submit", "lab-result", "--help"],
        ["submit", "measurement", "--help"],
        ["submit", "prescription", "--help"],
        ["pending", "--help"],
        ["pending", "list", "--help"],
        ["pending", "show", "--help"],
        ["audit", "--help"],
    ],
)
def test_help_exits_zero(argv: list[str]) -> None:
    runner = CliRunner()
    result = runner.invoke(main, argv)
    assert result.exit_code == 0, result.output


# ---------------------------------------------------------------------------
# "no active patient" path — every read/write should fail clean.
# ---------------------------------------------------------------------------

def test_query_without_active_patient_clean_error(isolated_home: Path) -> None:
    runner = CliRunner()
    # `login` first so credentials.toml exists; otherwise we'd fail on that.
    r1 = runner.invoke(main, ["login", "--storage", "http://example.invalid"])
    assert r1.exit_code == 0, r1.output
    r2 = runner.invoke(main, ["query", "glucose", "--last-day"])
    assert r2.exit_code != 0
    assert "no active patient" in r2.output


def test_submit_without_active_patient_clean_error(isolated_home: Path) -> None:
    runner = CliRunner()
    r1 = runner.invoke(main, ["login", "--storage", "http://example.invalid"])
    assert r1.exit_code == 0, r1.output
    r2 = runner.invoke(
        main,
        ["submit", "observation", "--type", "std.observation", "--value", "1.0", "--yes"],
    )
    assert r2.exit_code != 0
    assert "no active patient" in r2.output


def test_pending_list_without_active_patient_clean_error(isolated_home: Path) -> None:
    runner = CliRunner()
    r1 = runner.invoke(main, ["login", "--storage", "http://example.invalid"])
    assert r1.exit_code == 0, r1.output
    r2 = runner.invoke(main, ["pending", "list"])
    assert r2.exit_code != 0
    assert "no active patient" in r2.output


def test_current_without_active_patient_clean_error(isolated_home: Path) -> None:
    runner = CliRunner()
    r = runner.invoke(main, ["current"])
    assert r.exit_code != 0
    assert "no active patient" in (r.output + (r.stderr if hasattr(r, "stderr") else ""))


# ---------------------------------------------------------------------------
# Roster round-trip — add-patient, patients, use, current.
# ---------------------------------------------------------------------------

def test_roster_round_trip(isolated_home: Path) -> None:
    runner = CliRunner()

    r = runner.invoke(main, ["login", "--storage", "http://example.invalid"])
    assert r.exit_code == 0, r.output

    r = runner.invoke(
        main,
        [
            "add-patient",
            "--label", "Alice (DOB 1985-04-12)",
            "--token", "ohdg_TESTTOKEN",
            "--scope-summary", "Demo grant",
        ],
    )
    assert r.exit_code == 0, r.output
    assert "Alice" in r.output

    r = runner.invoke(main, ["patients"])
    assert r.exit_code == 0, r.output
    assert "Alice" in r.output
    # First patient added becomes active by convention; marker '*'.
    assert "*" in r.output

    r = runner.invoke(main, ["current"])
    assert r.exit_code == 0, r.output
    assert "Alice" in r.output

    # Remove, then verify it's gone.
    r = runner.invoke(main, ["remove-patient", "Alice (DOB 1985-04-12)"])
    assert r.exit_code == 0, r.output
    r = runner.invoke(main, ["patients"])
    assert r.exit_code == 0
    assert "no patients" in r.output


def test_use_unknown_label_clean_error(isolated_home: Path) -> None:
    runner = CliRunner()
    r = runner.invoke(main, ["use", "ghost-patient"])
    assert r.exit_code != 0
    assert "ghost-patient" in r.output


def test_query_invalid_alias_clean_error(isolated_home: Path) -> None:
    runner = CliRunner()
    runner.invoke(main, ["login", "--storage", "http://example.invalid"])
    runner.invoke(
        main,
        [
            "add-patient",
            "--label", "Bob",
            "--token", "ohdg_TEST",
        ],
    )
    r = runner.invoke(main, ["query", "definitely-not-a-real-alias", "--last-day"])
    assert r.exit_code != 0
    assert "unknown event-type" in r.output


def test_query_mutually_exclusive_time_flags(isolated_home: Path) -> None:
    runner = CliRunner()
    runner.invoke(main, ["login", "--storage", "http://example.invalid"])
    runner.invoke(main, ["add-patient", "--label", "Bob", "--token", "ohdg_TEST"])
    r = runner.invoke(main, ["query", "glucose", "--last-day", "--last-week"])
    assert r.exit_code != 0


# ---------------------------------------------------------------------------
# Confirmation step (SPEC §6.3) — uses --yes to skip the interactive prompt.
# ---------------------------------------------------------------------------

def test_submit_clinical_note_aborts_without_yes_or_input(
    isolated_home: Path,
) -> None:
    runner = CliRunner()
    runner.invoke(main, ["login", "--storage", "http://example.invalid"])
    runner.invoke(main, ["add-patient", "--label", "Carol", "--token", "ohdg_TEST"])
    # Without `--yes`, click.confirm() reads from stdin; CliRunner provides
    # empty input which interpreters as "abort" — and the request is never sent.
    r = runner.invoke(
        main,
        ["submit", "clinical-note", "--text", "lorem ipsum"],
        input="N\n",
    )
    assert r.exit_code != 0
    assert "Submitting to" in r.output
    assert "Carol" in r.output


def test_grants_dir_is_mode_0700(isolated_home: Path) -> None:
    runner = CliRunner()
    runner.invoke(main, ["login", "--storage", "http://example.invalid"])
    runner.invoke(main, ["add-patient", "--label", "Dora", "--token", "ohdg_TEST"])
    grants = isolated_home / "grants"
    assert grants.is_dir()
    mode = os.stat(grants).st_mode & 0o777
    assert mode == 0o700, f"expected 0700, got {oct(mode)}"
    grant_file = grants / "Dora.toml"
    assert grant_file.is_file()
    fmode = os.stat(grant_file).st_mode & 0o777
    assert fmode == 0o600, f"expected 0600, got {oct(fmode)}"
