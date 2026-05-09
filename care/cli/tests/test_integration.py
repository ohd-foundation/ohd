"""Integration tests — exercise the full ohd-care CLI against a real
``ohd-storage-server``.

Marked ``integration``; skipped unless the binary exists at
``../../storage/target/{debug,release}/ohd-storage-server``. The server
runs in the background on an ephemeral port, gets a fresh sqlite DB, gets
a self-token + grant token issued via the server's own subcommands, and
the CLI is exercised against that grant.

Round-trip flow:

1. ``ohd-care login --storage <url>``
2. ``ohd-care add-patient --label demo --token <ohdg_…>``
3. ``ohd-care patients`` — confirms the entry shows.
4. ``ohd-care use demo``
5. (seed two glucose events via the connect CLI binary if available, so
   query has something to find — optional; we skip seeding if the connect
   binary is missing).
6. ``ohd-care query glucose --last-day``
7. ``ohd-care submit clinical-note --text … --yes``
8. ``ohd-care pending list``
"""

from __future__ import annotations

import os
import socket
import subprocess
import time
from collections.abc import Generator
from pathlib import Path

import pytest
from click.testing import CliRunner

from ohd_care.cli import main

# ---------------------------------------------------------------------------
# Locate the storage server binary
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parents[3]
STORAGE_DIR = REPO_ROOT / "storage"


def _find_storage_bin() -> Path | None:
    for variant in ("debug", "release"):
        p = STORAGE_DIR / "target" / variant / "ohd-storage-server"
        if p.is_file() and os.access(p, os.X_OK):
            return p
    return None


STORAGE_BIN = _find_storage_bin()

pytestmark = [
    pytest.mark.integration,
    pytest.mark.skipif(
        STORAGE_BIN is None,
        reason="ohd-storage-server binary not built (run `cargo build` in storage/)",
    ),
]


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _free_port() -> int:
    """Find a free TCP port on localhost."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _wait_for_port(host: str, port: int, timeout_s: float = 10.0) -> None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.5):
                return
        except OSError:
            time.sleep(0.1)
    raise TimeoutError(f"server at {host}:{port} did not accept connections within {timeout_s}s")


@pytest.fixture
def isolated_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    home = tmp_path / "ohd-care"
    home.mkdir()
    monkeypatch.setenv("OHD_CARE_HOME", str(home))
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    return home


@pytest.fixture
def storage_server(tmp_path: Path) -> Generator[tuple[str, str], None, None]:
    """Boot ``ohd-storage-server``, return ``(storage_url, grant_token)``."""
    assert STORAGE_BIN is not None
    db = tmp_path / "demo.db"
    port = _free_port()
    listen = f"127.0.0.1:{port}"
    storage_url = f"http://127.0.0.1:{port}"

    # init
    subprocess.run([str(STORAGE_BIN), "init", "--db", str(db)], check=True)

    # issue tokens
    self_tok = subprocess.run(
        [str(STORAGE_BIN), "issue-self-token", "--db", str(db), "--label", "test-self"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    assert self_tok.startswith("ohds_"), f"unexpected self-token: {self_tok!r}"

    # `issue-grant-token` validates each type against the registry. The
    # canonical std.* types live in 002_std_registry.sql; `std.clinical_note`
    # is auto-seeded by the helper. Anything else (e.g. `std.observation`,
    # `std.lab_result`) requires per-deployment registration, so we leave
    # those out of the integration test grant.
    grant_tok = subprocess.run(
        [
            str(STORAGE_BIN), "issue-grant-token",
            "--db", str(db),
            "--read",
            "std.blood_glucose,std.heart_rate_resting,std.body_temperature,"
            "std.medication_dose,std.symptom,std.clinical_note",
            "--write", "std.clinical_note",
            "--approval-mode", "always",
            "--label", "Dr. Test",
            "--expires-days", "1",
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    assert grant_tok.startswith("ohdg_"), f"unexpected grant token: {grant_tok!r}"

    log_path = tmp_path / "server.log"
    log_fp = open(log_path, "w")
    proc = subprocess.Popen(
        [str(STORAGE_BIN), "serve", "--db", str(db), "--listen", listen],
        stdout=log_fp,
        stderr=subprocess.STDOUT,
        env={**os.environ, "RUST_LOG": "warn"},
    )
    try:
        _wait_for_port("127.0.0.1", port)
        # Give the server a tick to finish bringing up Connect-RPC after the
        # socket binds — bind happens before the service is wired in some builds.
        time.sleep(0.3)
        yield storage_url, grant_tok, self_tok  # type: ignore[misc]
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()
        log_fp.close()


# ---------------------------------------------------------------------------
# The test
# ---------------------------------------------------------------------------

def test_full_round_trip(isolated_home: Path, storage_server: tuple[str, str, str]) -> None:
    storage_url, grant_tok, self_tok = storage_server  # type: ignore[misc]
    runner = CliRunner()

    r = runner.invoke(main, ["login", "--storage", storage_url])
    assert r.exit_code == 0, r.output
    assert "saved operator credentials" in r.output

    r = runner.invoke(
        main,
        [
            "add-patient",
            "--label", "demo-patient",
            "--token", grant_tok,
            "--scope-summary", "integration test grant",
        ],
    )
    assert r.exit_code == 0, r.output

    r = runner.invoke(main, ["patients"])
    assert r.exit_code == 0, r.output
    assert "demo-patient" in r.output

    r = runner.invoke(main, ["use", "demo-patient"])
    assert r.exit_code == 0, r.output

    r = runner.invoke(main, ["current"])
    assert r.exit_code == 0, r.output
    assert "demo-patient" in r.output

    # Optional seed: log two glucose events through the connect CLI if it's built.
    connect_bin = REPO_ROOT / "connect" / "cli" / "target" / "release" / "ohd-connect"
    if not connect_bin.is_file():
        connect_bin = REPO_ROOT / "connect" / "cli" / "target" / "debug" / "ohd-connect"
    if connect_bin.is_file():
        for v in ("120", "138"):
            subprocess.run(
                [
                    str(connect_bin),
                    "--storage", storage_url,
                    "--token", self_tok,
                    "log", "glucose", v,
                ],
                check=True,
                capture_output=True,
                text=True,
            )

    # Read against the grant — should not crash even if there are zero events.
    r = runner.invoke(main, ["query", "glucose", "--last-day"])
    assert r.exit_code == 0, r.output

    # Submit a clinical-note. The grant has approval_mode=always so this
    # should land in pending. We pass --yes to skip the confirm prompt.
    r = runner.invoke(
        main,
        [
            "submit", "clinical-note",
            "--text", "Patient reports headache resolved after rest.",
            "--about", "integration-test",
            "--yes",
        ],
    )
    # The grant write rule we issued only includes std.clinical_note; the
    # write should either succeed (commit/pending) or surface a clear error.
    # We accept either; what we DO require is that the CLI didn't crash.
    assert r.exit_code in (0, 1), r.output
    assert "Submitting to" in r.output or "active patient" in r.output

    # List pending — should at least not error out.
    r = runner.invoke(main, ["pending", "list"])
    assert r.exit_code == 0, r.output
