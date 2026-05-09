# OHD Emergency — Status & Implementation Handoff

> Snapshot of where this directory is. Read this first if you're picking up implementation.

## OHDC wire/API version renamed to v0 (2026-05-09)

Emergency CLI, MCP, dispatch, and tablet references now point at the
pre-stable OHDC API namespace `ohdc.v0` where they consume storage proto
artifacts.

## Phase

**Scaffold complete; implementation not started.**

What "scaffold complete" means:

- All four sub-projects (`tablet/`, `dispatch/`, `mcp/`, `cli/`) have buildable / parseable starting points.
- The Rust CLI (`cli/`) builds cleanly and `--help` works (smoke test verified).
- Other sub-projects have placeholder source files and per-subdir READMEs documenting their smoke tests; they have NOT had their package managers run yet.
- Spec docs are mirrored into [`spec/`](spec/) and the implementation-ready spec is in [`SPEC.md`](SPEC.md).
- The reference deployment topology is described in [`deploy/`](deploy/).

## What's scaffolded (file-by-file)

### Top-level
- `README.md` — directory overview, deployment shape, smoke tests.
- `SPEC.md` — implementation-ready spec.
- `STATUS.md` — this file.
- `spec/{README.md, emergency-trust.md, screens-emergency.md, mcp-servers.md}` — local spec snapshots.

### `tablet/`
- `settings.gradle.kts`, `build.gradle.kts`, `app/build.gradle.kts` — Gradle Kotlin DSL for an Android Compose app.
- `app/src/main/AndroidManifest.xml` — declares BLE permissions (`BLUETOOTH_SCAN`, `BLUETOOTH_CONNECT`, fine-location for BLE on older Android), foreground service for active-case cache, internet.
- `app/src/main/java/com/ohd/emergency/MainActivity.kt` — single Compose screen rendering "OHD Emergency v0 — Paramedic".
- `README.md` — what's here, smoke test (`./gradlew :app:assembleDebug`), TBDs.

### `dispatch/`
- **v0 implementation built** (2026-05-09). Full Vite + React + TS SPA with sidebar nav, dark dense CAD-style theme, OHDC Connect-Web client, 5 pages (Active cases / Crew roster / Audit / Operator records / Settings), mock-backend toggle (`VITE_USE_MOCK=1`), 7 passing Vitest smoke tests, clean `pnpm build`. Dev server on port 5175 to stay clear of care/web (5173) and connect/web (5174). Per-area TBDs and run instructions tracked in [`dispatch/STATUS.md`](dispatch/STATUS.md).

### `mcp/`
- **Stack swapped to Python + FastMCP** per the Pinned implementation
  decisions in the repo root `README.md`. The Node + TypeScript scaffold
  was deleted.
- `pyproject.toml` (PEP 621, uv / hatchling).
- `src/ohd_emergency_mcp/` — `server.py`, `tools.py` (all 7 tools from
  SPEC §3.2 plus `list_active_cases` / `set_active_case`), `case_vault.py`
  (real in-memory state machine), `ohdc_client.py` (stub), `config.py`.
- `tests/test_tools.py` — FastMCP `Client` smoke tests; **`uv sync &&
  uv run pytest` passes** (7 tests).
- `README.md`, `STATUS.md` — install / run / test docs and the OHDC
  wire-up integration point.

