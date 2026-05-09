# OHD Emergency MCP — Status / Handoff

> Snapshot of what's scaffolded for the Python + FastMCP rewrite of the
> Emergency MCP server.

## OHDC wire/API version renamed to v0 (2026-05-09)

Generated protobuf stubs and OHDC client wiring now reference
`_gen/ohdc/v0/` and `ohdc.v0`.

**Phase:** v0.2 — OHDC client wired (real Connect-RPC over httpx); narrow tool surface registered; case-grant vault is a real state machine. Unit + integration tests pass.
**Date:** 2026-05-08

## Shared `ohd-shared` package extraction (2026-05-09)

The Connect-RPC transport, OHDC proto<->dict helpers, generated proto
stubs, and OAuth proxy bootstrap moved into the new
`packages/python/ohd-shared/` workspace package. This MCP now consumes
them via `ohd-shared[oauth]` (path dep declared in `pyproject.toml`'s
`[tool.uv.sources]`).

What stayed local (component-specific glue):

- `ohdc_client.py` — the case-bound `OhdcClient` class shape (per-call
  grant token; narrow `who_am_i`/`query_events`/`put_events` surface
  plus `OhdcNotWiredError`-raising stubs).
- `_connect_transport.py` — re-export shim.
- `case_vault.py`, `tools.py`, `config.py`, `server.py` — unchanged in shape.

The local `_gen/` directory was deleted; codegen now writes to
`packages/python/ohd-shared/src/ohd_shared/_gen/` via the shared
`scripts/regen_proto.sh`. This MCP's `scripts/regen_proto.sh` forwards
to the shared script.

## What landed

- TypeScript scaffold (`@modelcontextprotocol/sdk`) deleted; replaced with
  Python + FastMCP per the Pinned implementation decisions in the repo
  root README.
- `pyproject.toml` (PEP 621, `uv` / hatchling), `src/ohd_emergency_mcp/`
  package, `tests/` with smoke tests using `fastmcp.Client`.
- 7 tools per [`emergency/SPEC.md`](../SPEC.md) §3:
  - `list_active_cases`, `set_active_case` (case-selection state machine,
    per §3.1).
  - `find_relevant_context_for_complaint`, `summarize_vitals`,
    `flag_abnormal_vitals`, `check_administered_drug`,
    `draft_handoff_summary` (triage tools, per §3.2).
- Per §3.2, generic `query_events` / `put_events` are **deliberately not
  exposed**; a smoke test asserts that.
- Per §3.3, `OHD_EMERGENCY_MCP_ALLOW_EXTERNAL_LLM` defaults to false; the
  setting is read into the config and surfaced in the FastMCP server's
  `instructions` string so any LLM connecting to a default-deployed server
  sees a PHI guardrail.

## OHDC client — wire status

`src/ohd_emergency_mcp/ohdc_client.py` is a **real Connect-RPC client
over `httpx`**, mirroring `connect/mcp/src/ohd_connect_mcp/ohdc_client.py`.
Hand-rolled wire framing lives in
`src/ohd_emergency_mcp/_connect_transport.py` (Connect protocol unary +
server-streaming over HTTP/2). Generated protobuf message types live
under `src/ohd_emergency_mcp/_gen/ohdc/v0/ohdc_pb2.py`; regenerate with
`bash scripts/regen_proto.sh`.

Per `emergency/SPEC.md` §3.2 the LLM-facing tool surface is intentionally
narrow (5 triage tools + 2 case-selection tools); the OHDC client surface
is similarly narrow:

| OHDC method | Status | Notes |
|---|---|---|
| `who_am_i` | wired | Probe; not surfaced as a tool today. |
| `query_events` | wired | Backs `flag_abnormal_vitals`, `draft_handoff_summary`, and (eventually) the higher-level helpers. |
| `put_events` | wired | Backs `draft_handoff_summary` writing the summary back to the case timeline. |
| `aggregate` | `OhdcNotWiredError` | Storage `Aggregate` is `Unimplemented`; `summarize_vitals` will compose `query_events` client-side instead in v0.x. |
| `find_relevant_context` | `OhdcNotWiredError` | Needs storage-side complaint classifier or client-side composition over `query_events`. |
| `check_drug_interaction` | `OhdcNotWiredError` | Needs the operator-provided drug-interaction dataset (CSV/JSON loaded at server start), per SPEC §3.2. Out of OHDC scope. |

Every method takes `grant_token` as a kwarg, attached as
`Authorization: Bearer {grant_token}` per call. The token is *not* held
on the client — `case_vault.CaseVault` supplies it from the active case.

