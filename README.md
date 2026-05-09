# Open Health Data (OHD)

A decentralized, user-owned protocol for personal health data — five components, one wire format, three auth profiles.

The protocol spec, architecture, and design docs live in [`spec/`](spec/README.md). The five reference implementations are in this repo and have shipped working scaffolds with end-to-end demos.

## What's here

| Path | What it is |
|---|---|
| [`spec/`](spec/) | The protocol spec — architecture, components, data model, storage format, deployment modes, design files. The canonical source of truth. |
| [`storage/`](storage/) | OHD Storage — Rust core (SQLite + SQLCipher), OHDC server, on-disk format. Cargo workspace; `.proto` schemas live here at [`storage/proto/ohdc/v0/`](storage/proto/ohdc/v0/). |
| [`connect/`](connect/) | OHD Connect — personal app: Android (Kotlin/Compose), iOS (Swift), web (Vite/React), CLI (Rust), MCP (Python/FastMCP). |
| [`care/`](care/) | OHD Care — reference clinical app: web SPA (Vite/React), Python CLI, Python/FastMCP MCP server, Docker deploy. |
| [`emergency/`](emergency/) | OHD Emergency — paramedic tablet (Android), dispatch console (web), MCP (Python/FastMCP), CLI (Rust), Docker deploy. |
| [`relay/`](relay/) | OHD Relay — Rust binary that forwards opaque packets between OHDC clients and unreachable storage; optional emergency-authority mode. |
| [`packages/python/ohd-shared/`](packages/python/ohd-shared/) | Shared Python helpers (`OhdcTransport`, canonical query hash, OAuth proxy, generated proto stubs). Consumed by `care/cli`, `care/mcp`, `connect/mcp`, `emergency/mcp`. |
| [`packages/web/ohd-shared-web/`](packages/web/ohd-shared-web/) | Shared TS/React utilities (`oidc`, `OidcCallbackPage`, store hooks). Consumed by `connect/web`, `care/web`, `emergency/dispatch` via pnpm workspaces. |
| [`packaging/`](packaging/) | Cross-binary native-packaging tree: systemd units, .deb metadata, Arch PKGBUILDs. See [`PACKAGING.md`](PACKAGING.md). |
| [`landing/`](landing/) | Static landing page for ohd.dev. |
| [`docker-compose.yml`](docker-compose.yml) | Top-level demo stack (storage + relay). |
| [`PACKAGING.md`](PACKAGING.md) | Native Linux packaging: `.deb`, `.rpm`, Arch PKGBUILD for the five binaries. |
| [`DEPLOYMENT.md`](DEPLOYMENT.md) | Top-level deployment matrix (stub; deploy agent fills it in). |
| [`ux-design.md`](ux-design.md) | UX brief for the user-facing apps. |

Each component dir holds an implementation-ready `SPEC.md`, a `STATUS.md` with current state, a `spec/` subdir with copies of the design docs that component owns, and the implementation tree. The global `spec/` is the canonical source — component-dir specs reference back to it.

## Status

End-to-end demo stack works: a clinician submits a clinical note via Care, the patient approves, the note commits to the patient's file. See [`care/demo/`](care/demo/) for the 11-step flow. Cross-component test count is 600+ across Rust, Python, and TypeScript. Each component's `STATUS.md` tracks per-component progress.

Pinned implementation decisions:

