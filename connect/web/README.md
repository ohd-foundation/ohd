# OHD Connect — Web

Vite + React 18 + TypeScript + react-router-dom + `@connectrpc/connect-web`.
Browser-based personal-side SPA. Talks to a remote OHD Storage instance
under self-session auth.

## Status

**v0.1 implementation pass** — see [`STATUS.md`](STATUS.md) for the full
handoff including which OHDC RPCs are wired, what's stubbed, and the
recommended next steps.

The web client supports **remote-primary** deployments only — browsers
can't host an OHD Storage instance reliably (service workers expire, no
persistent FS, OPFS quotas are tight). Local-first users open Connect
mobile or the desktop CLI.

## Requirements

- Node 20+ (Node 22 LTS recommended)
- pnpm 9+ (the repo is a pnpm workspace — see [`../../pnpm-workspace.yaml`](../../pnpm-workspace.yaml))

## Install / dev / build

Run `pnpm install` from the repo root the first time so workspace links to `@ohd/shared-web` ([`../../packages/web/ohd-shared-web`](../../packages/web/ohd-shared-web)) are wired up. After that:

```bash
# From this directory (connect/web/):

# Codegen the OHDC TS client from ../../storage/proto/:
pnpm gen

# Dev server with HMR on :5174 (avoids care/web's :5173):
pnpm dev

# Type-check (no emit):
pnpm typecheck

# Vitest unit + smoke + page-mount tests:
pnpm test

# Production build → dist/:
pnpm build

# Preview the production build locally:
pnpm preview
```

## Quick demo

```bash
# Start the storage server (in another terminal):
cd ../../storage
cargo run -p ohd-storage-server -- init --db /tmp/ohd-demo.db
cargo run -p ohd-storage-server -- serve --db /tmp/ohd-demo.db \
  --listen 0.0.0.0:8443

# Mint a self-session token:
TOKEN=$(cargo run -p ohd-storage-server -- issue-self-token --db /tmp/ohd-demo.db)

# Start the web app and open with the token:
cd ../connect/web
pnpm dev
# → http://localhost:5174/?token=$TOKEN&storage=http://localhost:8443
```

## Layout

```
web/
├── README.md                                # this file
├── STATUS.md                                # handoff doc
├── buf.gen.yaml                             # TS codegen (`pnpm gen`)
├── index.html
├── package.json                             # pnpm workspace
├── tsconfig.json
├── vite.config.ts                           # :5174 dev port; vitest config
└── src/
    ├── main.tsx                             # React + BrowserRouter entry
    ├── App.tsx                              # 5-tab routing + bootstrap gate
    ├── index.css                            # dark-default palette
    ├── util.ts                              # date / event-type helpers
    ├── components/
    │   ├── AppShell.tsx                     # top bar + sidebar/bottom-bar
    │   ├── Modal.tsx                        # bottom-sheet on mobile
    │   ├── Sparkline.tsx                    # inline SVG, no chart lib
    │   └── Toast.tsx                        # ToastProvider + useToast
    ├── ohdc/
    │   ├── client.ts                        # Connect-Web transport, auth
    │   ├── store.ts                         # snapshot + submit helpers
    │   └── useStore.ts                      # React hooks (Bootstrap, Version)
    ├── pages/
    │   ├── LogPage.tsx                      # quick-entry tile grid
    │   ├── DashboardPage.tsx                # recent + sparklines
    │   ├── GrantsPage.tsx                   # list / create / revoke
    │   ├── PendingPage.tsx                  # review queue
    │   └── settings/
    │       ├── SettingsLayout.tsx           # sub-tab nav
    │       ├── StorageSettingsPage.tsx      # URL + token + status
    │       ├── EmergencySettingsPage.tsx    # 8-section break-glass form
    │       ├── CasesSettingsPage.tsx        # active + closed cases
    │       └── ExportSettingsPage.tsx       # TBD (storage RPC stubbed)
    └── test/
        ├── setup.ts
        ├── smoke.test.tsx                   # App boot
        ├── pages.test.tsx                   # per-page mount tests
        └── store.test.ts                    # ULID codec + selectors
```

## OHDC client

Codegen drops into `src/gen/ohdc/v0/` (gitignored). Run `pnpm gen` whenever
the storage protos at `../../storage/proto/ohdc/v0/` change. The wrapper at
`src/ohdc/client.ts` handles the bearer-token interceptor + Connect-Web
binary transport; `src/ohdc/store.ts` holds the React-facing facade and the
post-write refresh logic.

## Shared web utilities

OIDC, the callback page, and store hooks come from `@ohd/shared-web` ([`../../packages/web/ohd-shared-web`](../../packages/web/ohd-shared-web)) — the same package powers `care/web` and `emergency/dispatch`. Connect-web's `src/ohdc/oidc.ts`, `src/ohdc/useStore.ts`, and `src/pages/OidcCallbackPage.tsx` are thin wrappers over the shared engine.

## Auth

For v0.1, the self-session token is acquired from `?token=ohds_…` on first
load (then stripped from the URL) or from a paste-token field on the Storage
settings page. Both flow into `sessionStorage`. The proper OAuth 2.0
Authorization Code Flow + PKCE per [`../spec/auth.md`](../spec/auth.md) is
the v0.x deliverable — STATUS.md flags the integration points.

## Web Push

Push is per [`../spec/notifications.md`](../spec/notifications.md). The
service worker, manifest, and Web Push registration are not yet wired —
v0.x.

## Barcode scanning

Per [`../spec/barcode-scanning.md`](../spec/barcode-scanning.md). Browser
`BarcodeDetector` API. Not yet wired — v0.x.

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
