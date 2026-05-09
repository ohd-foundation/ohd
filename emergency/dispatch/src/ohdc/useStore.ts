// React glue for the dispatch store. Mirrors the per-SPA
// useStoreVersion + useBootstrap, plus a polling hook for the
// active-cases page. All three live in `@ohd/shared-web` now — this
// module is a thin wrapper that binds them to the local store.

import { bootstrap, getBootstrapStatus, getVersion, refresh, subscribe } from "./store";
import {
  useBootstrap as useBootstrapShared,
  useStoreVersion as useStoreVersionShared,
  usePoll as usePollShared,
} from "@ohd/shared-web/useExternalStore";
import type { Store } from "@ohd/shared-web/store-types";

const store: Store = { bootstrap, getBootstrapStatus, getVersion, refresh, subscribe };

export function useStoreVersion(): number {
  return useStoreVersionShared(store);
}

/** Kick off the OHDC bootstrap exactly once. */
export function useBootstrap(): { ready: boolean; error: string | null } {
  return useBootstrapShared(store);
}

/**
 * Poll `refresh()` on the given interval (default 5s). Pauses when the tab
 * is hidden so we don't burn the operator's bandwidth in the background.
 */
export function usePoll(intervalMs = 5000): void {
  usePollShared(store, intervalMs);
}
