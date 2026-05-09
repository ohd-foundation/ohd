# OHD Connect MCP — Status / Handoff

> Snapshot of what's scaffolded for the Python + FastMCP rewrite of the
> Connect MCP server.

## OHDC wire/API version renamed to v0 (2026-05-09)

Generated protobuf stubs and client imports now target `_gen/ohdc/v0/` and
the `ohdc.v0` package.

**Phase:** v0.1 — full tool surface registered against a stubbed OHDC client. Ready for the wire-up agent to fill in the OHDC RPC calls.
**Date:** 2026-05-08

## Shared `ohd-shared` package extraction (2026-05-09)

The Connect-RPC transport, OHDC proto<->dict helpers, generated proto
stubs, and OAuth proxy bootstrap moved into the new
`packages/python/ohd-shared/` workspace package. This MCP now consumes
them via `ohd-shared[oauth]` (path dep declared in `pyproject.toml`'s
`[tool.uv.sources]`).

What stayed local (component-specific glue):

- `ohdc_client.py` — the self-session-mode `OhdcClient` class shape
  (default access token, owner-side surface).
- `_connect_transport.py` — re-export shim.
- `tools.py`, `config.py`, `server.py` — unchanged in shape.

The local `_gen/` directory was deleted; codegen now writes to
`packages/python/ohd-shared/src/ohd_shared/_gen/` via the shared
`scripts/regen_proto.sh`. This MCP's `scripts/regen_proto.sh` forwards
to the shared script.

## What landed

- TypeScript scaffold (`@modelcontextprotocol/sdk`) deleted; replaced with
  Python + FastMCP per the Pinned implementation decisions in the repo
  root README and per [`spec/docs/research/mcp-servers.md`](../../spec/docs/research/mcp-servers.md).
- `pyproject.toml` (PEP 621, `uv` / hatchling), `src/ohd_connect_mcp/`
  package, `tests/` with smoke tests using `fastmcp.Client`.
- All 27 tools from [`connect/SPEC.md`](../SPEC.md) "Connect MCP — tool list"
  registered with real pydantic-validated input schemas, real docstrings,
  and a stubbed OHDC body. Tools surface `OhdcNotWiredError` (subclass of
  `NotImplementedError`) when called, with a pointer back to this file.
- `python -m ohd_connect_mcp` starts the FastMCP server on stdio (default)
  or Streamable HTTP (`OHD_MCP_TRANSPORT=http`).

## What's stubbed — OHDC client

`src/ohd_connect_mcp/ohdc_client.py` defines `OhdcClient` with one method
per OHDC RPC the Connect MCP needs (per `connect/SPEC.md` "OHDC client
surface Connect needs"). Every method raises `OhdcNotWiredError` for v0.

**Wire-up integration point** for the next agent:

1. Generate Python Connect-RPC stubs from
   `../../storage/proto/ohdc/v0/*.proto` into
   `src/ohd_connect_mcp/_gen/ohdc/v0/` (likely with `buf generate` plus
   `protoc-gen-connect-python` or a Connect-RPC Python runtime; pin the
   choice in this STATUS.md when made).
2. Replace each method body in `ohdc_client.py` with a real RPC call,
   attaching `Authorization: Bearer {self._config.access_token}`.
3. Translate generated message types to the dict shape documented in the
   tool docstrings (so the LLM-facing surface stays stable).
4. Keep `OhdcNotWiredError` as the surface for any tools that don't yet
   have RPC coverage (e.g. `find_patterns` needs Aggregate + statistical
   post-processing; `chart` needs a renderer).

The tool implementations in `tools.py` already build the OHDC request
shapes and call the client; once the client is wired, the tools work
end-to-end with no further edits.

## Pinned versions

| Package | Version | Why |
|---|---|---|
| Python | `>=3.11` | Modern type hints; matches `care/cli`. |
| `fastmcp` | `>=2.11,<4` | Standalone FastMCP — currently 3.2.4 on PyPI; supports both stdio and Streamable HTTP, has the in-process `Client` test harness used by `tests/test_tools.py`. The `>=2.11,<4` floor lets us absorb 3.x patch releases without surprise major bumps. |
| `pydantic` | `>=2.7` | Tool input/output schemas; FastMCP's hard dep. |
| `anyio` | `>=4.4` | FastMCP's async runtime. |

## Smoke test

```bash
cd connect/mcp
uv sync
uv run pytest
uv run python -c "from ohd_connect_mcp.server import build_server; print(len(build_server()._tool_manager._tools))"
```

The third line should print `27`. (Internal attribute; just a sanity check
during scaffolding.)

## Next steps

1. **Wire up OHDC client** — see "What's stubbed" above.
2. **Time parsing** — `tools.py:_resolve_ts` currently only accepts ISO
   8601. Add `dateparser` for "yesterday", "30 minutes ago", "last
   Tuesday" per `spec/docs/research/mcp-servers.md` "Handling time input".
3. **OAuth proxy** — for the remote Streamable HTTP transport, hook
   FastMCP's `OAuthProxy` against the OHD Storage OIDC providers per
   `connect/SPEC.md` "MCP (`ohd-connect-mcp`)".
4. **Claude Desktop install helper** — a `claude mcp add` shim that
   acquires a self-session token via OAuth device flow on first run and
   writes the desktop config.
5. **Inspector / `fastmcp dev`** — confirm the FastMCP CLI works against
   `python -m ohd_connect_mcp` for interactive tool poking.

## Out of scope here (intentional)

- Auto-generating tools from the FastAPI app (the original spec note) —
  storage is Rust + Connect-RPC now. Hand-written intent-shaped tools win
  for LLM ergonomics anyway.
- The CLI form factor (`connect/cli/`) — that's Rust + clap; lives in a
  separate component dir.
