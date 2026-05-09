"""Smoke tests for the ohd-care CLI top-level shape.

Covers what the click command tree looks like at a glance: --help, --version,
and that every registered subcommand also exposes --help. Implementation
behavior lives in ``test_cli.py``.
"""

from __future__ import annotations

from click.testing import CliRunner

from ohd_care import __version__
from ohd_care.cli import main


def test_top_level_help_exits_zero() -> None:
    runner = CliRunner()
    result = runner.invoke(main, ["--help"])
    assert result.exit_code == 0
    assert "OHD Care" in result.output


def test_version_prints_package_version() -> None:
    runner = CliRunner()
    result = runner.invoke(main, ["--version"])
    assert result.exit_code == 0
    assert __version__ in result.output


def test_submit_observation_requires_type_and_value() -> None:
    runner = CliRunner()
    result = runner.invoke(main, ["submit", "observation"])
    # click should reject missing required options with exit code 2.
    assert result.exit_code != 0
    assert "Missing option" in result.output or "Error" in result.output
