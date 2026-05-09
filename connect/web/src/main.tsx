import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import { App } from "./App";
import "./index.css";
import { bootstrapTheme } from "./theme";
import { _seedPendingQueriesMock } from "./ohdc/client";

// Apply the persisted theme BEFORE React mounts so there's no flash of
// dark-mode-on-a-light-day. Reads localStorage["ohd-connect-theme"].
bootstrapTheme();

// In dev / preview builds, seed the pending-queries mock with a couple
// of synthetic rows so reviewers see a populated UI. Production builds
// (or builds where the wire RPC is exposed) skip this.
if (import.meta.env.DEV || import.meta.env.MODE === "preview") {
  _seedPendingQueriesMock([
    {
      queryUlid: "01J0PENDQUERY0000000DEMO01",
      grantUlid: "01J0DEMO0000GRANT00000DOC01",
      grantLabel: "Dr. Patel — primary care review",
      queryKind: "query_events",
      summary: {
        eventTypes: ["std.blood_glucose", "std.heart_rate_resting"],
        fromMs: Date.now() - 7 * 86_400_000,
        toMs: null,
        hint: "Wants to read: glucose, heart rate, last 7 days",
      },
      requestedAtMs: Date.now() - 4 * 60_000,
      expiresAtMs: Date.now() + 86_400_000,
    },
    {
      queryUlid: "01J0PENDQUERY0000000DEMO02",
      grantUlid: "01J0DEMO0000GRANT00000RES01",
      grantLabel: "Beta-cell longitudinal study",
      queryKind: "aggregate",
      summary: {
        eventTypes: ["std.blood_glucose"],
        fromMs: Date.now() - 30 * 86_400_000,
        toMs: null,
        hint: "Aggregate-only, 30-day window",
      },
      requestedAtMs: Date.now() - 22 * 60_000,
      expiresAtMs: Date.now() + 12 * 3_600_000,
    },
  ]);
}

const rootEl = document.getElementById("root");
if (!rootEl) {
  throw new Error("OHD Connect: #root element missing from index.html");
}

ReactDOM.createRoot(rootEl).render(
  <React.StrictMode>
    <BrowserRouter>
      <App />
    </BrowserRouter>
  </React.StrictMode>,
);
