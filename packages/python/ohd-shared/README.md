# ohd-shared

Shared Python helpers used by the OHD Care CLI and the three MCP servers
(Care, Connect, Emergency). Extracted to remove the ~1500 LOC of byte-identical
copies that previously lived in each consumer.

## What's here

- `ohd_shared.connect_transport` — hand-rolled Connect-RPC over HTTP/2
  client (`OhdcTransport`, `OhdcRpcError`). Async / `httpx`-based; used by
  every async MCP `ohdc_client.py`.
- `ohd_shared.canonical_query_hash` — byte-for-byte mirror of storage's
  `pending_queries::enqueue` query-hash algorithm. Single source of truth
  for the operator-side audit JOIN per `care/SPEC.md` §7.3.
- `ohd_shared.ohdc_helpers` — proto <-> dict translation helpers used by
  the MCP `OhdcClient` implementations: ULID Crockford codec,
  `event_to_dict`, `event_input_from_dict`, `channels_from_dict_data`,
  `pending_to_dict`, `case_to_dict`, `grant_to_dict`, `audit_to_dict`,
  `put_result_to_dict`, `build_filter`. The helpers operate on the
  generated `ohdc.v0.ohdc_pb2` module bundled here at
  `ohd_shared/_gen/ohdc/v0/`.
- `ohd_shared.oauth_proxy` — `build_oauth_proxy(config)` plus the OIDC
  metadata `discover()` helper. Optional dep on `fastmcp` (extra `oauth`).

## Layout

```
packages/python/ohd-shared/
├── pyproject.toml
├── README.md
├── scripts/regen_proto.sh           # single source for `_gen/ohdc/v0/`
└── src/ohd_shared/
    ├── __init__.py
    ├── connect_transport.py
    ├── canonical_query_hash.py
    ├── ohdc_helpers.py
    ├── oauth_proxy.py
    └── _gen/ohdc/v0/
        ├── __init__.py
        └── ohdc_pb2.py              # `_gen/` is gitignored
```

## Codegen

The proto stubs in `src/ohd_shared/_gen/ohdc/v0/` are regenerated from
`storage/proto/ohdc/v0/ohdc.proto` by `scripts/regen_proto.sh`. Each
consumer's `pyproject.toml` no longer ships its own `_gen/` copy — it
picks the stubs up via the `ohd-shared` workspace dep.

The CLI (`care/cli`) keeps its own `ohdc_proto/` bundle for now because it
ships the four `auth_pb2`, `ohdc_pb2`, `relay_pb2`, `sync_pb2` modules at
the top of the `ohd_care` package and its lazy-codegen-on-import logic is
specific to single-binary CLI distribution. Consolidating that is a
follow-up.

## Consumer wiring

Consumers add a path-scoped uv source:

```toml
# In each consumer's pyproject.toml:
dependencies = [
    "ohd-shared",
    ...
]

[tool.uv.sources]
ohd-shared = { path = "../../packages/python/ohd-shared" }
```

The relative path varies (`../../../packages/...` for `care/mcp/`,
`care/cli/`, `connect/mcp/`, `emergency/mcp/`).

## Status

- v0.1: extraction landed; `connect_transport`, `canonical_query_hash`,
  `ohdc_helpers`, `oauth_proxy` are the single source. Proto stubs live
  here too; consumers' `_gen/` directories were removed.
- TODO: CLI `ohdc_proto/` lazy-codegen consolidation.
- TODO: dedup the CLI's `OhdcClient` (sync httpx HTTP/2) — its API
  shape is intentionally different from the async MCP clients (sync
  iter, per-request bearer, OperatorAuditEntry stamping).

## Test

```bash
uv run pytest
```

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root. See [`../../../spec/LICENSE`](../../../spec/LICENSE).
