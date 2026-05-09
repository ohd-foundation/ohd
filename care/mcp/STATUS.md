# OHD Care MCP — Status / Handoff

> Snapshot of what's scaffolded for the Python + FastMCP rewrite of the
> Care MCP server.

## OHDC wire/API version renamed to v0 (2026-05-09)

The MCP server's generated protobuf stubs and OHDC client imports now target
`_gen/ohdc/v0/` and `ohdc.v0`.

**Phase:** v0.3 — §10.5 case tools landed, canonical_query_hash matches TS / cli byte-for-byte, operator-side audit recorded on every read RPC.
**Date:** 2026-05-09

## Shared `ohd-shared` package extraction (2026-05-09)

The Connect-RPC transport, OHDC proto<->dict helpers, canonical query
hash, generated proto stubs, and OAuth proxy bootstrap moved into the
new `packages/python/ohd-shared/` workspace package. This MCP now
consumes them via `ohd-shared[oauth]` (path dep declared in
`pyproject.toml`'s `[tool.uv.sources]`).

What stayed local (component-specific glue):

- `ohdc_client.py` — the multi-patient `OhdcClient` class shape with
  per-call `grant_token` and operator-side audit stamping.
- `_connect_transport.py` — re-export shim.
- `canonical_query_hash.py` — re-export shim.
- `operator_audit.py` — Care-MCP-specific persistence (`OHD_CARE_MCP_AUDIT_DIR`).
- `grant_vault.py`, `tools.py`, `config.py`, `server.py` — unchanged in shape.

The local `_gen/` directory was deleted; codegen now writes to
`packages/python/ohd-shared/src/ohd_shared/_gen/` via the shared
`scripts/regen_proto.sh`. This MCP's `scripts/regen_proto.sh` forwards
to the shared script.

## What landed

- TypeScript scaffold (`@modelcontextprotocol/sdk`) deleted; replaced with
  Python + FastMCP per the Pinned implementation decisions in the repo
  root README and per [`spec/docs/research/mcp-servers.md`](../../spec/docs/research/mcp-servers.md).
- `pyproject.toml` (PEP 621, `uv` / hatchling), `src/ohd_care_mcp/`
  package, `tests/` with smoke tests using `fastmcp.Client`.
- 26 tools from [`care/SPEC.md`](../SPEC.md) §10.1–§10.5 registered with
  pydantic-validated input, real docstrings, and a stubbed OHDC body.
  §10.5 case tools (`open_case`, `close_case`, `list_cases`, `get_case`,
  `force_close_case`, `issue_retrospective_grant`) landed in v0.3.
- **Multi-patient grant vault is real**, not stubbed:
  - `config.CareMcpConfig.from_env()` reads `OHD_CARE_GRANTS_FILE` (JSON
    list of `{label, grant_token, scope_summary?}`) into a `list[PatientGrant]`.
  - `grant_vault.GrantVault` keeps an in-memory dict + `active_label`.
  - `switch_patient(label)` is the only tool that changes active context;
    every per-patient tool calls `vault.require_current()` and refuses if
    no patient is active.
  - The active patient label is surfaced in every tool result via
    `_orient(vault)` (per SPEC §10.6).
- Write-with-approval safety guard: every `submit_*` tool requires
  `confirm=True`. Without it, the tool raises `PermissionError(
  "Refusing to submit … to <patient> without confirm=True")`.

## OHDC client — wire status

`src/ohd_care_mcp/ohdc_client.py` is a **real Connect-RPC client over
`httpx`**, mirroring `connect/mcp/src/ohd_connect_mcp/ohdc_client.py`.
Hand-rolled wire framing lives in
`src/ohd_care_mcp/_connect_transport.py` (Connect protocol unary +
server-streaming over HTTP/2). Generated protobuf message types live
under `src/ohd_care_mcp/_gen/ohdc/v0/ohdc_pb2.py`; regenerate with
`bash scripts/regen_proto.sh`.

Per SPEC: every method takes `grant_token` as a kwarg, attached as
`Authorization: Bearer {grant_token}`. The token is *not* held on the
client — `grant_vault.GrantVault` supplies it from the active patient.

| OHDC method | Status | Notes |
|---|---|---|
| `who_am_i` | wired | Probe; not surfaced as a tool today. |
| `query_events` | wired | Server-streaming Connect-RPC; computes `canonical_query_hash` + records operator-side audit row before/after the call. |
| `get_event_by_ulid` | wired | Records operator audit row keyed on `query_kind="get_event_by_ulid"`. |
| `put_events` | wired | Backs every `submit_*` tool. |
| `list_pending` | wired | Operator's own queued submissions. |
| `open_case` / `close_case` / `list_cases` / `get_case` | wired | Maps to OHDC `CreateCase` / `CloseCase` / `ListCases` / `GetCase` (per `care/SPEC.md` §4 + §10.5). |
| `issue_retrospective_grant` | wired with fallback | Maps to `CreateGrant` with `case_ulids` set + `approval_mode='always'`. Storage's `CreateGrant` is documented as self-session-only; we surface a typed `OhdcNotWiredError` if storage refuses, so the LLM can fall back to "ask the patient to issue this from OHD Connect" rather than a wire-level stack. |
| `aggregate` | `OhdcNotWiredError` | Storage `Aggregate` is `Unimplemented` in v1.x. |
| `correlate` | `OhdcNotWiredError` | Storage `Correlate` is `Unimplemented` in v1.x. |
| `find_relevant_context` | `OhdcNotWiredError` | Needs storage-side complaint classifier or client-side composition over `query_events` (v0.x). |

Care MCP does **not** need `create_grant` / `revoke_grant` (Care holds
grants, doesn't issue them) or `audit_query` (storage's `AuditQuery` is
`Unimplemented` in v1) — those are explicitly out of surface here.

## Tests

- **Unit tests** (default): `MockOhdcClient` in-process. Covers tool
  registration, the patient state machine, write-confirm guard, tool →
  client transforms (verifies the active patient's grant token is
  threaded through), and the `OhdcNotWiredError` surface for the
  unwired methods.
- **Integration tests** (`-m integration`): spin up a real
  `ohd-storage-server` in a temp dir via the `init` → `issue-self-token`
  → `serve` CLI flow, then drive Care MCP tools end-to-end. The
  fixture is module-scoped so the binary spins up once. Skips
  gracefully when the binary is missing.

```bash
cd care/mcp
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
cd care/mcp
uv sync
uv run pytest -m "not integration"
uv run python -m ohd_care_mcp     # smoke-boot; Ctrl-C
```

## Two-sided audit (per `care/SPEC.md` §7)

- `src/ohd_care_mcp/canonical_query_hash.py` — byte-identical mirror of
  `care/web/src/ohdc/canonicalQueryHash.ts`. Cross-language parity is
  asserted by `tests/test_canonical_query_hash.py`, which loads the
  shared vectors at `care/web/src/ohdc/__golden__/query_hash_vectors.json`
  (the TS test does the same; passing on both sides means the operator-
  side audit JOIN per §7.3 holds).
- `src/ohd_care_mcp/operator_audit.py` — JSONL-backed (or in-memory)
  rolling 1000-entry audit. Persistent path is taken iff
  `OHD_CARE_MCP_AUDIT_DIR` is set; otherwise in-memory only (Care MCP
  can be ephemeral; the patient-side audit is the durable log).

## Deferred (intentional)

- **No-PHI-to-external-LLMs guardrail** — Care SPEC §14 mentions a
  `no_phi_to_external_llms` config knob; not implemented at this layer
  (cleaner to enforce at the FastMCP transport / OAuth proxy layer once
  remote HTTP deployment lands).
- **Encrypted at-rest grant storage** — SPEC §14 says deployment KMS;
  v0 vault is in-memory only.
- **Source signing for clinical writes** — SPEC §6.2; deferred until
  storage exposes the signing slot.

## Next steps

1. ~~OAuth proxy for the remote Streamable HTTP transport, against the
   operator's IdP (Okta / Keycloak / Authentik).~~ Landed 2026-05-09:
   when `OHD_CARE_OIDC_ISSUER`, `OHD_CARE_OIDC_CLIENT_ID`, and
   `OHD_CARE_MCP_BASE_URL` are set and `OHD_MCP_TRANSPORT=http`,
   `build_server` wires `fastmcp.server.auth.oauth_proxy.OAuthProxy`
   with `JWTVerifier` against the issuer's JWKS. Discovery is via
   `.well-known/oauth-authorization-server` with OIDC
   `/openid-configuration` fallback. Per
   `spec/docs/design/care-auth.md` "Operator authentication into Care".
2. ~~Add §10.5 case tools once the case lifecycle is pinned.~~ Landed
   2026-05-09: `open_case`, `close_case`, `list_cases`, `get_case`,
   `force_close_case`, `issue_retrospective_grant` registered.
3. Encrypted grant vault at rest. Mechanically: import the same KMS
   abstraction that `care/cli/src/ohd_care/kms.py` ships (keyring
   default, passphrase fallback, AES-GCM envelope). The grant_vault
   here is in-memory only today, but the configured grants are loaded
   from `OHD_CARE_GRANTS_FILE` (plaintext JSON) — that input file is
   what needs to land on the encrypted-envelope path.
4. Storage-side wire-up of `Aggregate`, `Correlate`,
   `find_relevant_context` (drop the `OhdcNotWiredError` shims once
   storage exposes the handlers).
