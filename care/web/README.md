# OHD Care — Web App

Operator-facing SPA. Vite + React + TypeScript. Real OHDC consumer over `@connectrpc/connect-web`; full roster + per-patient view + visit-prep + write-with-approval flow drives the [`../demo/`](../demo/) script.

See [`STATUS.md`](STATUS.md) for the implementation snapshot.

## Stack

- **Vite 5** — dev server, build, HMR.
- **React 18** + **TypeScript 5** — strict mode, JSX transform.
- **react-router-dom 6** — client-side routing.
- **`@connectrpc/connect-web`** — TS OHDC client.
- **`@ohd/shared-web`** — shared OIDC engine, callback page, store hooks ([`../../packages/web/ohd-shared-web`](../../packages/web/ohd-shared-web)).

## Requirements

- Node 20+ (Node 22 LTS recommended)
- pnpm 9+ (the repo is a pnpm workspace — see [`../../pnpm-workspace.yaml`](../../pnpm-workspace.yaml))

## Install + dev

Run `pnpm install` once at the repo root so the `@ohd/shared-web` workspace link resolves. Then:

```sh
# from this directory (care/web/)
pnpm gen           # codegen TS OHDC client from ../../storage/proto/
pnpm dev           # http://localhost:5173
pnpm typecheck
pnpm test          # vitest
pnpm build         # → dist/
pnpm preview       # serves dist/ on http://localhost:4173
```

## Demo

The end-to-end write-with-approval demo lives at [`../demo/`](../demo/). It boots a storage server, seeds patient data, issues a grant token, and prints a `/?token=ohdg_…` URL the SPA opens with.

## Layout

```
web/
├── package.json
├── vite.config.ts
├── tsconfig.json
├── buf.gen.yaml          # TS codegen (`pnpm gen`)
├── index.html
└── src/
    ├── main.tsx          # React entry
    ├── App.tsx           # Router
    ├── ohdc/             # OHDC client + store + OIDC wrapper (uses @ohd/shared-web)
    ├── pages/            # roster, per-patient view, audit, settings
    └── components/
```

## Deploy

For the operator-domain Docker compose stack (Caddy + Care MCP + Postgres + this SPA's `dist/`), see [`../deploy/README.md`](../deploy/README.md).

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
