# Research: MCP Servers (with FastMCP 2.x)

> How OHD exposes data to LLMs, for both entry (Connect-side) and retrieval (Care-side). We use the standalone **FastMCP** framework (`fastmcp` on PyPI, `jlowin/fastmcp` on GitHub) rather than the smaller `mcp.server.fastmcp` that ships in the official MCP SDK.

> **Status note:** This research doc was written when OHD Storage was sketched as a Python/FastAPI service. The contracted architecture is Rust + Connect-RPC. The `FastMCP.from_fastapi(app)` auto-generation pattern doesn't apply directly anymore — but the surrounding ideas (two MCP servers split by purpose, hand-written high-level tools, OAuth proxy for remote MCPs, transport choices) carry over. The concrete tool catalog (`log_symptom`, `summarize`, `correlate`, etc.) is still good as a v1 starting set. When MCP work resumes, expect a Rust-side or Python-shim implementation against the OHDC Connect-RPC client library.

## Why standalone FastMCP (not the SDK one)

There are two things called "FastMCP":

- **FastMCP 1.0** — the original, now folded into the official MCP Python SDK as `mcp.server.fastmcp`. Good for basic servers. No client, limited composition, no OpenAPI generation.
- **FastMCP 2.x / 3.x** — the standalone `fastmcp` package by jlowin (now maintained under PrefectHQ). Actively developed, ~1M downloads/day, powers the majority of MCP servers in the wild. The recommended path for modern projects.

We want the standalone version because it gives us:

1. **Auto-generation from FastAPI.** `FastMCP.from_fastapi(app)` converts a FastAPI app into an MCP server. Since OHD Storage is FastAPI, a huge chunk of the Care MCP is free.
2. **OAuth proxy.** Built-in handling for delegating auth to our OIDC providers — we don't have to reinvent bearer-token plumbing.
3. **Server composition.** We can mount sub-servers (e.g., a read sub-server and a write sub-server) under one endpoint, making deployment simpler.
4. **Tool transformation / search.** When we grow past ~15 tools, FastMCP 3's search transforms help LLMs find the right one without loading the whole catalog.
5. **Client library.** For testing and for OHD-internal calls from one service to another, a first-class MCP client is nice.
6. **Mature CLI.** `fastmcp dev`, `fastmcp install`, `fastmcp run` for development and deployment.
7. **Still works with Claude Desktop, Claude.ai's MCP support, OpenAI's MCP tools, etc.** — it implements the full MCP spec, just with more ergonomics on top.

## What MCP is (quick recap)

MCP (Model Context Protocol) is an open standard that connects LLMs to tools and data. MCP servers expose three primitives:

- **Tools** — functions the LLM can call (POST-like, with side effects).
- **Resources** — readable data URIs the LLM can load into context (GET-like).
- **Prompts** — reusable templates for LLM interactions.

MCP is transport-agnostic. Three transports that matter:

