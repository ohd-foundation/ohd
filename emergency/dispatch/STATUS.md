# dispatch/ — Status

Snapshot of the OHD Emergency dispatch console.

## OHDC wire/API version renamed to v0 (2026-05-09)

Dispatch Buf/codegen references now target `src/gen/ohdc/v0/` and the
`ohdc.v0` storage API package.

## Phase

**v0 implementation built.** Buildable, typechecked, tested. Talks real
OHDC by default; `VITE_USE_MOCK=1` flips to a mock backend for visual
review without storage.

## What works

| Area | Status |
|---|---|
| Vite + React 18 + TS scaffold | done |
| `react-router-dom` v6 routing (5 sections) | done |
| Connect-Web v2 client to OHDC over Bearer auth | done |
| `pnpm gen` (buf v2 → `protoc-gen-es`) into `src/gen/ohdc/v0/` | done |
| Mock backend toggleable via `VITE_USE_MOCK=1` | done |
| Active cases page: live table, metrics, map placeholder, detail drawer | done |
| Crew roster page (mock data) | done |
| Audit page with TBD banner (`AuditQuery` RPC stubbed in storage) | done |
| Operator records page (mock data + CSV export) | done |
| Settings page (storage URL, token, station, cert, push) | done |
| Vitest smoke tests (7 passing) | done |
| Production build | done |
| Dark dense CAD-style theme | done |

## OIDC wired (mirroring connect/web pattern) — 2026-05-09

Operator-OIDC sign-in via OAuth 2.0 Authorization Code + PKCE landed. Files:

- `src/ohdc/oidc.ts` — `oauth4webapi`-driven discovery (RFC 8414 with
  fallback to `/openid-configuration`), `beginLogin` /
  `completeLogin` / `refreshIfNeeded` / `clearSession`.
- `src/pages/LoginPage.tsx` — issuer / client-id / redirect-URI form.
- `src/pages/OidcCallbackPage.tsx` — receives `?code=&state=`, exchanges
  for tokens, mirrors the bearer into the existing
  `ohd-dispatch-operator-token` localStorage key so
  `ohdc/client.ts` picks it up unchanged.
- `App.tsx` — `/login` + `/oidc-callback` routes wired.
- `components/AppShell.tsx` — top-bar renders the OIDC operator name
  (from the id_token `name` / `preferred_username` claim) with a
  Sign-out button that clears the session and routes to `/login`.

Build-time defaults: `VITE_OIDC_ISSUER` / `VITE_OIDC_CLIENT_ID` /
`VITE_OIDC_REDIRECT_URI` / `VITE_OIDC_SCOPE` / `VITE_STORAGE_URL`.
Difference from connect/web: dispatch persists the bearer to
**localStorage** (operator hardware) rather than sessionStorage. The
paste-token UX on Settings → Storage continues to work as a fallback.

Tests: 8 smoke tests pass (`pnpm test`); typecheck + production bundle
both clean (`pnpm typecheck`, `pnpm build`).

## What's NOT done (explicit TBDs)

| TBD | Notes |
|---|---|
| Real-time updates | v0 polls every 5s. Websocket / SSE in v0.x. |
| Map | Placeholder grid. v0.x will plot scene GPS via Leaflet/MapLibre. |
| `OhdcService.AuditQuery` wiring | Storage returns `NOT_IMPLEMENTED` today; UI shows mock + banner. Single store-fn swap once it ships. |
| Operator records DB | Postgres schema not landed. Mock for now; real connector in v0.x. |
| Reopen-token issuance | Per SPEC §2.3 the issuance endpoint is on the relay; UI button currently surfaces a TBD alert. |
| Force-close authority semantics | UI calls `OhdcService.CloseCase` and shows the spec caveat ("only the patient can fully revoke"). Storage's authority enforcement detail is being tightened in `emergency-trust.md`. |
| Crew roster sync | Currently mock; relay's roster-sync endpoint is the source of truth. |
| Authorize new responder | Stub button; operator IdP flow lands with the relay. |
| Push provider config | Stub UI; secrets live in the relay env. |
| Operator-side fields on `CaseRow` | `opening_responder` / `destination` / `scene_note` come from the operator DB; OHDC `Case` doesn't carry them. The store joins client-side once the records connector ships. |

## Run / verify

```bash
pnpm install
pnpm gen        # buf codegen → src/gen/ohdc/v0/
pnpm typecheck
pnpm test       # 7 smoke tests
pnpm build      # production bundle in dist/
pnpm dev        # http://127.0.0.1:5175 (avoids 5173/5174 used by care/connect)
```

Mock-only review (no storage server):

```bash
VITE_USE_MOCK=1 pnpm dev
# → http://127.0.0.1:5175 — all pages fully populated from src/mock/
```

Live OHDC mode:

```bash
VITE_STORAGE_URL=https://storage.ems-prague.cz pnpm dev
# Then open: http://127.0.0.1:5175/?token=<operator-bearer>
# The token persists to localStorage; settings page lets you replace it.
```

## Layout

```
dispatch/
├── package.json          # pnpm workspace member
├── buf.gen.yaml          # buf v2 codegen
├── tsconfig.json
├── vite.config.ts        # port 5175
├── index.html
├── README.md
├── STATUS.md             # this file
└── src/
    ├── App.tsx           # routes + bootstrap gate
    ├── main.tsx
    ├── index.css         # dark dense CAD-style theme
    ├── types.ts          # CaseRow / CrewMember / AuditRow / OperatorSession
    ├── util.ts           # ULID, time formatting, CSV
    ├── components/       # AppShell, DataTable, StatusChip, MetricTile, TimelineFeed
    ├── pages/            # ActiveCasesPage, CrewRosterPage, AuditPage, OperatorRecordsPage, SettingsPage
    ├── ohdc/             # client.ts, store.ts, useStore.ts (Connect-Web wiring)
    ├── mock/             # cases.ts, crew.ts, audit.ts, records.ts, store.ts (selector)
    ├── gen/              # buf-generated TS (gitignored)
    └── test/             # setup.ts, smoke.test.tsx
```

## How this fits with `emergency/deploy/`

The reference deployment's `docker-compose.yml` already declares a
`dispatch-web` service. The expectation is:

- `pnpm build` produces `dist/` (a static SPA bundle).
- The `dispatch-web` container serves `dist/` behind Caddy (TLS termination
  on the operator's domain, e.g. `dispatch.ems-prague.cz`).
- Build-time env: `VITE_STORAGE_URL` points at the operator's storage
  endpoint; `VITE_USE_MOCK` is unset in production builds.
- Operator tokens are issued by the relay/IdP and pasted in via
  `?token=…` or the Settings page.
