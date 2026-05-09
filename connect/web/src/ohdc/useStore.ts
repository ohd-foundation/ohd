// React hooks gluing the OHDC store to React's render lifecycle.
//
// Thin wrapper over `@ohd/shared-web/useExternalStore`, bound to this
// SPA's concrete `./store` module. `useStoreVersion` is the canonical
// re-render trigger; every component that reads from the snapshot
// calls it.

import { bootstrap, getBootstrapStatus, getVersion, subscribe } from "./store";
import {
  useBootstrap as useBootstrapShared,
  useStoreVersion as useStoreVersionShared,
} from "@ohd/shared-web/useExternalStore";
import type { Store } from "@ohd/shared-web/store-types";

const store: Store = { bootstrap, getBootstrapStatus, getVersion, subscribe };

export function useStoreVersion(): number {
  return useStoreVersionShared(store);
}

export function useBootstrap(): { ready: boolean; error: string | null } {
  return useBootstrapShared(store);
}
