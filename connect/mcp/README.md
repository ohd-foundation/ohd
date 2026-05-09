# OHD Connect — MCP Server (Python + FastMCP)

Personal-side OHDC consumer exposed as an MCP server. Tools log the user's
own health events, read back their data, and manage grants / pending writes
/ cases / audit on their own OHD instance. Authentication is the user's
self-session token; the LLM never sees the token.

## Status

Full tool surface registered with real Pydantic input validation. The OHDC client uses `OhdcTransport` from [`ohd-shared`](../../packages/python/ohd-shared) over async HTTP/2. See [`STATUS.md`](STATUS.md) for the per-tool wire-up state.

## Stack

- Python 3.11+
- [`fastmcp`](https://github.com/jlowin/fastmcp) (standalone FastMCP framework, 3.x)
- `pydantic` v2 (tool input/output schemas)
- `anyio` (async runtime)
- `uv` for dependency management (PEP 621 / `pyproject.toml`)
- `ohd-shared` workspace package — proto stubs, transport, query-hash, OAuth proxy

## Install

```bash
cd connect/mcp
uv sync                 # creates .venv, installs deps + ohd-shared via workspace path
```

Without `uv`:

```bash
python3.11 -m venv .venv
. .venv/bin/activate
pip install -e ".[dev]"
```

`pyproject.toml` references `ohd-shared` via `[tool.uv.sources]` with a relative path (`../../packages/python/ohd-shared`). When using vanilla pip, install `ohd-shared` first (`pip install -e ../../packages/python/ohd-shared`).

## Run

```bash
# stdio (Claude Desktop / Cursor / Continue local install)
uv run python -m ohd_connect_mcp

# Streamable HTTP (remote, alongside OHD Storage)
OHD_MCP_TRANSPORT=http \
OHD_MCP_HTTP_HOST=0.0.0.0 \
OHD_MCP_HTTP_PORT=8765 \
uv run python -m ohd_connect_mcp

# Or via the installed entry point
uv run ohd-connect-mcp
```

### Environment

| Var | Default | Purpose |
|---|---|---|
| `OHD_STORAGE_URL` | `http://127.0.0.1:18443` | OHDC server base URL. |
| `OHD_ACCESS_TOKEN` | _unset_ | Self-session token. Bound at install time for stdio; OAuth proxy will provide it for remote. |
| `OHD_MCP_TRANSPORT` | `stdio` | `stdio` or `http`. |
| `OHD_MCP_HTTP_HOST` | `127.0.0.1` | HTTP host (when `OHD_MCP_TRANSPORT=http`). |
| `OHD_MCP_HTTP_PORT` | `8765` | HTTP port. |

## Test

```bash
uv run pytest
```

The suite uses FastMCP's in-process `Client` to discover tools and call
them through the same path the LLM does. Tests assert the tool catalog
matches `connect/SPEC.md` and that calls against the stubbed OHDC client
surface a clear `not yet wired` error.

## Layout

```
mcp/
├── pyproject.toml
├── README.md
├── STATUS.md
├── src/
│   └── ohd_connect_mcp/
│       ├── __init__.py
│       ├── __main__.py
│       ├── config.py        # env var loading
│       ├── ohdc_client.py   # OHDC client stub (raises OhdcNotWiredError)
│       ├── server.py        # FastMCP bootstrap
│       └── tools.py         # all 27 tools
└── tests/
    ├── __init__.py
    └── test_tools.py        # tool registration + smoke tests
```

## Tool catalog

Per [`connect/SPEC.md`](../SPEC.md) "Connect MCP — tool list":

- **Logging** — `log_symptom`, `log_food`, `log_medication`,
  `log_measurement`, `log_exercise`, `log_mood`, `log_sleep`,
  `log_free_event`
- **Reading** — `query_events`, `query_latest`, `summarize`, `correlate`,
  `find_patterns`, `get_medications_taken`, `get_food_log`, `chart`
- **Grants** — `create_grant`, `list_grants`, `revoke_grant`
- **Pending review** — `list_pending`, `approve_pending`, `reject_pending`
- **Cases** — `list_cases`, `get_case`, `force_close_case`,
  `issue_retrospective_grant`
- **Audit** — `audit_query`

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
