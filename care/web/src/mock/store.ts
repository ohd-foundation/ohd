// Care v0 store — entry point.
//
// Two backends:
//   - **Default**: `../ohdc/store` — calls into a real OHD Storage instance via
//     the Connect-RPC client at `../ohdc/client.ts`. The active grant token is
//     read from `?token=...` on first load and persisted to `sessionStorage`.
//   - **Fallback**: `./store.fallback` — the original 5-patient in-memory mock.
//     Switch on with `VITE_USE_MOCK_STORE=1` at build time. Useful for offline
//     UI work or running tests without a storage server.
//
// The two backends export the same symbol surface so call sites don't need to
// branch. The OHDC store also exports `bootstrap` / `subscribe` / `refresh` /
// `getVersion` / `getBootstrapStatus` for the React shell to wire up
// async-aware re-rendering.
//
// This file is the union of those surfaces. Each function delegates to
// whichever backend the build mode picks, decided once at module load.

import * as ohdcStore from "../ohdc/store";
import * as fallbackStore from "./store.fallback";

const USE_MOCK = (import.meta.env?.VITE_USE_MOCK_STORE as string | undefined) === "1";

const backend = USE_MOCK ? fallbackStore : ohdcStore;

export const MOCK_OPERATOR = backend.MOCK_OPERATOR;

export const listPatients = backend.listPatients;
export const getPatientBySlug = backend.getPatientBySlug;
export const submitNote = backend.submitNote;
export const submitVital = backend.submitVital;
export const submitSymptom = backend.submitSymptom;
export const submitFood = backend.submitFood;
export const submitMedication = backend.submitMedication;
export const submitLab = backend.submitLab;
export const submitImaging = backend.submitImaging;
export const bootstrap = backend.bootstrap;
export const refresh = backend.refresh;
export const subscribe = backend.subscribe;
export const getVersion = backend.getVersion;
export const getBootstrapStatus = backend.getBootstrapStatus;
