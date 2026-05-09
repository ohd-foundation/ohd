# OHD Care — Implementation Status

> Handoff from the scaffolding pass to the implementation phase.

## OHDC wire/API version renamed to v0 (2026-05-09)

Care web, CLI, and MCP now reference the storage-owned pre-stable OHDC API as
`ohdc.v0`, including generated stub paths and Connect-RPC service names.

**Phase:** v0.4 — Care/web closes the MCP + audit gaps: chat panel routing LLM tool calls through Care MCP (Streamable HTTP), two-sided audit panel JOINing storage's `AuditQuery` server-stream against the operator-side audit log by `query_hash`, Settings → MCP for runtime config + the `OHD_CARE_NO_PHI_TO_EXTERNAL_LLMS` posture banner.
**Date:** 2026-05-09
**Implementation owner:** TBD

## OHDC wiring pass (2026-05-08)

Care/web now talks to a real `ohd-storage-server` over Connect-Web (binary
Protobuf framing) backed by a single grant token. The pre-existing UI
(roster + per-patient view + write-with-approval modal) didn't change shape;
the in-memory mock is replaced by a thin OHDC client wrapper at
`web/src/ohdc/client.ts` and a snapshot store at `web/src/ohdc/store.ts`.

### What landed

- **TS Connect-RPC codegen pipeline.** `web/buf.gen.yaml` invokes
  `@bufbuild/protoc-gen-es` against `../../storage/proto`; `pnpm gen` writes
  to `web/src/gen/ohdc/v0/` (gitignored). Run once after install. The
  storage's `OhdcService` schema is the single source of truth.
- **OHDC client wrapper** at `web/src/ohdc/client.ts`. Reads `?token=ohdg_…`
  on first load, persists in `sessionStorage`, builds a Connect-Web
  transport, attaches `Authorization: Bearer …` via an interceptor.
  `VITE_STORAGE_URL` overrides the default `http://localhost:18443`.
- **OHDC-backed store** at `web/src/ohdc/store.ts`. Same exported surface
  as the mock (`listPatients`, `getPatientBySlug`, `submitNote`, …) so
  the existing UI components don't change. On bootstrap it calls WhoAmI +
  QueryEvents and projects the events into the per-tab `PatientDetail`
  shape (vitals from glucose/HR/temp, symptoms from `std.symptom`, etc.).
  Writes are optimistic: the UI updates immediately, the OHDC PutEvents
  fires in the background, and a `refresh()` reconciles the snapshot.
- **Toggle.** `mock/store.ts` is now a thin re-exporter that picks the OHDC
  store by default and the original 5-patient mock when
  `VITE_USE_MOCK_STORE=1`. The smoke test sets that flag in
  `test/setup.ts`. `mock/store.fallback.ts` is the verbatim copy of the
  original mock.
- **App-level bootstrap gate.** `App.tsx` calls a `useBootstrap()` hook on
  first render. While bootstrap is in flight, `BootstrapGate` shows
  "Loading patient data from storage…". On `error === "no_token"` the user
  is routed to `/no-grant` with a paste-your-token call to action.
- **End-to-end demo.** `care/demo/run.sh` automates steps 1–5, 9 of the
  11-step write-with-approval flow; `care/demo/README.md` walks through
  the browser interactions. Verified locally: glucose log → grant put →
  `pending_events` row → `pending-approve` → `events` row, ULID preserved.

### Pinned versions

