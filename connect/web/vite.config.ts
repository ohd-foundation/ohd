/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// OHD Connect — personal-facing SPA.
//
// Dev server runs on :5174 by default to avoid clashing with `care/web` on :5173.
// In production it's built and served behind the deployment's reverse proxy
// (or a static-hosting target — the SPA is just a bundle of files).
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5174,
    strictPort: false,
  },
  build: {
    outDir: "dist",
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    css: false,
    // Restrict to TS sources only — guards against any stray emitted .js
    // pollution from an earlier `tsc -b` run.
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
