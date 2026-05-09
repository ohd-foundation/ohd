# `@ohd/shared-web`

Shared TS/React utilities for the three OHD web SPAs:

- `connect/web` — `@ohd/connect-web`
- `care/web` — `@ohd/care-web`
- `emergency/dispatch` — `@ohd/emergency-dispatch`

The package is consumed via pnpm's workspace protocol (`"@ohd/shared-web": "workspace:*"`). It ships raw `.ts` / `.tsx` — there's no build step. Each SPA's Vite + tsc transpile the sources directly.

The package was extracted from three near-identical copies of `oidc.ts`, `OidcCallbackPage.tsx`, and `useStore.ts` that lived in each SPA's `src/ohdc/` and `src/pages/` trees. After the extraction those files are thin SPA-flavoured wrappers that delegate to the engine here while preserving their existing public API (so call sites in the SPAs are untouched).

## Layout

```
src/
├── index.ts              # Re-exports everything below
├── oidc.ts               # OAuth Code+PKCE engine (~430 LOC, the bulk)
├── OidcCallbackPage.tsx  # Generic callback handler component
├── useExternalStore.ts   # useBootstrap / useStoreVersion / usePoll
└── store-types.ts        # The Store<T> contract those hooks bind to
```

## API

### `oidc.ts` — generic OAuth 2.0 Code+PKCE flow

```ts
import {
  beginLogin,
  completeLogin,
  loadSession,
  saveSession,
  clearSession,
  refreshIfNeeded,
  type OidcOptions,
  type OidcSession,
} from "@ohd/shared-web/oidc";
```

`OidcOptions` captures every legitimate point of divergence between the three callers:

| Field                    | Purpose                                                                                  |
| ------------------------ | ---------------------------------------------------------------------------------------- |
| `issuer`                 | AS root URL (storage-as-AS for connect, clinic IdP for care, operator IdP for dispatch). |
| `clientId`               | OAuth public client_id.                                                                  |
| `redirectUri`            | Registered redirect.                                                                     |
| `scope`                  | Space-separated scope string. Caller picks the default.                                  |
| `discoveryAlgorithm`     | `"oauth2"`, `"oidc"`, `"oauth2-then-oidc"`, or `"oauth2-then-fallback-paths"`.           |
| `sessionStorageBackend`  | `"session"` (connect, care) or `"local"` (dispatch).                                     |
| `storageNamespace`       | Prefix for the in-flight + persisted storage keys.                                       |
| `idTokenClaims`          | `"validated"` (care), `"unsafe-decode"` (dispatch), or `"skip"` (connect).               |
| `storageUrl?`            | Optional — paired storage URL (dispatch only).                                           |
| `onSessionSaved?`        | Side-effect after a session is persisted (e.g. mirror to a legacy bearer key).           |
| `onSessionCleared?`      | Side-effect on `clearSession`.                                                           |

Calls take an `OidcOptions` first, then their flow-specific args:

- `beginLogin(opts)` — discover, generate PKCE, redirect.
- `completeLogin(opts, { search, redirectUri })` — exchange the code, persist the session, return it.
- `loadSession(opts) → OidcSession | null`
- `saveSession(opts, session)`
- `clearSession(opts)`
- `refreshIfNeeded(opts, bufferMs?)` — silent refresh when the access token's about to expire.

### `OidcCallbackPage.tsx` — generic callback page

```tsx
import { OidcCallbackPage } from "@ohd/shared-web/OidcCallbackPage";

<OidcCallbackPage
  options={toSharedOptions(defaultOidcConfig())}
  successPath="/log"           // or "/roster", or "/active"
  onSessionComplete={async (s) => {
    // Optional — dispatch uses this to record the storage URL.
  }}
  layout={(body) => <div className="page">{body}</div>}  // optional chrome
/>
```

The component handles the redirect-back, runs the code exchange, and then either calls `useNavigate()(successPath)` on success or surfaces the error in its layout. Each SPA passes its own success route and (optionally) a `layout` prop that wraps the body in its app-shell chrome.

### `useExternalStore.ts` — React glue for the OHDC store

```tsx
import {
  useBootstrap,
  useStoreVersion,
  usePoll,
} from "@ohd/shared-web/useExternalStore";
import type { Store } from "@ohd/shared-web/store-types";
import * as backend from "./store";

const store: Store = {
  bootstrap: backend.bootstrap,
  getBootstrapStatus: backend.getBootstrapStatus,
  getVersion: backend.getVersion,
  refresh: backend.refresh,
  subscribe: backend.subscribe,
};

useBootstrap(store);              // run bootstrap once, return { ready, error }
useStoreVersion(store);           // re-render on every snapshot bump
usePoll(store, 5000);             // optional polling refresh (dispatch)
```

The `Store` interface from `./store-types`:

```ts
export interface Store {
  subscribe(fn: () => void): () => void;
  getVersion(): number;
  bootstrap(): Promise<void>;
  getBootstrapStatus(): { ready: boolean; error: string | null };
  refresh?(): Promise<void>;
}
```

## How each SPA wires this up

- **connect/web**: `src/ohdc/oidc.ts` keeps the existing `OidcConfig` (with `storageUrl` instead of the engine's `issuer`) for call-site compatibility, and exposes `toSharedOptions()` for `OidcCallbackPage` to call. `src/ohdc/useStore.ts` binds the shared hooks to `./store`.
- **care/web**: Same shape, but `OidcConfig` keeps `issuer` and the engine is configured with `idTokenClaims: "validated"` so the `oidcSubject` operator-subject header in `client.ts` keeps working. `useStore.ts` binds to `../mock/store` (which conditionally re-exports the real or fallback store backends).
- **emergency/dispatch**: `oidc.ts` configures `sessionStorageBackend: "local"`, `idTokenClaims: "unsafe-decode"`, and an `onSessionSaved` that mirrors the access token via `setOperatorToken()`. `OidcCallbackPage.tsx` passes a `layout` that wraps the body in `<div className="page">` to match the dispatch app shell.

## Adding a new consumer

1. Add `"@ohd/shared-web": "workspace:*"` in your SPA's `package.json` and add the SPA to the root [`pnpm-workspace.yaml`](../../../pnpm-workspace.yaml). `pnpm install` from the repo root.
2. Build a thin `oidc.ts` wrapper that maps your SPA's existing config shape to `OidcOptions`. Pick a unique `storageNamespace` so your storage keys don't collide.
3. Use `<OidcCallbackPage>` with your success route.
4. Bind your store to `useBootstrap` / `useStoreVersion` (and optionally `usePoll`) via the `Store` contract.

See the three existing wrappers for working examples (`connect/web/src/ohdc/`, `care/web/src/ohdc/`, `emergency/dispatch/src/ohdc/`).

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root. See [`../../../spec/LICENSE`](../../../spec/LICENSE).
