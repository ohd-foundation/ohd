"""Tests for the OHD Emergency MCP server.

Two test classes:

1. **Unit tests** (default) — drive the FastMCP server with a mock OHDC
   client (``MockOhdcClient``) so we can assert on tool transforms (e.g.
   "draft_handoff_summary calls query_events with the active case's
   grant token, then folds the timeline into a draft") without spinning
   up storage.

2. **Integration tests** (``@pytest.mark.integration``) — spin up a real
   ``ohd-storage-server`` on a temp dir, issue a self-session token via
   the storage's CLI, use it to drive the Emergency MCP tools end-to-end.
   Skipped by default (``-m "not integration"``); the storage-binary
   lookup is best-effort and the whole class skips when the binary is
   missing.
"""

from __future__ import annotations

import os
import shutil
import socket
import subprocess
import time
from pathlib import Path
from typing import Any

import pytest
from fastmcp import Client

from ohd_emergency_mcp.case_vault import CaseVault
from ohd_emergency_mcp.config import CaseGrant, EmergencyMcpConfig, OidcProxyConfig
from ohd_emergency_mcp.ohdc_client import OhdcClient, OhdcClientConfig
from ohd_emergency_mcp.server import build_server

EXPECTED_TOOLS = {
    # Case selection (analogous to Care's switch_patient — required for
    # the multi-case routing model in emergency/SPEC.md §3.1)
    "list_active_cases",
    "set_active_case",
    # Triage tools (§3.2)
    "find_relevant_context_for_complaint",
    "summarize_vitals",
    "flag_abnormal_vitals",
    "check_administered_drug",
    "draft_handoff_summary",
}


# ---------------------------------------------------------------------------
# Mock OHDC client — records calls; lets unit tests assert tool transforms.
# ---------------------------------------------------------------------------


class MockOhdcClient:
    """In-memory OHDC client stand-in.

    Each method records ``(method_name, kwargs)`` to ``self.calls`` and
    returns a canned response. Tests can either inspect ``calls`` to
    verify the tool routed correctly, or assert on the returned
    structured content.
    """

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.put_events_response: dict[str, Any] = {
            "results": [
                {"outcome": "pending", "ulid": "01HMOCK" + "0" * 19, "expires_at_ms": 0}
            ]
        }
        self.query_events_response: list[dict[str, Any]] = []
        self.who_am_i_response: dict[str, Any] = {
            "user_ulid": "01HMOCKUSER" + "0" * 15,
            "token_kind": "grant",
            "caller_ip": "127.0.0.1",
            "grantee_label": "test",
        }

    async def aclose(self) -> None:
        pass

    async def who_am_i(self, *, grant_token: str) -> dict[str, Any]:
        self.calls.append(("who_am_i", {"grant_token": grant_token}))
        return self.who_am_i_response

    async def query_events(self, **kwargs: Any) -> list[dict[str, Any]]:
        self.calls.append(("query_events", kwargs))
        return self.query_events_response

    async def aggregate(self, **kwargs: Any) -> list[dict[str, Any]]:
        self.calls.append(("aggregate", kwargs))
        from ohd_emergency_mcp.ohdc_client import OhdcNotWiredError

        raise OhdcNotWiredError("aggregate")

    async def put_events(self, **kwargs: Any) -> dict[str, Any]:
        self.calls.append(("put_events", kwargs))
        return self.put_events_response

    async def find_relevant_context(self, **kwargs: Any) -> dict[str, Any]:
        self.calls.append(("find_relevant_context", kwargs))
        from ohd_emergency_mcp.ohdc_client import OhdcNotWiredError

        raise OhdcNotWiredError("find_relevant_context")

    async def check_drug_interaction(self, **kwargs: Any) -> dict[str, Any]:
        self.calls.append(("check_drug_interaction", kwargs))
        from ohd_emergency_mcp.ohdc_client import OhdcNotWiredError

        raise OhdcNotWiredError("check_drug_interaction")


def _seeded_config() -> EmergencyMcpConfig:
    return EmergencyMcpConfig(
        storage_url="http://127.0.0.1:18443",
        operator_token="dummy",
        allow_external_llm=False,
        transport="stdio",
        http_host="127.0.0.1",
        http_port=8767,
        cases=[
            CaseGrant(
                case_id="01HEMERG_DEMO_A",
                grant_token="ohdg_em_a",
                label="MVA scene 14:32",
            ),
            CaseGrant(
                case_id="01HEMERG_DEMO_B",
                grant_token="ohdg_em_b",
                label="OD response 16:08",
            ),
        ],
    )


