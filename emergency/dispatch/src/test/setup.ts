// Vitest global setup.
import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// Tests run against the in-memory mock store. Setting VITE_USE_MOCK=1
// flips `src/mock/store.ts` to the mock backend so no network is needed.
vi.stubEnv("VITE_USE_MOCK", "1");
