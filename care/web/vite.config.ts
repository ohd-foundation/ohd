/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// OHD Care — operator-facing SPA.
// Dev server runs on :5173 by default; in production it's built and served behind
// the operator's Caddy (see ../deploy/Caddyfile).
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
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