## Tests

- **Unit tests** (default): `MockOhdcClient` in-process. Covers tool
  registration, the case state machine, the deliberate-narrowness check
  (no `query_events` / `put_events` exposed to the LLM), tool → client
  transforms (verifies the active case's grant token is threaded
  through), and the `OhdcNotWiredError` surface for the unwired methods.
- **Integration tests** (`-m integration`): spin up a real
  `ohd-storage-server` in a temp dir via the `init` →
  `issue-self-token` → `serve` CLI flow, then drive Emergency MCP tools
  end-to-end. The fixture is module-scoped so the binary spins up once.
  Skips gracefully when the binary is missing.

```bash
cd emergency/mcp
uv run pytest -m "not integration"     # unit only — fast
uv run pytest -m integration           # integration — needs ohd-storage-server
uv run pytest                          # both
```

## Pinned versions

| Package | Version | Why |
|---|---|---|
| Python | `>=3.11` | Modern type hints; matches `care/cli`. |
| `fastmcp` | `>=2.11,<4` | Standalone FastMCP — currently 3.2.4 on PyPI. |
| `pydantic` | `>=2.7` | FastMCP's hard dep. |
| `anyio` | `>=4.4` | FastMCP's async runtime. |
| `httpx` | `>=0.27` | Connect-RPC transport. |
| `protobuf` | `>=5.0` | Generated `ohdc_pb2` runtime. |
| `grpcio-tools` | `>=1.80` (dev) | `protoc` driver for `regen_proto.sh`. |

## Smoke test

```bash
cd emergency/mcp
uv sync
uv run pytest -m "not integration"
uv run python -m ohd_emergency_mcp     # smoke-boot; Ctrl-C
```

## OIDC wired (mirroring connect/mcp pattern) — 2026-05-09

The Streamable-HTTP transport now fronts itself with FastMCP's
`OAuthProxy` against the operator IdP when the operator sets:

- `OHD_EMERGENCY_OIDC_ISSUER` — operator IdP issuer URL (Keycloak /
  Authentik / Auth0 / Azure AD).
- `OHD_EMERGENCY_OIDC_CLIENT_ID` — OAuth client_id registered with
  the issuer for this MCP.
- `OHD_EMERGENCY_OIDC_CLIENT_SECRET` — confidential-client secret.
- `OHD_EMERGENCY_MCP_BASE_URL` — public URL the MCP is exposed at
  (used by the OAuth proxy for redirect-URI rendering).

Discovery hits `/.well-known/oauth-authorization-server` (RFC 8414)
with fallback to `/openid-configuration`, picks up the JWKS, and
wires `JWTVerifier` + `OAuthProxy` exactly as `connect/mcp` does.
Generic `OHD_OIDC_*` env names also work as a fallback for shared
deployments.

When any of those is missing, or the transport is `stdio`, the
proxy is disabled — useful for Claude Desktop installs where the
operator already issued a pre-shared bearer.

5 new unit tests added in `tests/test_tools.py` (`OidcProxyConfig`
enabled/disabled matrix + env-var read + a build_server smoke test
that confirms `auth=None` is the default). All 16 unit tests pass
(`uv run pytest`).

## Deferred (intentional)

- **External-LLM origin allowlist** — §3.3 says the server should refuse
  tool calls coming from clients whose origin is outside the operator's
  allowed list when `ALLOW_EXTERNAL_LLM=false`. The OAuthProxy now
  validates issuer-signed tokens at the transport layer; the
  origin-check / scope-allowlist on top is still a v0.x task. For v0
  the config is read and surfaced in the `instructions` string only.
- **Drug-interaction dataset** — operator-provided artefact, not in
  scope for the MCP scaffold.
- **Active case from relay-issued reopen tokens** — per
  `emergency/SPEC.md` §2 the dispatcher can issue case reopen tokens.
  Wiring those into `case_vault` (so the LLM can resume a case after
  inactivity-driven auto-close) is a v0.2 task.

## Next steps

1. Origin allowlist enforcement when `ALLOW_EXTERNAL_LLM=false` (the
   FastMCP transport / OAuth proxy layer).
2. Drug-interaction dataset loader (drops the `OhdcNotWiredError` shim
   on `check_drug_interaction`).
3. Storage-side wire-up of `Aggregate` (drops the shim on
   `summarize_vitals`'s direct path; the v0.x fallback composes
   `query_events` client-side).
4. Reopen-token flow for `case_vault` (relay-issued case-reopen tokens
   feed in alongside paramedic-tablet-issued case grants).
