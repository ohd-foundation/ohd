"""Smoke tests for the OHD Connect MCP server.

These tests do NOT depend on a real OHDC client. They construct the
FastMCP server with the stubbed ``OhdcClient`` and verify:

1. Every tool the spec promises is registered.
2. Calling a tool with valid input either returns a real result (for tools
   that don't hit OHDC, none in this set) or surfaces a structured
   ``OhdcNotWiredError`` payload — never a silent crash.
"""

from __future__ import annotations

from typing import Any

import pytest
from fastmcp import Client

from ohd_connect_mcp.server import build_server


class _MockOhdcClient:
    """Minimal OHDC client stub for unit tests.

    Records each method call's name + kwargs in ``self.calls`` and returns
    a canned response. Tests inject this via ``build_server(client=...)``
    so tool routing can be asserted without spinning up storage.
    """

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []
        self.put_events_response: dict[str, Any] = {
            "results": [{"outcome": "committed", "ulid": "01HMOCK" + "0" * 19}]
        }
        self.query_events_response: list[dict[str, Any]] = []
        self.who_am_i_response: dict[str, Any] = {
            "user_ulid": "01HMOCKUSER" + "0" * 15,
            "token_kind": "self_session",
        }

    async def aclose(self) -> None:
        pass

    async def put_events(self, *args: Any, **kwargs: Any) -> dict[str, Any]:
        self.calls.append(("put_events", {"args": args, **kwargs}))
        return self.put_events_response

    async def query_events(self, *args: Any, **kwargs: Any) -> list[dict[str, Any]]:
        self.calls.append(("query_events", {"args": args, **kwargs}))
        return self.query_events_response

    async def who_am_i(self, *args: Any, **kwargs: Any) -> dict[str, Any]:
        self.calls.append(("who_am_i", {"args": args, **kwargs}))
        return self.who_am_i_response

    def __getattr__(self, name: str) -> Any:
        async def _stub(*args: Any, **kwargs: Any) -> Any:
            self.calls.append((name, {"args": args, **kwargs}))
            return {}
        return _stub

EXPECTED_TOOLS = {
    # Logging
    "log_symptom",
    "log_food",
    "log_medication",
    "log_measurement",
    "log_exercise",
    "log_mood",
    "log_sleep",
    "log_free_event",
    # Reading
    "query_events",
    "query_latest",
    "summarize",
    "correlate",
    "find_patterns",
    "get_medications_taken",
    "get_food_log",
    "chart",
    # Grants
    "create_grant",
    "list_grants",
    "revoke_grant",
    # Pending review
    "list_pending",
    "approve_pending",
    "reject_pending",
    # Cases
    "list_cases",
    "get_case",
    "force_close_case",
    "issue_retrospective_grant",
    # Audit
    "audit_query",
}


@pytest.fixture
def mcp_server():
    return build_server()


@pytest.mark.anyio
async def test_all_expected_tools_registered(mcp_server) -> None:
    async with Client(mcp_server) as c:
        tools = await c.list_tools()
        names = {t.name for t in tools}
    missing = EXPECTED_TOOLS - names
    assert not missing, f"Connect MCP is missing tools: {sorted(missing)}"


@pytest.mark.anyio
async def test_tool_count_matches_spec(mcp_server) -> None:
    """27 tools per connect/SPEC.md 'Connect MCP — tool list'."""
    async with Client(mcp_server) as c:
        tools = await c.list_tools()
    assert len(tools) == len(EXPECTED_TOOLS)


@pytest.mark.anyio
async def test_log_symptom_routes_to_put_events() -> None:
    """log_symptom should call OhdcClient.put_events with a typed event."""
    mock = _MockOhdcClient()
    server = build_server(client=mock)
    async with Client(server) as c:
        await c.call_tool(
            "log_symptom",
            {"symptom": "headache", "severity": "moderate"},
        )
    assert any(name == "put_events" for name, _ in mock.calls), (
        f"log_symptom did not call put_events; calls were: {mock.calls}"
    )


@pytest.mark.anyio
async def test_query_latest_routes_to_query_events() -> None:
    """query_latest should call OhdcClient.query_events with a bounded filter."""
    mock = _MockOhdcClient()
    server = build_server(client=mock)
    async with Client(server) as c:
        await c.call_tool("query_latest", {"event_type": "glucose", "count": 5})
    assert any(name == "query_events" for name, _ in mock.calls), (
        f"query_latest did not call query_events; calls were: {mock.calls}"
    )


@pytest.mark.anyio
async def test_invalid_input_is_rejected(mcp_server) -> None:
    """Pydantic validation should reject invalid input before reaching OHDC."""
    async with Client(mcp_server) as c:
        with pytest.raises(Exception):
            # `count` must be in [1, 100]; 0 is invalid.
            await c.call_tool("query_latest", {"event_type": "glucose", "count": 0})
