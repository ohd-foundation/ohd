# cord-web

Browser SPA frontend for **OHD CORD** — a conversational health-data agent.
Built with Vite + React + TypeScript. No CSS framework; plain CSS in
`src/styles.css`.

The Rust backend (`cord-server`) serves the production build from `dist/` as
static files and exposes the API under `/v1` (plus `/healthz`).

## Prerequisites

- Node.js 20+ and npm.
- For `npm run dev`: a running `cord-server` on `http://127.0.0.1:8446`.

## Setup

```sh
npm install
```

## Development

```sh
npm run dev
```

Starts the Vite dev server (default `http://localhost:5173`). Requests to
`/v1/*` and `/healthz` are proxied to `http://127.0.0.1:8446`, so cookie
sessions and the OIDC redirect flow work against a local backend.

## Production build

```sh
npm run build
```

Type-checks and bundles into `dist/`. `cord-server` serves that directory
directly; no separate static host is required. Preview the built bundle with
`npm run preview`.

## Layout

- `src/api.ts` — typed API client; includes the SSE chat-stream reader.
- `src/auth.tsx` — session context, gates the app on `GET /v1/me`.
- `src/App.tsx` — react-router routes.
- `src/components/` — app shell (`Layout`) and shared UI bits.
- `src/pages/` — Login, New chat, Chat, Sources, Models.

## Notes

- All API calls use `credentials: "include"` for the `cord_session` cookie.
- The chat endpoint currently returns HTTP 501 (the agent ships in a later
  phase); the chat UI detects this and shows a friendly notice instead of
  failing.