@pytest.fixture
def mock_client() -> MockOhdcClient:
    return MockOhdcClient()


@pytest.fixture
def mcp_server(mock_client: MockOhdcClient):
    cfg = _seeded_config()
    return build_server(
        config=cfg,
        client=mock_client,  # type: ignore[arg-type]
        vault=CaseVault.from_list(cfg.cases),
    )


# ---------------------------------------------------------------------------
# Unit tests
# ---------------------------------------------------------------------------


# ---------------------------------------------------------------------------
# OIDC config wiring (mirror of connect/mcp test shape)
# ---------------------------------------------------------------------------


def test_oidc_proxy_disabled_when_unconfigured() -> None:
    """Default config — no env vars set — must not enable the OAuthProxy."""
    cfg = OidcProxyConfig()
    assert not cfg.enabled


def test_oidc_proxy_enabled_when_all_three_set() -> None:
    """All of (issuer, client_id, base_url) required to flip `enabled` on."""
    cfg = OidcProxyConfig(
        issuer_url="https://idp.example",
        client_id="ohd-emergency-mcp",
        base_url="https://mcp.example",
    )
    assert cfg.enabled


def test_oidc_proxy_disabled_with_partial_config() -> None:
    """Missing base_url leaves the proxy off — caller fell back to stdio."""
    cfg = OidcProxyConfig(issuer_url="https://idp.example", client_id="x")
    assert not cfg.enabled


def test_emergency_mcp_config_reads_oidc_env(monkeypatch) -> None:
    """``OHD_EMERGENCY_OIDC_*`` env vars must populate :class:`OidcProxyConfig`."""
    monkeypatch.setenv("OHD_EMERGENCY_OIDC_ISSUER", "https://idp.example")
    monkeypatch.setenv("OHD_EMERGENCY_OIDC_CLIENT_ID", "ohd-emergency-mcp")
    monkeypatch.setenv("OHD_EMERGENCY_OIDC_CLIENT_SECRET", "shh")
    monkeypatch.setenv("OHD_EMERGENCY_MCP_BASE_URL", "https://mcp.example")
    cfg = EmergencyMcpConfig.from_env()
    assert cfg.oidc.enabled
    assert cfg.oidc.issuer_url == "https://idp.example"
    assert cfg.oidc.client_id == "ohd-emergency-mcp"
    assert cfg.oidc.client_secret == "shh"
    assert cfg.oidc.base_url == "https://mcp.example"


def test_build_server_with_oidc_disabled_returns_no_auth() -> None:
    """When ``cfg.oidc.enabled`` is false, ``auth`` on the FastMCP must be None.

    Mirrors the connect/mcp shape: stdio installs always run with auth=None
    even if the OIDC env vars happen to be partly set.
    """
    cfg = _seeded_config()
    # Default: oidc empty + stdio transport → no proxy.
    server = build_server(
        config=cfg,
        client=MockOhdcClient(),  # type: ignore[arg-type]
        vault=CaseVault.from_list(cfg.cases),
    )
    # FastMCP exposes `auth` as a public attribute when set; absent
    # otherwise. Either path means "no proxy installed".
    assert getattr(server, "auth", None) is None


@pytest.mark.anyio
async def test_all_expected_tools_registered(mcp_server) -> None:
    async with Client(mcp_server) as c:
        names = {t.name for t in await c.list_tools()}
    missing = EXPECTED_TOOLS - names
    assert not missing, f"Emergency MCP is missing tools: {sorted(missing)}"


@pytest.mark.anyio
async def test_no_generic_query_events_exposed(mcp_server) -> None:
    """Per SPEC §3.2, generic CRUD MUST NOT be exposed to the LLM."""
    forbidden = {"query_events", "put_events", "log_symptom", "log_food"}
    async with Client(mcp_server) as c:
        names = {t.name for t in await c.list_tools()}
    leaked = names & forbidden
    assert not leaked, f"Emergency MCP leaks forbidden tools: {sorted(leaked)}"


@pytest.mark.anyio
async def test_tool_count_matches_spec(mcp_server) -> None:
    async with Client(mcp_server) as c:
        tools = await c.list_tools()
    assert len(tools) == len(EXPECTED_TOOLS)


@pytest.mark.anyio
async def test_case_vault_state_machine(mcp_server) -> None:
    async with Client(mcp_server) as c:
        listed = await c.call_tool("list_active_cases", {})
        data = listed.structured_content
        ids = [c["case_id"] for c in data["cases"]]
        assert "01HEMERG_DEMO_A" in ids and "01HEMERG_DEMO_B" in ids
        assert data["active_case_id"] is None

        switched = await c.call_tool(
            "set_active_case", {"case_id": "01HEMERG_DEMO_A"}
        )
        assert switched.structured_content["active_case_id"] == "01HEMERG_DEMO_A"


