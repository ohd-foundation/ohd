# OHD Care — MCP Server (Python + FastMCP)

Operator-side OHDC consumer exposed as an MCP server. Tools route per-patient
operations through the **active grant** the LLM selects via
`switch_patient(label)`. The grant vault is a real (not stubbed) state
machine — see `care/SPEC.md` §10.6 for the safety rules.

## Status

Full tool surface registered. Real Pydantic input validation, real multi-patient grant vault (seeded from a JSON file), real OHDC client over `OhdcTransport` from [`ohd-shared`](../../packages/python/ohd-shared). See [`STATUS.md`](STATUS.md) for per-tool wire state.

## Stack

- Python 3.11+
- [`fastmcp`](https://github.com/jlowin/fastmcp) (standalone FastMCP framework, 3.x)
- `pydantic` v2
- `anyio`
- `uv` for dependency management
- `ohd-shared` workspace package — proto stubs, transport, query-hash, OAuth proxy

## Install

```bash
cd care/mcp
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
# Seed the grant vault from a JSON file (required for multi-patient routing).
cat > /tmp/care-grants.json <<'EOF'
[
  {"label": "alice",   "grant_token": "ohdg_…",  "scope_summary": "primary care"},
  {"label": "bob_v2",  "grant_token": "ohdg_…",  "scope_summary": "cardiology consult"}
]
EOF

OHD_STORAGE_URL=https://ohd.example.com \
OHD_OPERATOR_TOKEN=ohds_… \
OHD_CARE_GRANTS_FILE=/tmp/care-grants.json \
uv run python -m ohd_care_mcp
```

### Environment

| Var | Default | Purpose |
|---|---|---|
| `OHD_STORAGE_URL` | `http://127.0.0.1:18443` | OHDC server base URL. |
| `OHD_OPERATOR_TOKEN` | _unset_ | Operator session token (OIDC; FastMCP's OAuth proxy will replace this for remote deployments). |
| `OHD_CARE_GRANTS_FILE` | _unset_ | Path to a JSON file listing patient grants `[{label, grant_token, scope_summary?}]`. |
| `OHD_MCP_TRANSPORT` | `stdio` | `stdio` or `http`. |
| `OHD_MCP_HTTP_HOST` | `127.0.0.1` | HTTP host. |
| `OHD_MCP_HTTP_PORT` | `8766` | HTTP port. |

## Test

```bash
uv run pytest
```

Tests assert the tool catalog matches `care/SPEC.md` §10, the grant vault
state machine works, per-patient tools refuse without an active patient,
and write tools refuse without `confirm=True`.

## Layout

```
mcp/
├── pyproject.toml
├── README.md
├── STATUS.md
├── src/
│   └── ohd_care_mcp/
│       ├── __init__.py
│       ├── __main__.py
│       ├── config.py        # env loading + grant-vault file reader
│       ├── grant_vault.py   # in-memory multi-patient state machine
│       ├── ohdc_client.py   # OHDC client stub (raises OhdcNotWiredError)
│       ├── server.py        # FastMCP bootstrap
│       └── tools.py         # all 20 tools (patient mgmt + read + write + workflow)
└── tests/
    ├── __init__.py
    └── test_tools.py
```

## Tool catalog

Per [`care/SPEC.md`](../SPEC.md) §10:

- **§10.1 Patient management** — `list_patients`, `switch_patient`, `current_patient`
- **§10.2 Read** — `query_events`, `query_latest`, `summarize`, `correlate`,
  `find_patterns`, `chart`, `get_medications_taken`, `get_food_log`
- **§10.3 Write-with-approval** — `submit_lab_result`, `submit_measurement`,
  `submit_observation`, `submit_clinical_note`, `submit_prescription`,
  `submit_referral`
- **§10.4 Workflow** — `draft_visit_summary`, `compare_to_previous_visit`,
  `find_relevant_context_for_complaint`

§10.5 case tools (`open_case`, `close_case`, `handoff_case`, `list_cases`)
are **deliberately deferred** — they're cross-cutting with `connect/mcp`
and `emergency/mcp` and the case lifecycle on the operator side hasn't
been pinned. Add them in v0.2 once the case state machine has a single
home.

## Safety rules (per SPEC §10.6)

- The active patient label appears in **every** tool result (`active_patient`
  field).
- `switch_patient` is the **only** tool that changes active context.
- Write tools require `confirm=True` before submitting; the safety guard
  is enforced at the tool layer, not just convention.

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