- **Runtime stack** (storage + relay + Rust clients): `connectrpc` + `buffa` (Anthropic's Rust ConnectRPC + Protobuf) over `hyper` (HTTP/1.1 + HTTP/2) and `quinn` + `h3` (HTTP/3, in-binary). `rusqlite` + `sqlcipher` for encrypted storage.
- **MCP servers** (Connect, Care, Emergency): Python + [FastMCP](https://github.com/jlowin/fastmcp) per [`spec/docs/research/mcp-servers.md`](spec/docs/research/mcp-servers.md). All three share `ohd-shared`.
- **Web SPAs** (connect/web, care/web, emergency/dispatch): Vite + React + TypeScript, consuming `@ohd/shared-web` via pnpm workspaces.
- **Wire version**: `ohdc.v0` — protos at [`storage/proto/ohdc/v0/`](storage/proto/ohdc/v0/), all clients codegen against them.

## Running the demo stack

The top-level Docker Compose brings up the two daemons (storage + relay):

```bash
cp .env.example .env
docker compose up --build -d
docker compose logs -f storage relay
```

This exposes `ohd-storage` on `:8443` and `ohd-relay` on `:9443` (TCP + UDP). After the stack is up, see [`storage/deploy/README.md`](storage/deploy/README.md) for the database init + token issuance flow.

For the full end-to-end write-with-approval demo (storage + Connect CLI + Care web SPA + pending approval flow), see [`care/demo/README.md`](care/demo/README.md).

For deployment recipes per component (or to deploy `care/web` against an existing storage), each component dir has its own `deploy/` with a focused compose file. See also:

- [`PACKAGING.md`](PACKAGING.md) — native Linux packages (.deb / .rpm / Arch) for the five binaries.
- `DEPLOYMENT.md` — top-level deployment guidance (operator-facing; populated alongside the deploy agent's work).

## Running the workspace

The repo is a pnpm workspace ([`pnpm-workspace.yaml`](pnpm-workspace.yaml)) for the three web SPAs and `@ohd/shared-web`, plus a Cargo workspace under [`storage/`](storage/), and Python projects per component.

```bash
# Install all web workspace deps in one pass:
pnpm install

# Per component:
cd connect/web    && pnpm dev      # :5174
cd care/web       && pnpm dev      # :5173
cd emergency/dispatch && pnpm dev  # :5175

# Rust:
cd storage   && cargo build
cd relay     && cargo build
cd connect/cli   && cargo build
cd emergency/cli && cargo build

# Python (uv):
cd care/cli      && uv sync
cd care/mcp      && uv sync
cd connect/mcp   && uv sync
cd emergency/mcp && uv sync
```

Each component README has the focused recipe.

## Testing

Each component runs its own test suite. Quick reference:

| Component | Command |
|---|---|
| `storage/` | `cargo test --workspace` |
| `relay/` | `cargo test` |
| `connect/cli/` | `cargo test` |
| `connect/web/` | `pnpm test` (vitest) |
| `connect/mcp/` | `uv run pytest` |
| `care/cli/` | `uv run pytest` |
| `care/web/` | `pnpm test` |
| `care/mcp/` | `uv run pytest` |
| `emergency/cli/` | `cargo test` |
| `emergency/dispatch/` | `pnpm test` |
| `emergency/mcp/` | `uv run pytest` |
| `packages/python/ohd-shared/` | `uv run pytest` |

Total cross-project: 600+ tests as of this writing.

## Where to start

- [`spec/README.md`](spec/README.md) — protocol overview and doc index.
- [`spec/docs/01-architecture.md`](spec/docs/01-architecture.md) — how the components fit together.
- [`spec/docs/deployment-modes.md`](spec/docs/deployment-modes.md) — where the data lives (on-device, self-hosted, custom provider, OHD Cloud).
- [`spec/docs/components/`](spec/docs/components/) — per-component specs.
- [`spec/docs/glossary.md`](spec/docs/glossary.md) — every term defined once.
- Component `SPEC.md` and `STATUS.md` files for implementation details.

## Contributing

We welcome contributions. The norms (data ownership, portability, audit-by-default) are in [`spec/SPIRIT.md`](spec/SPIRIT.md). Open an issue or PR against the relevant component dir. Cross-component changes (e.g. proto schema bumps) start in [`storage/proto/`](storage/proto/) and ripple out via the codegen pipeline; touch every affected component's STATUS.md.

Component-specific conventions live in each component's `STATUS.md`.

## License

Dual-licensed under your choice of:

- **Apache License, Version 2.0** ([`spec/LICENSE-APACHE`](spec/LICENSE-APACHE))
- **MIT License** ([`spec/LICENSE-MIT`](spec/LICENSE-MIT))

See [`spec/LICENSE`](spec/LICENSE) for the dual-license arrangement and the `Apache-2.0 OR MIT` Rust convention this follows.