- **stdio** — server runs as a subprocess; client spawns it. For local installs (Claude Desktop, the user's laptop).
- **Streamable HTTP** — the modern HTTP transport. For remote servers. Single endpoint, bidirectional.
- **SSE** — older HTTP transport, being superseded.

For OHD we use **Streamable HTTP** for remote servers (SaaS Connect MCP, Care MCP served alongside OHD Storage) and **stdio** for locally installed tools.

## Two distinct MCP servers

OHD has two MCP surfaces with different purposes.

### 1. Connect MCP — data entry

**Purpose:** Let an LLM log health events on behalf of the user. "I have a headache" → event created.

**Who runs it:**

- User locally, installed into Claude Desktop via `fastmcp install`.
- Future: inside the Android app.
- The SaaS, for users who want to chat-log.

**Auth:** The server holds a *write-only* token to the user's OHD instance, configured at install time. The LLM never sees the token.

**Transport:** stdio (local) or Streamable HTTP (remote).

### 2. Care MCP — data retrieval and analysis

**Purpose:** Let an LLM query and analyze the user's health data. "What's my glucose trend this month?" → runs aggregation, returns result.

**Who runs it:**

- User locally against their own OHD.
- Doctor, configured with a grant token for a specific patient.
- Researcher, configured with a study-scoped grant.

**Auth:** Read token (own session) or grant token. Scope enforced server-side by OHD Storage.

**Transport:** stdio or Streamable HTTP.

## Care MCP: auto-generated from FastAPI

The Care MCP can be built in two complementary ways, both supported by FastMCP:

### Approach A — auto-generate from the OHD Storage FastAPI app

```python
# care_mcp/server.py
from fastmcp import FastMCP
from ohd_core.app import app as ohd_fastapi_app

# Auto-generate an MCP server from the FastAPI app.
# Every FastAPI endpoint becomes an MCP tool, with schemas derived from
# Pydantic models and summaries/descriptions from docstrings.
mcp = FastMCP.from_fastapi(
    ohd_fastapi_app,
    name="OHD Care",
    # Only expose read operations for Cord
    include_operations={"GET"},
    # Strip admin/internal routes
    exclude_routes=["/admin/*", "/internal/*"],
)

if __name__ == "__main__":
    mcp.run()  # stdio by default
```

**Pros:**

- Minimal new code. When the OHD API grows, the MCP surface grows with it.
- Single source of truth for the schema.
- Consistency between REST and MCP (doctors can use either).

**Cons:**

- Not every REST endpoint makes a good MCP tool. REST tends to be CRUD; good MCP tools tend to be "do the analysis I actually want."
- LLMs do better with higher-level tools like `summarize` than with `GET /events` — too many raw events is noisy.

### Approach B — hand-written high-level tools

```python
# care_mcp/server.py
from fastmcp import FastMCP
from ohd_core.client import OHDClient

mcp = FastMCP("OHD Care")
ohd = OHDClient.from_env()  # reads OHD_BASE_URL and OHD_TOKEN

@mcp.tool
async def summarize(
    event_type: str,
    period: str,
    aggregation: str = "avg",
    from_time: str | None = None,
    to_time: str | None = None,
) -> list[dict]:
    """Aggregate an event type over time.

    Args:
        event_type: Event type to summarize (e.g., "glucose", "heart_rate").
        period: "hourly", "daily", "weekly", "monthly".
        aggregation: "avg", "min", "max", "sum", "count", "median".
        from_time: ISO 8601 start. Defaults to 90 days ago.
        to_time: ISO 8601 end. Defaults to now.
    """
    return await ohd.summarize(event_type, period, aggregation, from_time, to_time)
```

**Pros:**

- Tailored to real LLM use cases.
- Can combine multiple REST calls into one tool ("give me meals and glucose response" is one tool, two REST calls).
- Cleaner docstrings → better LLM tool-use accuracy.

**Cons:**

- More code to maintain.
- Separate place to register new capabilities.

### Our choice: both, composed

FastMCP supports server composition: mount multiple sub-servers under one. We use that.

```python
# care_mcp/server.py
from fastmcp import FastMCP
from ohd_core.app import app as ohd_fastapi_app
from .high_level_tools import high_level_mcp  # hand-written

# Raw REST surface (auto-generated), mounted at /raw
raw_mcp = FastMCP.from_fastapi(
    ohd_fastapi_app,
    name="OHD Raw",
    include_operations={"GET"},
)

# Top-level server
mcp = FastMCP("OHD Care")
mcp.mount("raw", raw_mcp)
mcp.mount("analysis", high_level_mcp)

if __name__ == "__main__":
    mcp.run(transport="http", host="0.0.0.0", port=8001)
```

LLMs see both sets of tools namespaced (`raw.get_events`, `analysis.summarize`). For most tasks the hand-written tools are better; for odd edge cases the raw surface is available as a fallback.

## Connect MCP tool definitions

Connect MCP is hand-written. The tool surface is small and focused.

```python
# connect_mcp/server.py
import os
from typing import Annotated
import httpx
from pydantic import Field
from fastmcp import FastMCP

mcp = FastMCP("OHD Connect")

OHD_BASE_URL = os.environ["OHD_BASE_URL"]
OHD_WRITE_TOKEN = os.environ["OHD_WRITE_TOKEN"]

_client = httpx.AsyncClient(
    base_url=OHD_BASE_URL,
    headers={"Authorization": f"Bearer {OHD_WRITE_TOKEN}"},
    timeout=10.0,
)


async def _post_event(event: dict) -> dict:
    r = await _client.post("/events", json=event)
    r.raise_for_status()
    return r.json()


@mcp.tool
async def log_symptom(
    symptom: Annotated[str, Field(description="Symptom name, e.g. 'headache'.")],
    severity: Annotated[str | None, Field(description="Qualitative severity: 'mild', 'moderate', 'severe'.")] = None,
    severity_scale: Annotated[str | None, Field(description="Numeric severity with scale, e.g. '1-10:7'.")] = None,
    location: Annotated[str | None, Field(description="Anatomical location, e.g. 'frontal'.")] = None,
    notes: Annotated[str | None, Field(description="Free-text notes.")] = None,
    timestamp: Annotated[str | None, Field(description="ISO 8601; defaults to now.")] = None,
) -> dict:
    """Log a symptom the user is experiencing."""
    event = {
        "event_type": "symptom",
        "timestamp": timestamp or _now_iso(),
        "data": {
            "symptom": symptom,
            "severity": severity,
            "severity_scale": severity_scale,
            "location": location,
            "notes": notes,
        },
        "metadata": {"source": "connect_mcp"},
    }
    result = await _post_event(event)
    return {"event_id": result["id"], "status": "logged"}


@mcp.tool
async def log_medication(
    name: Annotated[str, Field(description="Medication name, e.g. 'metformin'.")],
    dose: Annotated[str | None, Field(description="Dose, e.g. '500mg', '2 tablets'.")] = None,
    status: Annotated[str, Field(description="One of 'taken', 'skipped', 'late', 'refused'.")] = "taken",
    timestamp: Annotated[str | None, Field(description="ISO 8601; defaults to now.")] = None,
    notes: Annotated[str | None, Field(description="Notes, e.g. 'with food'.")] = None,
) -> dict:
    """Log a medication dose (taken, skipped, or late)."""
    # ... similar shape
    ...


@mcp.tool
async def log_food(
    description: Annotated[str, Field(description="What was eaten.")],
    quantity: Annotated[str | None, Field(description="Amount, e.g. '120g', '1 cup'.")] = None,
    started: Annotated[str | None, Field(description="When eating started.")] = None,
    ended: Annotated[str | None, Field(description="When eating ended.")] = None,
    barcode: Annotated[str | None, Field(description="EAN-13/UPC barcode; enables OpenFoodFacts lookup.")] = None,
) -> dict:
    """Log a food or drink the user consumed."""
    ...


@mcp.tool
async def log_measurement(
    measurement_type: Annotated[str, Field(description="Standard type like 'body_temperature' or namespaced custom like 'urine.glucose'.")],
    value: Annotated[float, Field(description="Numeric value.")],
    unit: Annotated[str, Field(description="Unit, e.g. '°C', 'mmHg'.")],
    timestamp: Annotated[str | None, Field(description="ISO 8601; defaults to now.")] = None,
    notes: Annotated[str | None, Field(description="Notes.")] = None,
) -> dict:
    """Log a generic health measurement not covered by more specific tools."""
    ...


@mcp.tool
async def log_exercise(
    activity: Annotated[str, Field(description="Activity, e.g. 'running', 'cycling'.")],
    duration_minutes: Annotated[int | None, Field(description="Duration in minutes.")] = None,
    intensity: Annotated[str | None, Field(description="'low', 'moderate', 'high'.")] = None,
    started: Annotated[str | None, Field(description="When the session started.")] = None,
    notes: Annotated[str | None, Field(description="Notes.")] = None,
) -> dict:
    """Log an exercise session."""
    ...


@mcp.tool
async def log_mood(
    mood: Annotated[str, Field(description="Mood description.")],
    energy: Annotated[str | None, Field(description="Energy level.")] = None,
    notes: Annotated[str | None, Field(description="Notes.")] = None,
    timestamp: Annotated[str | None, Field(description="ISO 8601; defaults to now.")] = None,
) -> dict:
    """Log the user's current mood or emotional state."""
    ...


@mcp.tool
async def log_sleep(
    bedtime: Annotated[str, Field(description="When the user went to bed.")],
    wake_time: Annotated[str, Field(description="When the user woke up.")],
    quality: Annotated[str | None, Field(description="Subjective quality.")] = None,
    notes: Annotated[str | None, Field(description="Notes.")] = None,
) -> dict:
    """Log a sleep session."""
    ...


@mcp.tool
async def log_free_event(
    event_type: Annotated[str, Field(description="Event type. Use namespaced ids for custom events, e.g. 'com.user.dialysis'.")],
    data: Annotated[dict, Field(description="Event-type-specific data.")],
    timestamp: Annotated[str | None, Field(description="ISO 8601; defaults to now.")] = None,
    duration_seconds: Annotated[int | None, Field(description="Duration, if applicable.")] = None,
) -> dict:
    """Fallback for event types not covered by other tools."""
    ...


if __name__ == "__main__":
    mcp.run()  # stdio
```

## Care MCP: the hand-written high-level tools

These live alongside the auto-generated ones.

```python
# care_mcp/high_level_tools.py
from typing import Annotated
from pydantic import Field
from fastmcp import FastMCP
from ohd_core.client import OHDClient

high_level_mcp = FastMCP("OHD Care Analysis")
_ohd = OHDClient.from_env()


@high_level_mcp.tool
async def query_latest(
    event_type: Annotated[str, Field(description="Event type to fetch.")],
    count: Annotated[int, Field(description="How many recent events.", ge=1, le=100)] = 1,
) -> list[dict]:
    """Fetch the most recent N events of a given type."""
    return await _ohd.get_events(event_type=event_type, limit=count, order="desc")


@high_level_mcp.tool
async def summarize(
    event_type: Annotated[str, Field(description="Event type.")],
    period: Annotated[str, Field(description="'hourly', 'daily', 'weekly', 'monthly'.")],
    aggregation: Annotated[str, Field(description="'avg', 'min', 'max', 'sum', 'count', 'median'.")] = "avg",
    from_time: Annotated[str | None, Field(description="ISO 8601; defaults to 90 days ago.")] = None,
    to_time: Annotated[str | None, Field(description="ISO 8601; defaults to now.")] = None,
) -> list[dict]:
    """Aggregate events over time buckets."""
    return await _ohd.summarize(event_type, period, aggregation, from_time, to_time)


@high_level_mcp.tool
async def correlate(
    event_type_a: Annotated[str, Field(description="First event type (the trigger).")],
    event_type_b: Annotated[str, Field(description="Second event type (the response).")],
    window_minutes: Annotated[int, Field(description="Minutes after A to look for B.", ge=1, le=1440)] = 120,
    from_time: Annotated[str | None, Field(description="ISO 8601.")] = None,
    to_time: Annotated[str | None, Field(description="ISO 8601.")] = None,
) -> dict:
    """Find temporal relationships between two event types.

    For each event of type A, find events of type B within `window_minutes`
    afterwards. Returns per-pair data plus summary statistics.

    Example: correlate(event_type_a='meal', event_type_b='glucose', window_minutes=180)
    to see post-meal glucose response.
    """
    return await _ohd.correlate(event_type_a, event_type_b, window_minutes, from_time, to_time)


@high_level_mcp.tool
async def get_medications_taken(
    from_time: Annotated[str | None, Field(description="ISO 8601; defaults to 30 days ago.")] = None,
    to_time: Annotated[str | None, Field(description="ISO 8601; defaults to now.")] = None,
    medication_name: Annotated[str | None, Field(description="Filter by medication.")] = None,
) -> dict:
    """Get medication adherence data."""
    return await _ohd.medication_adherence(from_time, to_time, medication_name)


@high_level_mcp.tool
async def get_food_log(
    from_time: Annotated[str | None, Field(description="ISO 8601; defaults to 7 days ago.")] = None,
    to_time: Annotated[str | None, Field(description="ISO 8601; defaults to now.")] = None,
    include_nutrition_totals: bool = True,
) -> dict:
    """Get the user's food log with optional nutrition aggregates."""
    return await _ohd.food_log(from_time, to_time, include_nutrition_totals)


@high_level_mcp.tool
async def find_patterns(
    event_type: Annotated[str, Field(description="Event type to analyze.")],
    description: Annotated[str, Field(description="Natural-language pattern description, e.g. 'unusually high readings'.")],
    from_time: Annotated[str | None, Field(description="ISO 8601.")] = None,
    to_time: Annotated[str | None, Field(description="ISO 8601.")] = None,
) -> list[dict]:
    """Find events matching a natural-language pattern description.

    Implementation uses statistical thresholds (z-scores, percentiles) for
    phrases like 'unusually high'. For more semantic queries, a small LLM
    classifier can be added later.
    """
    return await _ohd.find_patterns(event_type, description, from_time, to_time)


@high_level_mcp.tool
async def chart(
    description: Annotated[str, Field(description="Natural-language chart description.")],
) -> dict:
    """Generate a chart from a natural-language description.

    Returns {image_base64, chart_spec, underlying_data}. The chart_spec
    can be saved as a reusable template.
    """
    return await _ohd.chart_from_description(description)
```

## Entry point and deployment

### Local install (stdio, into Claude Desktop)

```bash
# Install the fastmcp CLI
uv tool install fastmcp

# Install the Connect MCP into Claude Desktop
fastmcp install connect_mcp/server.py \
    --name "OHD Connect" \
    --env OHD_BASE_URL=https://ohd.example.com \
    --env OHD_WRITE_TOKEN=<token>

# Install the Care MCP
fastmcp install care_mcp/server.py \
    --name "OHD Care (self)" \
    --env OHD_BASE_URL=https://ohd.example.com \
    --env OHD_TOKEN=<session_token>
```

Claude Desktop then lists these servers and makes their tools available in chat.

### Remote deployment (HTTP, part of OHD Storage)

In the OHD Storage Docker deployment, we can run Care MCP as a sibling service:

```yaml
# docker-compose.yml (excerpt)
services:
  ohd-api:
    # ... the FastAPI service

  ohd-care-mcp:
    image: ohd/ohd-care-mcp:latest
    environment:
      OHD_BASE_URL: http://ohd-api:8000
      # Inherits auth from the incoming request; see OAuth proxy below
    expose:
      - "8001"

  caddy:
    # ... route /mcp/cord/* to ohd-care-mcp:8001
```

Then Claude.ai (or any MCP-aware client) connects to `https://ohd.example.com/mcp/cord/` with a grant token.

### Development and testing

```bash
# Interactive tool testing with the FastMCP inspector
fastmcp dev connect_mcp/server.py

# List tools on a running server
fastmcp list http://localhost:8001/mcp

# Call a tool from the CLI (great for integration tests)
fastmcp call http://localhost:8001/mcp summarize \
    --arg event_type=glucose \
    --arg period=daily
```

## Authentication with FastMCP's OAuth proxy

FastMCP 2.x ships with an OAuth proxy that handles OIDC flows for MCP clients. For the remote Care MCP, this means:

1. Claude.ai (or any MCP client) connects and needs auth.
2. FastMCP redirects to the configured OIDC provider (the same one OHD Storage uses).
3. User authenticates, receives a session.
4. The MCP client includes the bearer token on subsequent calls.
5. FastMCP validates the token via OHD Storage's introspection endpoint before routing the request.

```python
from fastmcp import FastMCP
from fastmcp.server.auth import OAuthProxy

mcp = FastMCP(
    "OHD Care",
    auth=OAuthProxy(
        # Forward OAuth to OHD Storage's OIDC flow
        issuer_url="https://ohd.example.com/auth",
        token_introspection_url="https://ohd.example.com/auth/introspect",
        required_scopes=["ohd.read"],
    ),
)
```

For the Connect MCP run locally, we skip OAuth entirely — the server is already authenticated by virtue of holding the write token in env.

## Handling time input

Users (and LLMs) will pass time in natural phrases: "yesterday", "30 minutes ago", "last Tuesday". Accept both ISO 8601 and natural phrases server-side:

```python
from datetime import datetime, timezone
from dateparser import parse

def resolve_time(value: str | None, default: datetime | None = None) -> datetime:
    if value is None:
        return default or datetime.now(timezone.utc)
    dt = parse(value, settings={"TIMEZONE": "UTC", "RETURN_AS_TIMEZONE_AWARE": True})
    if dt is None:
        raise ValueError(f"Could not parse timestamp: {value!r}")
    return dt
```

This runs *inside* each tool, right after argument validation. Keeps the tool signatures honest (they still declare `str`) while being forgiving at the edges.

## Tool catalog management (Phase 2+)

Once the tool count grows past ~15, LLMs start to struggle with discovery. FastMCP 3's **search transforms** let the server respond to LLM tool-discovery queries with just the relevant subset:

```python
from fastmcp.transforms import SearchTransform

mcp = FastMCP("OHD Care", transforms=[SearchTransform()])
```

With this, an LLM asking "how do I get medication data" gets back a short list focused on medication tools, not the whole catalog. Not Phase 1, but a nice escape valve.

## Testing strategy

- **Unit tests:** test each tool function directly (FastMCP 3 keeps functions callable, so unit tests are trivial — no mocking of decorators).
- **Integration tests:** spin up OHD Storage in Docker Compose, point MCP servers at it, call tools via the FastMCP client library, assert effects.
- **Live testing:** `fastmcp dev` launches the inspector for manual poking; `fastmcp install` deploys to Claude Desktop for end-to-end LLM testing.

```python
# Example integration test
import pytest
from fastmcp import Client
from connect_mcp.server import mcp as connect_mcp

@pytest.mark.asyncio
async def test_log_symptom_end_to_end(ephemeral_ohd):
    async with Client(connect_mcp) as c:
        result = await c.call_tool("log_symptom", {
            "symptom": "headache",
            "severity": "moderate",
        })
        assert result["status"] == "logged"
        # Verify it landed in OHD
        events = await ephemeral_ohd.get_events(event_type="symptom")
        assert len(events) == 1
        assert events[0]["data"]["symptom"] == "headache"
```

## Dependencies

```toml
# pyproject.toml (MCP servers)
[project]
dependencies = [
    "fastmcp>=3.0",
    "httpx",
    "pydantic>=2",
    "dateparser",
]
```

## Open questions

- **OAuth proxy configuration.** FastMCP's OAuth proxy supports OIDC; we need to confirm it plays nicely with the OIDC providers we'll offer users (Google, Keycloak, Authentik).
- **Per-component authorization.** FastMCP 3 supports per-component auth, so a single deployed Care MCP could restrict specific tools to specific scopes ("chart" requires `ohd.read` but "find_patterns" requires `ohd.read+analysis"). Useful for tiered access.
- **Streaming results.** Large queries shouldn't block. FastMCP supports progressive responses via the Context object; use for `query_events` with large ranges and for `chart` with slow rendering.
- **Apps (interactive UIs).** FastMCP 3 added "Apps" — interactive UIs rendered in the conversation. A "log this meal" app (barcode scanner, quantity input, nutrition display) could run inside a Claude.ai chat. Worth exploring once MVP is shipped.
