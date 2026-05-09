// The store contract that the React glue hooks bind to.
//
// Each SPA has its own concrete store (`care/web/src/mock/store.ts`,
// `connect/web/src/ohdc/store.ts`, `emergency/dispatch/src/ohdc/store.ts`)
// — they all expose the same five-method surface, which is the contract
// here. Hooks take a `Store` value, not a module import, so the same
// hook code services every SPA.

export interface Store {
  /** Subscribe to snapshot bumps. Returns an unsubscribe function. */
  subscribe(fn: () => void): () => void;
  /** Read the current snapshot version (a monotonic counter). */
  getVersion(): number;
  /** Bootstrap the store — typically WhoAmI + initial fetches. Idempotent. */
  bootstrap(): Promise<void>;
  /** Read whether bootstrap has completed (and what error if any). */
  getBootstrapStatus(): { ready: boolean; error: string | null };
  /** Optional lightweight refresh; only stores that poll need this. */
  refresh?(): Promise<void>;
}