@pytest.mark.anyio
async def test_triage_tool_requires_active_case(mcp_server) -> None:
    async with Client(mcp_server) as c:
        with pytest.raises(Exception) as excinfo:
            await c.call_tool("summarize_vitals", {"window": "last_1h"})
    msg = str(excinfo.value)
    assert "active case" in msg.lower() or "set_active_case" in msg


@pytest.mark.anyio
async def test_unknown_case_id_rejected(mcp_server) -> None:
    async with Client(mcp_server) as c:
        with pytest.raises(Exception) as excinfo:
            await c.call_tool("set_active_case", {"case_id": "01NOPE"})
    msg = str(excinfo.value)
    assert "01NOPE" in msg or "No grant" in msg


@pytest.mark.anyio
async def test_flag_abnormal_vitals_routes_active_grant_token(
    mcp_server, mock_client: MockOhdcClient
) -> None:
    """flag_abnormal_vitals → query_events with the active case's token."""
    mock_client.query_events_response = [
        {
            "ulid": "01HMOCK" + "0" * 19,
            "event_type": "vitals",
            "timestamp_ms": 1,
            "channels": [],
        }
    ]
    async with Client(mcp_server) as c:
        await c.call_tool("set_active_case", {"case_id": "01HEMERG_DEMO_A"})
        result = await c.call_tool("flag_abnormal_vitals", {})
    assert result.structured_content["active_case_id"] == "01HEMERG_DEMO_A"
    matching = [(m, kw) for (m, kw) in mock_client.calls if m == "query_events"]
    assert len(matching) == 1
    _, kw = matching[0]
    assert kw["grant_token"] == "ohdg_em_a"
    assert kw["event_type"] == "vitals"


@pytest.mark.anyio
async def test_draft_handoff_summary_routes_active_grant_token(
    mcp_server, mock_client: MockOhdcClient
) -> None:
    """draft_handoff_summary calls query_events for the timeline, scoped to the case."""
    mock_client.query_events_response = [
        {"ulid": "01HMOCK" + "0" * 19, "event_type": "vitals", "timestamp_ms": 1, "channels": []}
    ]
    async with Client(mcp_server) as c:
        await c.call_tool("set_active_case", {"case_id": "01HEMERG_DEMO_B"})
        result = await c.call_tool("draft_handoff_summary", {})
    draft = result.structured_content["draft"]
    assert draft["case_id"] == "01HEMERG_DEMO_B"
    assert draft["case_label"] == "OD response 16:08"
    assert isinstance(draft["timeline"], list)
    matching = [(m, kw) for (m, kw) in mock_client.calls if m == "query_events"]
    assert len(matching) == 1
    _, kw = matching[0]
    assert kw["grant_token"] == "ohdg_em_b"


@pytest.mark.anyio
async def test_check_administered_drug_surfaces_not_wired(
    mcp_server, mock_client: MockOhdcClient
) -> None:
    """check_drug_interaction is OhdcNotWiredError; the tool surfaces it."""
    async with Client(mcp_server) as c:
        await c.call_tool("set_active_case", {"case_id": "01HEMERG_DEMO_A"})
        with pytest.raises(Exception) as excinfo:
            await c.call_tool(
                "check_administered_drug",
                {"drug_name": "naloxone", "dose": "0.4mg IM"},
            )
    msg = str(excinfo.value)
    assert "not yet wired" in msg or "drug-interaction" in msg


# ---------------------------------------------------------------------------
# Integration tests — real ohd-storage-server. Skipped by default.
# ---------------------------------------------------------------------------


def _storage_binary() -> Path | None:
    """Locate ohd-storage-server. None if missing (skip integration tests)."""
    here = Path(__file__).resolve()
    repo_root = here.parents[3]  # tests/test_tools.py -> mcp -> emergency -> ohd
    candidates = [
        repo_root / "storage" / "target" / "release" / "ohd-storage-server",
        repo_root / "storage" / "target" / "debug" / "ohd-storage-server",
    ]
    for c in candidates:
        if c.exists() and os.access(c, os.X_OK):
            return c
    on_path = shutil.which("ohd-storage-server")
    return Path(on_path) if on_path else None


