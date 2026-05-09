// React glue for an OHDC-style store. Mirrors the per-SPA `useStore.ts`
// modules (connect/web, care/web, emergency/dispatch) — which were
// near-identical — over the `Store` contract from `./store-types.ts`.
//
// The store is a plain singleton in each SPA; component re-renders are
// driven by `useSyncExternalStore` against the store's tiny event
// emitter. The bootstrap is kicked off exactly once per mount via the
// store's idempotent `bootstrap()`.

import { useEffect, useSyncExternalStore } from "react";
import type { Store } from "./store-types";

/** Force the component to re-render whenever the store snapshot bumps. */
export function useStoreVersion(store: Store): number {
  return useSyncExternalStore(store.subscribe, store.getVersion, store.getVersion);
}

/**
 * Trigger the store's bootstrap exactly once (idempotent inside the
 * store). Returns the live `{ ready, error }` status the App uses to
 * render a loading / error gate.
 */
export function useBootstrap(store: Store): { ready: boolean; error: string | null } {
  // Re-render on every snapshot bump so `ready` flips false → true.
  useStoreVersion(store);
  useEffect(() => {
    void store.bootstrap();
  }, [store]);
  return store.getBootstrapStatus();
}

/**
 * Poll `store.refresh()` on the given interval (default 5s). Pauses
 * while the tab is hidden so we don't burn the operator's bandwidth in
 * the background. Used by the dispatch console's active-cases page.
 */
export function usePoll(store: Store, intervalMs = 5000): void {
  useEffect(() => {
    const refresh = store.refresh;
    if (!refresh) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout> | null = null;
    function tick() {
      if (!active) return;
      if (typeof document !== "undefined" && document.hidden) {
        timer = setTimeout(tick, intervalMs);
        return;
      }
      void refresh!.call(store).finally(() => {
        if (!active) return;
        timer = setTimeout(tick, intervalMs);
      });
    }
    timer = setTimeout(tick, intervalMs);
    return () => {
      active = false;
      if (timer) clearTimeout(timer);
    };
  }, [store, intervalMs]);
}
