// Vitest global setup — bring in jest-dom matchers (toBeInTheDocument, etc).
import "@testing-library/jest-dom/vitest";

// Smoke tests run against the in-memory fallback store, not the OHDC backend.
// Setting `VITE_USE_MOCK_STORE=1` flips `src/mock/store.ts` to re-export
// `src/mock/store.fallback.ts` (the original 5-patient seed).
import { vi } from "vitest";
vi.stubEnv("VITE_USE_MOCK_STORE", "1");