### `cli/`
- **Real bodies landed (2026-05).** Clap stubs replaced with working
  implementations: `login`, `cert info`, `cert refresh|rotate` (TBD
  pointers), `roster {list,add,remove,status}` (operator-side TOML
  state), `audit {list,export}` (calls `OhdcService.AuditQuery`;
  storage's handler still returns `Unimplemented`), `case-export`
  (calls `GetCase` + `QueryEvents` + best-effort `AuditQuery`, writes
  a portable JSON archive `ohd-emergency.case-export.v1`).
- Wire stack mirrors `connect/cli`: `connectrpc 0.4` + `buffa 0.5`
  generated client over HTTP/2 h2c, plus an in-binary HTTP/3 client
  (`https+h3://`) for QUIC paths.
- See [`cli/README.md`](cli/README.md) for the full subcommand surface
  + archive schema.

### `deploy/`
- `docker-compose.yml` — services `relay`, `dispatch-web`, `postgres-records`, `caddy`.
- `Caddyfile` — TLS + reverse proxy stub for the operator's domain.
- `.env.example` — env var placeholders.
- `README.md` — deployment overview, smoke test (`docker compose config`), TBDs.

## Smoke test results

| Test | Result |
|---|---|
| `cd cli && cargo build && cargo run -- --help` | **PASS** at scaffold time. See [`cli/README.md`](cli/README.md). |
| `cd tablet && ./gradlew :app:assembleDebug` | **NOT RUN.** Requires Android SDK + Gradle wrapper bootstrap. |
| `cd dispatch && npm install && npm run build` | **NOT RUN.** Requires Node toolchain + network access. |
| `cd mcp && npm install && npm run build` | **NOT RUN.** Requires Node toolchain + network access. |
| `cd deploy && docker compose config` | **NOT RUN.** Requires Docker + the relay image (which itself is in `../relay/` and not yet built). |

## What's NOT done — implementation TBDs

These are the real work items. Roughly ordered by what unblocks what.

### Cross-cutting
- **OHDC client library (Kotlin / TS / Rust)** — Emergency depends on the Buf-generated OHDC client. Not in this skeleton; expected to be a sibling library produced by the OHDC layer.
- **Relay's emergency-authority HTTP API** — Emergency calls relay-internal endpoints (`/emergency/initiate`, `/emergency/handoff`, `/emergency/reopen`, `/healthz/cert`). These are not in OHDC; they're relay-private. Spec for them is a TBD owned jointly with `../relay/`.
- **BLE service UUID + characteristic IDs** — open item in [`spec/emergency-trust.md`](spec/emergency-trust.md) "Open items". Tablet and Connect (patient side) need to agree.

### `tablet/`
- All real screens: discovery, break-glass status, patient view, intervention forms, case timeline, handoff. Layouts pinned in [`spec/screens-emergency.md`](spec/screens-emergency.md).
- BLE scan implementation (Android `BluetoothLeScanner`).
- **Operator OIDC integration** — ✅ **OIDC wired (mirroring connect/cli
  pattern)** (2026-05-09).
  - `data/OidcManager.kt` — wraps AppAuth-Android (`net.openid:appauth:0.11.1`).
    Discovery via `AuthorizationServiceConfiguration.fetchFromIssuer`,
    Custom-Tab launch via `ActivityResultLauncher<Intent>`,
    PKCE-protected token exchange.
  - `data/OperatorSession.kt` — bearer + refresh + AppAuth state JSON
    persist via `EncryptedSharedPreferences` (`androidx.security:security-crypto:1.1.0-alpha06`)
    backed by a Keystore-bound AES-256-GCM master key. Falls back to
    plain SharedPreferences with a warning if Keystore is unavailable.
  - `LoginScreen.kt` — replaces the OIDC stub with the real flow:
    `OidcManager.startAuthFlow(activity, launcher, config)` → Custom Tab
    → `OidcManager.handleAuthResult(...)` → `OperatorSession.signInWithOidc(...)`
    → routes to `/discovery`. Stub sign-in kept below the fold for
    offline dev.
  - IdP issuer / client_id / redirect URI come from `BuildConfig`
    defaults set via Gradle `manifestPlaceholders` + `buildConfigField`,
    overridable per build with `-Pohd.emergency.oidc.*` flags.
  - Documented in [`tablet/BUILD.md`](tablet/BUILD.md) "Operator OIDC".
- Optional responder-cert generation + secure-element keystore.
- Offline event queue + flush.
- Panic-logout action.
- iOS port (deferred phase).

### `dispatch/`
- Real React routing (the skeleton uses inline placeholders).
- Backend service for the operator records DB (currently absent — the SPA has no backend yet). Likely a small Node/Fastify or Rust/Axum sidecar; choice deferred.
- **Authentication against operator IdP** — ✅ **OIDC wired (mirroring
  connect/web pattern)** (2026-05-09). `src/ohdc/oidc.ts` (oauth4webapi
  PKCE) + `src/pages/{LoginPage,OidcCallbackPage}.tsx` + routes in
  `App.tsx` + AppShell sign-out button. Bearer mirrors into the
  existing `ohd-dispatch-operator-token` localStorage key so the
  rest of the SPA picks it up unchanged. See [`dispatch/STATUS.md`](dispatch/STATUS.md)
  for the full handoff.
- Active-case board with live updates (server-sent events or WebSocket from the relay).
- Records UI, audit UI, roster UI.
- Reopen-token issuance flow.

### `mcp/`
- [x] All 7 tools registered (5 triage tools + `list_active_cases` /
      `set_active_case`) with pydantic-validated input.
- [x] `set_active_case` state machine + per-tool active-case scoping.
- [x] `OHD_EMERGENCY_MCP_ALLOW_EXTERNAL_LLM` config knob read; surfaced in
      the FastMCP `instructions` string.
- [x] **OHDC client wired (2026-05-08).** Hand-rolled Connect-RPC client
      over `httpx` (mirrors `connect/mcp/`). Narrow real-RPC surface per
      SPEC §3.2: `who_am_i`, `query_events`, `put_events`. The
      higher-level helpers `aggregate`, `find_relevant_context`,
      `check_drug_interaction` remain `OhdcNotWiredError` (storage-side
      classifier / operator-provided dataset still TBD). Codegen via
      `mcp/scripts/regen_proto.sh`. Unit tests use a `MockOhdcClient`;
      integration tests (`-m integration`) spin up
      `ohd-storage-server` end-to-end. See `mcp/STATUS.md`.
- [x] **OAuthProxy + JWTVerifier wired (mirroring connect/mcp pattern)**
      (2026-05-09). `_build_oauth_proxy` discovers the issuer's
      `.well-known/oauth-authorization-server`, picks up the JWKS,
      and wires `JWTVerifier` + `OAuthProxy`. Env vars:
      `OHD_EMERGENCY_OIDC_ISSUER`, `OHD_EMERGENCY_OIDC_CLIENT_ID`,
      `OHD_EMERGENCY_OIDC_CLIENT_SECRET`, `OHD_EMERGENCY_MCP_BASE_URL`.
      5 new unit tests; 16 total passing (`uv run pytest`).
- [ ] Origin allowlist enforcement when `ALLOW_EXTERNAL_LLM=false` (the
      knob is read, but rejection at the transport layer is a wire-up task).
- [ ] Drug-interaction dataset loader (operator-provided artefact).
- [ ] Reopen-token plumbing into `case_vault`.

### `cli/`
- Real implementations of every subcommand. Currently they stub-out and print.
- Wire to: relay's cert-refresh endpoint, operator IdP, operator audit DB, OHD case-export format.
- **Operator OIDC + KMS vault** — ✅ **OIDC wired (mirroring connect/cli pattern)** (2026-05-09).
  - `oidc-login --issuer URL --client-id ID` runs the OAuth 2.0 Device
    Authorization Grant (RFC 8628) via `oauth2 = "5"`. Discovery (RFC
    8414) hits `/.well-known/oauth-authorization-server` with fallback
    to `/openid-configuration`. End-to-end against any compliant issuer.
  - `logout` clears tokens locally; preserves storage URL, station
    label, authority-cert path, and roster path.
  - `--kms-backend auto|keyring|passphrase|none` — default `auto` tries
    the OS keyring (Linux Secret Service / macOS Keychain / Windows
    Credential Manager) via the `keyring` crate; falls back to
    passphrase-derived AES-GCM (Argon2id KDF) on headless machines.
    `OHD_EMERGENCY_VAULT_PASSPHRASE` env var supports CI / Docker.
  - Vault on-disk format is now a JSON envelope wrapping AES-GCM
    ciphertext; legacy plaintext-TOML configs still load (back-compat).
  - Tests: 5 new unit tests (3 KMS round-trip + 2 OIDC discovery JSON
    parse) added; all 29 unit tests still pass (`cargo test`).

### `deploy/`
- Pin the relay image tag (`../relay/` not yet built).
- Real Caddyfile (currently a stub).
- Real Postgres init schema for operator records (currently absent — SPEC.md has a placeholder schema).
- TLS + LUKS / volume-encryption guidance.

### Operator records DB
- Final schema (the SPEC.md placeholder is intentionally provisional).
- Migration tooling (Flyway / sqlx-migrate / etc.; choice deferred).
- Retention-policy enforcement.

## Open design items inherited from the global spec

These are spec-level TBDs that affect Emergency. Not for this directory to resolve; flagging them so implementers don't be surprised.

- **NEMSIS / HL7 bridge** — sidecar against operator records DB. ([`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) "Open design items")
- **Multi-station handoff workflows** when receiving ER is a different operator. UX-only; protocol mechanism (`predecessor_case_id`) exists.
- **Bystander permission UX** — should bystanders see they're forwarding emergency requests? Currently invisible; debated. Not a tablet/dispatch concern, but the tablet may surface "via bystander chain" telemetry to the dispatch UI.
- **Country-CA federation** — long-term governance; affects which trust roots ship.
- **Emergency-revocation deny-list** — rare "kill cert now" lever; spec-level.
- **Patient-side Rekor inclusion-proof verification** — opt-in, off by default; not Emergency-side.
- **Concrete BLE service UUID + characteristics** — needed before tablet BLE scan can be implemented.

## Constraints respected during scaffolding

- Did NOT modify anything outside `/home/jakub/contracts/personal/ohd/emergency/`.
- Did NOT modify `/home/jakub/contracts/personal/ohd/spec/`.
- Did NOT run `npm install`, `gradle`, `docker compose` — only the Rust CLI smoke test.
- Did NOT git add / git commit anything.
- Spec snapshots in [`spec/`](spec/) are `cp` copies of files in `../spec/`; they will drift if the global spec changes. Treat the global spec as the canonical version.
