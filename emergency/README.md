# OHD Emergency

> Reference, real, lightweight emergency-services consumer of OHDC. The professional-side counterpart to OHD Care for emergency response — paramedic tablet, dispatch console, Emergency MCP, and CLI.

The canonical component spec is in [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md); local copies of the directly relevant design docs live in [`./spec/`](spec/).

## What's here

| Path | Contents |
|---|---|
| [`SPEC.md`](SPEC.md) | Implementation-ready spec, keyed to the four sub-projects below. |
| [`STATUS.md`](STATUS.md) | Per-sub-project state, blockers, current implementation pickup. |
| [`spec/`](spec/) | Snapshot copies of `emergency-trust.md`, `screens-emergency.md`, `mcp-servers.md`. |
| [`tablet/`](tablet/) | Android (Kotlin / Jetpack Compose) paramedic-tablet app. iOS deferred. |
| [`dispatch/`](dispatch/) | Vite + React + TypeScript operator dispatch console (web). |
| [`mcp/`](mcp/) | Python + FastMCP Emergency MCP server. |
| [`cli/`](cli/) | Rust `ohd-emergency` CLI. |
| [`deploy/`](deploy/) | Docker Compose + Caddyfile reference deployment for an EMS station. |

## How OHD Emergency fits the global system

```
    ┌──────────────────────────────────┐
    │   OHD Emergency (this repo dir)  │
    │                                  │
    │  ┌──────────────┐ ┌────────────┐ │
    │  │   tablet/    │ │ dispatch/  │ │   Operator's
    │  │  paramedic   │ │  EMS web   │ │   employees,
    │  │   tablet     │ │  console   │ │   on duty
    │  └──────┬───────┘ └─────┬──────┘ │
    │         │               │        │
    │         └───────┬───────┘        │
    │                 │                │
    │           ┌─────┴─────┐          │
    │           │   mcp/    │          │
    │           │  triage   │          │
    │           │   LLM     │          │
    │           └─────┬─────┘          │
    │                 │                │
    │                 │ OHDC over      │
    │                 │ HTTP/3         │
    └─────────────────┼────────────────┘
                      │
                      ▼
   ┌──────────────────────────────────────┐
   │   ../relay/  in emergency-authority  │
   │   mode: holds the Fulcio-issued      │
   │   24h authority cert; signs the      │
   │   EmergencyAccessRequest payloads;   │
   │   forwards opaque OHDC bytes to      │
   │   the patient's storage.             │
   └──────────────────┬───────────────────┘
                      │ signed request +
                      │ TLS-tunneled OHDC
                      ▼
   ┌──────────────────────────────────────┐
   │   Patient's OHD Storage (phone /     │
   │   home server / cloud / custom).     │
   │   Verifies the cert chain, shows the │
   │   break-glass dialog (via Connect),  │
   │   issues the case-bound grant.       │
   └──────────────────────────────────────┘
```

## Reference deployment shape

A small EMS station deploying OHD Emergency end-to-end runs (see [`deploy/docker-compose.yml`](deploy/docker-compose.yml)):

| Service | Image / source | Role |
|---|---|---|
| `relay` | `../relay/` (emergency-authority mode) | Holds the org's authority cert; signs emergency requests; forwards OHDC traffic. |
| `dispatch-web` | this dir's `dispatch/` build | Active-case board, crew roster, audit, operator records UI. |
| `postgres-records` | `postgres:16` | Operator-side records DB (separate from patient OHD). |
| `caddy` | `caddy:2` | TLS termination + HTTP/3 + reverse proxy. |

Plus paramedic tablets connecting from the field over the public internet to the relay's domain. See [`deploy/README.md`](deploy/README.md).

## What OHD Emergency does NOT do

(Restated from the spec for fast onboarding — the canonical list is in [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) "What OHD Emergency does NOT do".)

- Not a CAD platform. Vehicle dispatch / GPS routing / resource allocation stays in the operator's existing CAD product.
- Not a NEMSIS / HL7 reporter. Could be added as a sidecar against the operator records DB; not in scope here.
- Not a hospital ER information system. The receiving ER uses OHD Care or its own EHR; Emergency hands the case off and transitions out.
- Not an HR / scheduling / billing system.
- Not the relay. The relay binary lives in `../relay/`; this directory is the relay's emergency-authority-mode consumer.

## Build / test per sub-project

| Sub-project | Build | Test |
|---|---|---|
| `cli/` | `cargo build` | `cargo test` |
| `tablet/` | `./gradlew :app:assembleDebug` (after [`tablet/BUILD.md`](tablet/BUILD.md) Stages 1–3) | `./gradlew :app:test` |
| `dispatch/` | `pnpm install && pnpm build` (workspace) | `pnpm test` |
| `mcp/` | `uv sync` | `uv run pytest` |
| `deploy/` | `docker compose -f deploy/docker-compose.yml config` | (see [`deploy/README.md`](deploy/README.md)) |

The Python MCP shares helpers with `connect/mcp` and `care/mcp` via [`ohd-shared`](../packages/python/ohd-shared); the dispatch web SPA shares OIDC and store hooks with `connect/web` and `care/web` via [`@ohd/shared-web`](../packages/web/ohd-shared-web).

## Deploy

The reference EMS-station compose stack lives in [`deploy/`](deploy/) — relay (in emergency-authority mode), dispatch SPA, Postgres-records, Caddy. See [`deploy/README.md`](deploy/README.md).

For native packages, [`../PACKAGING.md`](../PACKAGING.md) covers `ohd-emergency` (CLI).

## License

Same as the rest of OHD: dual-licensed Apache-2.0 OR MIT. See [`../spec/LICENSE`](../spec/LICENSE).
