/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// OHD Emergency — dispatch console SPA.
//
// Dev server intentionally pinned to :5175 to stay out of the way of:
//   - care/web on :5173
//   - connect/web on :5174
//
// In production the bundle is served by Caddy alongside the relay
// (see ../deploy/Caddyfile + ../deploy/docker-compose.yml's `dispatch-web`).
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5175,
    strictPort: false,
    host: "127.0.0.1",
  },
  build: {
    target: "es2022",
    outDir: "dist",
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    css: false,
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
