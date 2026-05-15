import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// cord-server serves the production build from dist/ as static files.
// During `npm run dev`, proxy API + healthcheck to the local cord-server.
const BACKEND = "http://127.0.0.1:8446";

export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
  },
  server: {
    proxy: {
      "/v1": { target: BACKEND, changeOrigin: true },
      "/healthz": { target: BACKEND, changeOrigin: true },
    },
  },
});
