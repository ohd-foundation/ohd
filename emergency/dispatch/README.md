# `dispatch/` — OHD Emergency Dispatch Console

> Vite + React + TypeScript SPA. Operator-side web app for the EMS station.
> Dark, dense, CAD/dispatch-console aesthetic — desktop monitor target,
> not the tablet.

See [`STATUS.md`](STATUS.md) for the implementation snapshot and
[`../SPEC.md`](../SPEC.md) §2 "Dispatch console" for the spec.

Shares OIDC, the callback page, and store hooks with `connect/web` and `care/web` via [`@ohd/shared-web`](../../packages/web/ohd-shared-web).

## Run

The repo is a pnpm workspace ([`../../pnpm-workspace.yaml`](../../pnpm-workspace.yaml)). Run `pnpm install` from the repo root once so workspace links resolve.

```bash
pnpm gen        # buf codegen of OHDC proto into src/gen/
pnpm dev        # http://127.0.0.1:5175
```

Mock mode (no storage server required, all pages populated from
`src/mock/`):

```bash
VITE_USE_MOCK=1 pnpm dev
```

Live OHDC mode:

```bash
VITE_STORAGE_URL=https://storage.ems-prague.cz pnpm dev
# then open: http://127.0.0.1:5175/?token=<operator-bearer>
```

## Verify

```bash
pnpm typecheck   # tsc --noEmit
pnpm test        # vitest (7 smoke tests)
pnpm build       # tsc --noEmit && vite build → dist/
```

## Sections (sidebar)

| Route | Purpose |
|---|---|
| `/active` | Live case board: dense table + metric strip + map placeholder + detail drawer. Default. |
| `/roster` | Crew roster: who's on duty, who has a current case, contact, on/off-duty toggle. |
| `/audit` | Filterable break-glass / operator-action log. (Storage `AuditQuery` is stubbed; banner says so.) |
| `/records` | Operator-side records DB browser (mock). CSV export. |
| `/settings` | Storage URL, operator token, relay URL, station label, authority cert info, push provider. |

## Auth

Operator-session bearer (distinct from care/web's grant token), sent as
`Authorization: Bearer <token>` on every OHDC call. Token resolution order:

1. `?token=…` on the URL.
2. `localStorage` (persists across reloads — the dispatch console runs on
   the operator's hardware so longer-lived storage is appropriate).
3. `VITE_DISPATCH_TOKEN` build-time fallback.

Forget the token via the Settings page or by clearing localStorage.

## Layout

See [`STATUS.md` § Layout](STATUS.md#layout).

## Deploy

Bundled into the EMS-station compose stack at [`../deploy/`](../deploy/) — the dispatch SPA's `dist/` is bind-mounted into a Caddy container alongside the relay.

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
