# OHD Emergency — MCP Server (Python + FastMCP)

Triage-assistant OHDC consumer exposed as an MCP server. Narrow tool
surface (5 high-level tools + 2 case-selection tools) per
`emergency/SPEC.md` §3.2 — emergencies are time-critical and the LLM is
deliberately not given exploratory analytics power.

## Status

Full tool surface registered with real Pydantic input validation, real case-vault state machine, and a real OHDC client over `OhdcTransport` from [`ohd-shared`](../../packages/python/ohd-shared). See [`STATUS.md`](STATUS.md) for per-tool wire state.

## Stack

- Python 3.11+
- [`fastmcp`](https://github.com/jlowin/fastmcp) (3.x)
- `pydantic` v2
- `anyio`
- `uv` for dependency management
- `ohd-shared` workspace package — proto stubs, transport, query-hash, OAuth proxy

## Install

```bash
cd emergency/mcp
uv sync                  # creates .venv, installs deps + ohd-shared via workspace path
```

Without `uv`:

```bash
python3.11 -m venv .venv
. .venv/bin/activate
pip install -e ../../packages/python/ohd-shared
pip install -e ".[dev]"
```

## Run

```bash
# Seed the case-grant vault from a JSON file.
cat > /tmp/em-cases.json <<'EOF'
[
  {"case_id": "01HC…", "grant_token": "ohdg_…", "label": "MVA scene 14:32"}
]
EOF

OHD_STORAGE_URL=https://ohd.example.com \
OHD_OPERATOR_TOKEN=ohds_… \
OHD_EMERGENCY_CASES_FILE=/tmp/em-cases.json \
OHD_EMERGENCY_MCP_ALLOW_EXTERNAL_LLM=false \
uv run python -m ohd_emergency_mcp
```

### Environment

| Var | Default | Purpose |
|---|---|---|
| `OHD_STORAGE_URL` | `http://127.0.0.1:18443` | OHDC server base URL. |
| `OHD_OPERATOR_TOKEN` | _unset_ | Operator session token (OIDC). |
| `OHD_EMERGENCY_CASES_FILE` | _unset_ | Path to a JSON file listing case grants `[{case_id, grant_token, label?}]`. |
| `OHD_EMERGENCY_MCP_ALLOW_EXTERNAL_LLM` | `false` | Per SPEC §3.3 — default-deny external LLMs. Origin enforcement at the FastMCP transport layer is a wire-up agent's job. |
| `OHD_MCP_TRANSPORT` | `stdio` | `stdio` or `http`. |
| `OHD_MCP_HTTP_HOST` | `127.0.0.1` | HTTP host. |
| `OHD_MCP_HTTP_PORT` | `8767` | HTTP port. |

## Test

```bash
uv run pytest
```

The suite asserts:

- The narrow tool catalog matches `emergency/SPEC.md` §3.2.
- Generic `query_events` / `put_events` are NOT exposed.
- The case vault state machine works.
- Triage tools refuse when no case is active.
- Triage tools surface the OHDC stub error when called with an active case.

## Layout

```
mcp/
├── pyproject.toml
├── README.md
├── STATUS.md
├── src/
│   └── ohd_emergency_mcp/
│       ├── __init__.py
│       ├── __main__.py
│       ├── config.py        # env loading + cases-file reader
│       ├── case_vault.py    # in-memory case-grant state machine
│       ├── ohdc_client.py   # OHDC client stub (raises OhdcNotWiredError)
│       ├── server.py        # FastMCP bootstrap
│       └── tools.py         # all 7 tools
└── tests/
    ├── __init__.py
    └── test_tools.py
```

## Tool catalog

Per [`emergency/SPEC.md`](../SPEC.md) §3:

- **Case selection** (analogous to Care MCP's switch_patient, per §3.1)
  — `list_active_cases`, `set_active_case`
- **Triage tools** (§3.2)
  — `find_relevant_context_for_complaint`,
  `summarize_vitals`,
  `flag_abnormal_vitals`,
  `check_administered_drug`,
  `draft_handoff_summary`

Generic `query_events` / `put_events` are **deliberately** not exposed.

## License

Dual-licensed `Apache-2.0 OR MIT`.
