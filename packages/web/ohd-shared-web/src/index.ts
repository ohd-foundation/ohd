// `@ohd/shared-web` — shared web utilities for OHD Connect, Care, and
// Emergency SPAs. See `./oidc.ts`, `./OidcCallbackPage.tsx`, and
// `./useExternalStore.ts` for the pieces. The package is consumed as
// raw TS via `pnpm`'s workspace protocol; each SPA's Vite + tsc do the
// transpilation.

export {
  beginLogin,
  completeLogin,
  loadSession,
  saveSession,
  clearSession,
  refreshIfNeeded,
  type OidcOptions,
  type OidcSession,
  type DiscoveryAlgorithm,
  type CallbackParams,
} from "./oidc";

export {
  OidcCallbackPage,
  type OidcCallbackPageProps,
} from "./OidcCallbackPage";

export {
  useStoreVersion,
  useBootstrap,
  usePoll,
} from "./useExternalStore";

export { type Store } from "./store-types";
