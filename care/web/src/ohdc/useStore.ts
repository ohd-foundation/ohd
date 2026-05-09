// React hook gluing the OHDC store's subscribe/version mechanism to React.
//
// Thin wrapper over `@ohd/shared-web/useExternalStore`, bound to this
// SPA's concrete `../mock/store` module. The store at `../mock/store`
// is a plain singleton; component re-renders are triggered by
// `useSyncExternalStore`, which subscribes to the store's tiny
// event-emitter and reads the version counter as the snapshot.

import { bootstrap, getBootstrapStatus, getVersion, refresh, subscribe } from "../mock/store";
import {
  useBootstrap as useBootstrapShared,
  useStoreVersion as useStoreVersionShared,
} from "@ohd/shared-web/useExternalStore";
import type { Store } from "@ohd/shared-web/store-types";

const store: Store = { bootstrap, getBootstrapStatus, getVersion, refresh, subscribe };

/** Force the component to re-render whenever the store snapshot bumps. */
export function useStoreVersion(): number {
  return useStoreVersionShared(store);
}

/**
 * Trigger the OHDC bootstrap exactly once (idempotent). Returns the live
 * `{ ready, error }` status the App uses to render a loading / error gate.
 */
export function useBootstrap(): { ready: boolean; error: string | null } {
  return useBootstrapShared(store);
}