| Package | Version | Why |
|---|---|---|
| `@bufbuild/protobuf` | `^2.2.2` | v2 line — required by `@bufbuild/protoc-gen-es@2`. Encodes via Crockford-style `create(Schema, init)`. |
| `@bufbuild/protoc-gen-es` | `^2.2.2` | Generates TS message types + service descriptors. The Connect plugin (`protoc-gen-connect-es`) was folded into this in v2. |
| `@bufbuild/buf` | `^1.47` | `buf generate` driver. |
| `@connectrpc/connect` | `^2.0.0` | Core client + interceptor types. |
| `@connectrpc/connect-web` | `^2.0.0` | Browser fetch-based transport. Speaks Connect-Protocol (binary or JSON) and gRPC-Web. v0 uses binary Protobuf (matches the Rust server's gRPC test fixture). |

### Storage-side changes (in scope per the wiring brief)

- `ohd-storage-server` got `issue-grant-token`, `pending-list`, and
  `pending-approve` subcommands (see `../storage/STATUS.md`).
- `ohd-storage-server serve` got a `--no-cors` flag; CORS is permissive by
  default in dev. `tower-http` is added as a server-crate dep.
- `std.clinical_note` is bootstrap-seeded into the registry by
  `issue-grant-token` since the canonical migration doesn't include it
  yet. Documented as a v1.x cleanup target.

### What's stubbed / TBD

- **Multi-patient roster.** v0 = one grant = one patient. Multi-grant vault
  with `switch_patient` MCP tool comes when Care holds N grants.
- **`OhdcService.ListPending` / `ApprovePending` wire RPCs.** Care uses the
  storage CLI subcommands directly for the demo — the wire-side handlers
  are still `Unimplemented` per `../storage/STATUS.md`.
- ✅ **Audit transparency panel.** Cross-references patient-side audit
  via `query_hash`. Done in v0.4 — see `care/web/src/pages/AuditPage.tsx`
  and `care/web/STATUS.md`.
- **Operator OIDC + sign-out.** "Sign out" now clears the grant token from
  `sessionStorage` and routes to `/no-grant`; full operator-session flow
  is the next pass.
- **Add-patient share-artifact import.** "Add patient" still alerts
  "deferred". The grant-vault flow needs the share-URL parser + the
  decrypt step (per `spec/care-auth.md`).
- **Reject pending.** No `pending-reject` CLI subcommand yet (next-pass
  pickup; symmetric with `pending-approve`). Web side ships bulk approve
  AND reject in v0.3 — see `web/src/pages/PendingPage.tsx`.

### v0.4 deliveries (2026-05-09)

- ✅ **Two-sided audit panel.** `care/web/src/pages/AuditPage.tsx`
  wires `OhdcService.AuditQuery` (now live in storage per the V1
  backfill agent) and JOINs each storage-side row to the operator's
  local audit by re-hashing `(query_kind, query_params_json)` via
  `canonicalQueryHashFromRawJson`. Filter chips (actor / op-kind /
  time window), red asymmetry pill for storage-only or operator-only
  rows, CSV export of the joined view. Tests at
  `care/web/src/pages/AuditPage.test.tsx` (4 cases).
- ✅ **MCP integration in the web app.** `care/web/src/mcp/client.ts`
  wraps `@modelcontextprotocol/sdk@1.18.0`'s
  `StreamableHTTPClientTransport`; `pages/ChatPage.tsx` is an
  operator-facing chat panel routing the LLM's tool calls through
  Care MCP. The OpenAI-compatible chat-completions client lives at
  `mcp/llm.ts`. Per SPEC §10.6, write tools (`submit_*` + the
  `CASE_MUTATION_TOOLS` set) surface a `Submitting to <patient> —
  confirm?` modal and block dispatch on operator click. Tests at
  `care/web/src/pages/ChatPage.test.tsx` (5 cases).
- ✅ **Settings → MCP page.** `care/web/src/pages/SettingsMcpPage.tsx`
  + `mcp/settings.ts` — runtime `mcpUrl`, `llmUrl`, `llmApiKey`,
  `llmModel`, plus the operator-confirmable
  `OHD_CARE_NO_PHI_TO_EXTERNAL_LLMS` banner. Persisted in
  localStorage; build-time defaults from `VITE_*` env vars.
- ✅ **Care/web STATUS.md.** New `care/web/STATUS.md` file lists
  what's wired and what's still TBD (multi-grant vault inside chat,
  streaming LLM responses, operator tool-permission policy,
  AuditQuery `tail=true` mode).

### v0.3 deliveries (2026-05-09)

- **Canonical `query_hash` across all three Care surfaces.** TS
  (`care/web/src/ohdc/canonicalQueryHash.ts`), Python-cli
  (`care/cli/src/ohd_care/canonical_query_hash.py`), and Python-mcp
  (`care/mcp/src/ohd_care_mcp/canonical_query_hash.py`) all emit the
  same SHA-256 hex for the same `(query_kind, filter)` pair. Verified by
  shared golden vectors at
  `care/web/src/ohdc/__golden__/query_hash_vectors.json`; all three
  test suites assert against the same JSON, so any drift fails CI on
  every side.
- **Operator-side audit log.** Per SPEC §7.2:
  - `care/web/src/ohdc/operatorAudit.ts` — localStorage rolling buffer.
  - `care/cli/src/ohd_care/operator_audit.py` — JSONL under
    `$OHD_CARE_HOME/operator_audit.jsonl`.
  - `care/mcp/src/ohd_care_mcp/operator_audit.py` — JSONL under
    `$OHD_CARE_MCP_AUDIT_DIR` if set, in-memory otherwise.
- **Bulk approve/reject UI.** `care/web/src/pages/PendingPage.tsx` adds
  multi-select, sticky toolbar, confirmation dialog, per-item
  progress, mid-batch error pause-with-continue/abort, success toast,
  and the §6.1 "trust forever" path that adds the event_type to the
  grant's auto-approval allowlist on the first approve call.
- **Care MCP §10.5 case tools.** `open_case`, `close_case`,
  `list_cases`, `get_case`, `force_close_case`,
  `issue_retrospective_grant` registered. See `mcp/STATUS.md`.
- **Optimistic writes for non-clinical-note types.** `submitFood`,
  `submitMedication`, `submitLab`, `submitImaging` update the UI but don't
  call OHDC because the corresponding `std.*` event types aren't wired
  through the demo grant. They log a warning and keep going. A later
  pass aligns the registry + the per-event-type write mappings.

## What exists

| Path | State | Notes |
|---|---|---|
| `README.md` | Done | Overview + deployment shapes + global-spec links. |
| `SPEC.md` | Done | Implementation-ready spec distilled from `spec/docs/components/care.md` + `spec/docs/design/care-auth.md`. |
| `STATUS.md` | This file. | |
| `spec/care-auth.md` | Done (copied) | Mirrors `../spec/docs/design/care-auth.md`. Refresh when the canonical changes. |
| `spec/mcp-servers.md` | Done (copied) | Mirrors `../spec/docs/research/mcp-servers.md`. Care-relevant sections noted in `spec/README.md`. |
| `spec/README.md` | Done | Index pointing to the snapshots and listing which mcp-servers.md sections apply to Care. |
| `web/` | **v0 shell — runnable** | Vite + React + TS SPA; routing, roster, per-patient view (header + brief + 8 tabs), write-with-approval modal with active-patient confirmation. Mock data only — no OHDC wiring. `pnpm install` was run; `pnpm dev` / `test` / `build` / `typecheck` all pass. See "Web app — v0 shell" below for the full status break-down. |
| `mcp/` | **Python + FastMCP** — full tool surface registered | All 20 tools from SPEC §10.1–§10.4 registered against a stubbed OHDC client. Multi-patient grant vault is real (in-memory, seeded from `OHD_CARE_GRANTS_FILE`). `uv sync && uv run pytest` passes. See [`mcp/STATUS.md`](mcp/STATUS.md). |
| `cli/` | Skeleton only | `pyproject.toml` (uv-style); `ohd_care.cli` exposes `patients` / `use` / `temperature` / `submit` subcommands as not-implemented stubs. No installs run. |
| `deploy/` | Reference Compose | `docker-compose.yml` + `Caddyfile` for an operator's domain; `README.md` documents usage. |

## What's NOT done (everything below is for the implementation phase)

### Auth

- [ ] Operator OIDC flow into Care.
- [ ] `care_operator_users` + `care_operator_sessions` schema and persistence.
- [ ] Operator session token issuance (`ohdo_…` prefix) with hashing per the auth.md pattern.
- [ ] Role-based UI gating (`clinician` / `nurse` / `admin` / `auditor`).
- [ ] `Operator.RotateOperatorSession` (refresh).
- [ ] Staff-turnover handler (`active=0` + `revoked_reason='staff_left'`).

### Grant vault

- [ ] `care_patient_grants` schema + CRUD.
- [ ] KMS integration for token encryption-at-rest. v0.1 ship: PBKDF2-from-passphrase local key file (solo-deployment posture); cloud-KMS adapters deferred to per-deployment.
- [ ] `Operator.ImportGrant(share_artifact)` — paste/scan → decrypt → `Auth.WhoAmI` → persist.
- [ ] Grant lifecycle handlers: 401 EXPIRED / 401 CASE_CLOSED / 401 REVOKED / 429.
- [ ] Cache (in-memory + on-disk encrypted) keyed by `(grant_id, query_hash)`; eviction on revocation detection.

### OHDC client

- [ ] Connect-RPC client per language (TS for web + mcp; Python for cli).
- [ ] HTTP/3 with HTTP/2 fallback; TLS cert pin enforcement when `storage_cert_pin` is set.
- [ ] Relay-aware path: rendezvous URL handling; first-call latency UX.

### Web app — v0 shell (runnable, mock-data-only)

Done in this pass (visual review / UX validation; no OHDC wiring yet):

- [x] Vite + React + TS dev/build/test pipeline; `pnpm install` / `dev` / `typecheck` / `test` / `build` all pass.
- [x] Routing skeleton (`react-router-dom` v6):
  - `/` → redirect to `/roster`.
  - `/roster` → roster page.
  - `/patient/:label` → per-patient view; redirects to `/patient/:label/timeline`.
  - `/patient/:label/{timeline,vitals,medications,symptoms,foods,labs,imaging,notes}` → tabs.
- [x] Roster page: card grid, status flags, last-visit, current-meds summary (3 lines max), click-through to per-patient view; "Add patient" stub button.
- [x] Per-patient view: header (active patient label highlighted, grant scope read/write, approval mode, expiry, active-case banner), visit-prep brief, all 8 tabs rendering varied mock data (sparkline for vitals, adherence for meds, severity table for symptoms, lab panels w/ flags, etc.).
- [x] **Active-patient safety (SPEC §3.3)**: top bar shows the active patient label on every screen; per-patient header repeats the label inside an accent-bordered band; submission modal has a confirmation step echoing the label ("Submitting to <label> — confirm?").
- [x] Write-with-approval modal for all event types (note, vital, symptom, food, medication, lab, imaging) with per-grant approval-mode awareness ("auto-commit" vs "queues for approval"); submissions append to the in-memory mock store and surface a toast.
- [x] Mock data: 5 patients with varied state — Alice (active flags), Pavel (no recent activity, grant expiring), Marta (active EMS handoff case), Jiri (patient-curated case grant), Eva (vanilla active grant).
- [x] Smoke test: `vitest run` boots `<App>` against `MemoryRouter`, asserts roster has 5 patient cards.

### Web app — what's still stubbed for the OHDC-wiring phase

- [ ] Real OHDC client (Connect-RPC) replacing the mock store at `src/mock/store.ts`.
- [ ] Operator OIDC + session token; "Sign out" is currently a stub `alert()`.
- [ ] Grant-vault import: paste/scan share artifact → decrypt → `Auth.WhoAmI` → persist. The "Add patient" button is currently a stub `alert()`.
- [x] Audit transparency panel (cross-references patient-side audit via `query_hash`). Done in v0.4 — `care/web/src/pages/AuditPage.tsx` JOINs the storage-side `OhdcService.AuditQuery` server-stream to the operator-side audit log by re-hashing `(query_kind, query_params_json)` and falling back to `(action, ts ± 5s)` for non-pending-query rows.
- [x] LLM chat panel routing tool calls through Care MCP. Done in v0.4 — `care/web/src/pages/ChatPage.tsx` over the Streamable HTTP MCP transport, with the SPEC §10.6 `confirm=True` write guard surfaced as an in-UI confirm modal.
- [x] Settings → MCP page (operator-configured `mcp_url`, `llm_url`, `llm_api_key`, `llm_model`, plus the `OHD_CARE_NO_PHI_TO_EXTERNAL_LLMS` posture banner). Done in v0.4 — `care/web/src/pages/SettingsMcpPage.tsx`.
- [ ] Case operations UI (open / close / handoff / reopen-from-token). The active-case banner renders today, but the actions are not wired.
- [ ] Roster search / filter — small dataset for v0; not painful yet.
- [ ] Mobile-responsive layout — desktop-first per the v0 brief.
- [ ] Localization — English-only per SPEC §13.

### Care MCP

- [x] All 20 tools per SPEC §10.1–§10.4 registered with pydantic-validated
      input and real docstrings. (§10.5 case tools deferred — see
      `mcp/STATUS.md`.)
- [x] Active-patient orientation surfaced in every tool result + the
      FastMCP server's `instructions` string.
- [x] Confirmation guard on every `submit_*` tool (`confirm=True` required;
      raises `PermissionError` otherwise).
- [x] Multi-patient grant vault as a real in-memory state machine.
- [x] **OHDC client wired (2026-05-08).** Hand-rolled Connect-RPC client
      over `httpx` (mirrors `connect/mcp/`). `who_am_i`, `query_events`,
      `get_event_by_ulid`, `put_events`, `list_pending` are real RPCs;
      `aggregate`, `correlate`, `find_relevant_context` remain
      `OhdcNotWiredError` because the storage handlers are
      `Unimplemented` in v1. Codegen via
      `mcp/scripts/regen_proto.sh`. Unit tests use a `MockOhdcClient`;
      integration tests (`-m integration`) spin up
      `ohd-storage-server` end-to-end. See `mcp/STATUS.md`.
- [ ] FastMCP `OAuthProxy` wiring against the operator IDP.
- [ ] Encrypted at-rest grant storage (deployment KMS).
- [ ] §10.5 case tools (`open_case`, `close_case`, `handoff_case`,
      `list_cases`).

### CLI

- [ ] Browser-OAuth flow into `~/.config/ohd-care/` for operator session.
- [ ] `patients` listing.
- [ ] `use <label>` switches active grant in local state.
- [ ] `temperature --last-72h` etc. (read tools).
- [ ] `submit observation` / `submit clinical-note` etc. (write tools with confirmation prompt).
- [ ] `pending list`.

### Audit

- [ ] `care_operator_audit` schema + writer wired into every OHDC dispatch.
- [ ] `query_hash` canonicalization matches OHDC's hash on the patient side (so two-sided join works).
- [ ] Compliance-export query path (CLI).

### Deploy

- [ ] Real Care storage image (the SPA + MCP + CLI bundle / separate images).
- [ ] Caddyfile route plan: `/` → web; `/mcp/care/*` → MCP; `/api/operator/*` → operator-only endpoints.
- [ ] Optional Postgres for operator-side state (current default: SQLite in a volume).

## Decisions to flag for the implementation phase

These are choices made during scaffolding that the implementer should validate:

1. **CLI language: Python.** `pyproject.toml` is uv-style (`[project]` with PEP 621 metadata). Click is the chosen CLI framework (vs. argparse) for rich help and composability. If the implementer prefers Typer (also Click-based), the swap is mechanical.

2. **MCP language: Python + FastMCP.** Pinned at the repo level — see the
   Pinned implementation decisions in the root `README.md`. The MCP scaffold
   originally proposed TypeScript on Node so it could share the OHDC TS
   client with the web build, but FastMCP's pydantic-validated tool schemas
   and in-process `Client` test harness give a cleaner LLM-facing surface
   than `@modelcontextprotocol/sdk`. The grant-vault wiring **is**
   reimplemented in Python in `mcp/src/ohd_care_mcp/grant_vault.py`; there's
   no shared cross-language vault yet.

3. **Web stack: Vite + React + TS, `pnpm` documented.** No state library is locked in; default to React Context + custom hooks, evaluate Zustand or TanStack Query as the data layer when Care MCP-style multi-query views land. `react-router-dom` is the router.

4. **No shared client library yet.** The grant-vault and OHDC dispatch are reimplemented in each form (web/TS, mcp/Py, cli/Py). When the third reimplementation starts diverging, extract a shared TS+Py client. Premature now.

5. **KMS posture for v0.1: PBKDF2-from-passphrase local key file.** The solo-practitioner / single-clinic deployment shape this targets. Cloud-KMS / HSM adapters layer in per-deployment when those deployments materialize.

6. **System DB: SQLite in v0.1.** Postgres adapter behind the same schema is a deployment concern; defer until a deployment requires it.

7. **No build/install run during scaffolding.** Per the scaffolding contract — `pnpm install`, `npm install`, `pip install` were all skipped. Per-form READMEs document the commands.

### Web v0-shell decisions

8. **Dependency-light UI.** No Material / Antd / Chakra; plain CSS in `src/index.css` plus a small set of primitives (`.btn`, `.card`, `.flag`, `.modal`, `.tabs`, `.data-table`, `.sparkline`). System font stack. Aesthetic per `../ux-design.md` (clean, restrained, type-driven, black/white/red palette).

9. **Mock store is module-level mutable state** at `src/mock/store.ts`. Submissions append in-place; reset on page reload. No `localStorage`. When OHDC wiring lands, this module is replaced by a thin wrapper around the OHDC client + grant vault — the call sites (`listPatients`, `getPatientBySlug`, `submitNote`, etc.) are designed as the interface boundary.

10. **Mock-data shape mirrors target types.** `src/types.ts` defines `PatientSummary`, `PatientDetail`, `VitalReading`, `MedicationEntry`, `SymptomEntry`, `FoodEntry`, `LabResult`, `ImagingStudy`, `ClinicalNote`, `TimelineEvent`. These lean toward what OHDC will return so the swap to real data is a re-implementation of the store rather than a refactor of the UI.

11. **`tsc --noEmit` for typechecking; vite owns transpile.** The build script is `tsc --noEmit && vite build` — no `tsc -b` (which would emit `.js` next to sources). `tsconfig.json` sets `"noEmit": true` and a per-package `.gitignore` defends against any stray emit.

12. **Vitest for tests; jsdom environment.** A single smoke test that boots `<App>` and asserts roster has 5 patient cards. `@testing-library/react` + `@testing-library/jest-dom` for matchers.

## What's blocked / TBD

- **Real OHDC client codegen.** The OHDC `.proto` files and the Connect-RPC codegen pipeline are owned by the Storage component. Care depends on those landing before any real OHDC call works. v0.1 stubs print "not yet implemented" so Care can be built / smoke-tested standalone.
- **Operator-grant-request flow** (path 3 in §2.5 of SPEC.md) — deferred to v2 (`future-implementations/operator-grant-request.md` in the global spec, not yet written).
- **OAuth proxy wiring for Care MCP**. FastMCP 3.x ships an `OAuthProxy` that
  delegates to OIDC providers; needs to be hooked against the operator IdP
  (Okta / Keycloak / Authentik) for the remote Streamable HTTP deployment.
- **Two-sided audit `query_hash` canonicalization.** Has to match the storage side's hash byte-for-byte. The exact canonicalization rules live with the OHDC spec (`spec/docs/design/ohdc-protocol.md`); when those land, mirror in Care's audit writer.
- **Cohort grants, operator-to-operator warm handoff, localization** — all called out as open / deferred per SPEC §15.

## How to pick this up

1. Read `README.md`, then `SPEC.md` end-to-end.
2. Read `spec/care-auth.md` (full schemas, lifecycle rules).
3. Read the canonical Care component spec at `../spec/docs/components/care.md`.
4. Pick a form to start with — recommended order: `cli/` (smallest surface area, exercises auth + grant vault end-to-end) → `web/` (the primary user-facing form) → `mcp/` (depends on patterns from the other two).
5. Stand up a local OHD Storage instance (the Storage component scaffolding lives at `../storage/`); use that to exercise OHDC calls during development.
6. Update this `STATUS.md` as features land.