def _free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def _wait_ready(port: int, *, timeout: float = 30.0) -> None:
    """Poll until the storage server's listen port accepts connections."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return
        except OSError:
            time.sleep(0.2)
    raise TimeoutError(f"storage didn't open port {port} within {timeout}s")


@pytest.fixture(scope="module")
def storage_server():
    """Spin up an ohd-storage-server in a temp dir; yield (url, self_token).

    See ``care/mcp/tests/test_tools.py`` for the CLI-flow rationale.
    """
    binary = _storage_binary()
    if binary is None:
        pytest.skip("ohd-storage-server binary not found; build with `cargo build` in storage/")

    import tempfile

    tmp = tempfile.mkdtemp(prefix="ohd-emergency-mcp-test-")
    db_path = Path(tmp) / "data.db"

    init = subprocess.run(
        [str(binary), "init", "--db", str(db_path)],
        capture_output=True,
        text=True,
        timeout=20,
    )
    if init.returncode != 0:
        shutil.rmtree(tmp, ignore_errors=True)
        pytest.skip(
            f"ohd-storage-server init failed: stdout={init.stdout!r} "
            f"stderr={init.stderr!r}"
        )

    iss = subprocess.run(
        [str(binary), "issue-self-token", "--db", str(db_path), "--label", "emergency-mcp-test"],
        capture_output=True,
        text=True,
        timeout=20,
    )
    if iss.returncode != 0:
        shutil.rmtree(tmp, ignore_errors=True)
        pytest.skip(
            f"ohd-storage-server issue-self-token failed: stdout={iss.stdout!r} "
            f"stderr={iss.stderr!r}"
        )
    token = iss.stdout.strip().splitlines()[-1].strip()

    port = _free_port()
    url = f"http://127.0.0.1:{port}"
    proc = subprocess.Popen(
        [
            str(binary),
            "serve",
            "--db",
            str(db_path),
            "--listen",
            f"127.0.0.1:{port}",
        ],
        env={**os.environ, "RUST_LOG": "warn"},
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:
        _wait_ready(port, timeout=20.0)
    except Exception:
        proc.terminate()
        try:
            out, err = proc.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            out, err = proc.communicate()
        shutil.rmtree(tmp, ignore_errors=True)
        pytest.skip(
            "ohd-storage-server serve didn't open port "
            f"(stdout={out[:300]!r} stderr={err[:300]!r})"
        )

    yield url, token

    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
    shutil.rmtree(tmp, ignore_errors=True)


@pytest.mark.integration
@pytest.mark.anyio
async def test_integration_who_am_i_with_self_token(storage_server) -> None:
    """Smoke: the real client can talk to a real storage with a real token."""
    url, token = storage_server
    client = OhdcClient(OhdcClientConfig(storage_url=url))
    try:
        result = await client.who_am_i(grant_token=token)
    finally:
        await client.aclose()
    assert "token_kind" in result


@pytest.mark.integration
@pytest.mark.anyio
async def test_integration_put_then_handoff_via_emergency_tools(storage_server) -> None:
    """End-to-end: set_active_case → put a vitals event directly via the client →
    draft_handoff_summary reads back the timeline."""
    url, token = storage_server

    cfg = EmergencyMcpConfig(
        storage_url=url,
        operator_token=None,
        allow_external_llm=False,
        transport="stdio",
        http_host="127.0.0.1",
        http_port=8767,
        cases=[
            CaseGrant(
                case_id="01HEMERG_INTEG_A",
                grant_token=token,
                label="integration test case",
            ),
        ],
    )
    client = OhdcClient(OhdcClientConfig(storage_url=url))
    server = build_server(
        config=cfg, client=client, vault=CaseVault.from_list(cfg.cases)
    )
    try:
        # Pre-seed: write one vitals event via the real client (the tool
        # surface intentionally doesn't expose put_events, so this is the
        # straight client path).
        put = await client.put_events(
            grant_token=token,
            events=[
                {
                    "event_type": "std.heart_rate_resting",
                    "timestamp_ms": int(time.time() * 1000),
                    "data": {"value": 88, "unit": "bpm"},
                    "metadata": {"source": "emergency_mcp_integration_test"},
                }
            ],
        )
        # The wire round-trip succeeded if we got a results list back.
        assert "results" in put and put["results"]

        async with Client(server) as c:
            await c.call_tool("set_active_case", {"case_id": "01HEMERG_INTEG_A"})
            handoff = await c.call_tool("draft_handoff_summary", {})
            draft = handoff.structured_content["draft"]
            assert draft["case_id"] == "01HEMERG_INTEG_A"
            assert isinstance(draft["timeline"], list)
    finally:
        await client.aclose()
