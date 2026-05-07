# Roadmap

> Phase-by-phase plan. Concrete tasks, target deliverables. Priority-ordered. Timelines are rough — they assume one developer (the founder) with real life competing for time.

**The north star for Phase 1:** by the time the founder's next doctor visit rolls around (~1 month), there's a running OHD instance collecting real personal health data from Health Connect, with manual food and medication logging, queryable by Claude via an MCP server. Something to actually show the doctor.

Everything else is in service of that.

## Phase 0 — Foundation (this week, one evening)

Claim the name. Establish the project. Don't overthink.

- [ ] Register domains: `ohd.org`, `openhealth data.org` (and `.com` as backup). `.org` preferred.
- [ ] Create GitHub organization: `openhealth-data` (or similar). Private until public release.
- [ ] Create core repos:
  - `openhealth-data/ohd` — this spec + project meta
  - `openhealth-data/ohd-core` — the backend
  - `openhealth-data/ohdc-android` — the Android Connector
  - `openhealth-data/cord-mcp` — the Cord MCP server
  - `openhealth-data/connector-mcp` — the Connector MCP server
- [ ] Commit this spec to `openhealth-data/ohd`.
- [ ] Deploy placeholder landing page to one domain:
  - Single HTML file. Logo (placeholder is fine). Tagline. One paragraph. Link to GitHub.
  - Served via Docker + Caddy on a small Hetzner VM.
- [ ] DNS for remaining domains → same server (redirect or serve same page).

**Deliverable:** public project existence. Domains secured. Repo structure in place. Spec committed.

**Time:** one evening.

---

## Phase 1 — Personal MVP (weeks 1–4)

Get the core running end-to-end with real personal data.

### 1.1 — OHD Core MVP backend (days 1–4)

- [ ] Python project scaffold: `uv init`, Python 3.12, `pyproject.toml`, `ruff`, `mypy`, `pytest`.
- [ ] FastAPI app with structured logging (JSON to stdout).
- [ ] Postgres schema: `users`, `health_events` tables with indexes (see `design/data-model.md`).
- [ ] Alembic migrations.
- [ ] Endpoints:
  - `POST /events` — single write
  - `POST /events/batch` — batched write (for sync scenarios)
  - `GET /events` — filter by type, from, to, limit (cursor pagination)
  - `GET /events/{id}` — fetch one
  - `GET /health` — liveness for Docker healthcheck
- [ ] Minimal auth: single hardcoded API key from env. Multi-user arrives in 1.5.
- [ ] Docker Compose stack: api, postgres, redis, caddy.
- [ ] Caddyfile with automatic HTTPS.
- [ ] Deploy to Hetzner via the founder's existing provisioning flow.
- [ ] Smoke test: curl write, curl read, from laptop.

**Deliverable:** production OHD instance at `https://ohd.<founder-domain>` accepting and returning events. Database ready to be filled.

### 1.2 — Android OHDC: Health Connect sync (days 5–10)

- [ ] Android Studio project: Kotlin, Jetpack Compose, minimum SDK 29.
- [ ] Add Health Connect SDK dependency (`androidx.health.connect:connect-client`).
- [ ] Declare all read permissions we need (see `research/health-connect.md`).
- [ ] Permission request flow + rationale activity.
- [ ] Config screen: OHD base URL, write token (stored in EncryptedSharedPreferences).
- [ ] Read implementations for Phase 1 data types:
  - BloodGlucoseRecord
  - HeartRateRecord (as series, with sample expansion)
  - WeightRecord, BodyFatRecord
  - StepsRecord
  - SleepSessionRecord
  - ExerciseSessionRecord
  - NutritionRecord, HydrationRecord
  - BloodPressureRecord
  - BodyTemperatureRecord
  - OxygenSaturationRecord
  - RespiratoryRateRecord, RestingHeartRateRecord
- [ ] Translation layer: each Health Connect record → OHD event (preserving source, timestamps, zone offsets).
- [ ] Idempotency: deterministic OHD event IDs derived from `(source_package, record_id)` to prevent duplicates on retry.
- [ ] WorkManager periodic job: sync every 30 min. Uses change tokens (not full reads) for incremental.
- [ ] Local queue in Room: if offline, queue events; flush on network.
- [ ] Sync status UI: last sync time, record counts per type, error state.
- [ ] Backfill: on first install, read the last 90 days (requires `READ_HEALTH_DATA_HISTORY` permission).
- [ ] Build a debug APK; sideload onto the founder's phone.

**Deliverable:** real glucose, heart rate, weight, steps, sleep, etc. flowing from Health Connect into OHD, automatically, every 30 min.

### 1.3 — Android OHDC: Manual logging (days 11–16)

- [ ] Food logging screen:
  - Barcode scanner (CameraX + ML Kit, see `research/barcode-scanning.md`)
  - OpenFoodFacts lookup with snapshot caching (see `research/openfoodfacts.md`)
  - Quantity input + nutrition preview
  - Start time + end time pickers (duration-based)
  - Submit → POST to OHD as `meal` event
- [ ] Medication logging:
  - User-local list of "my medications" (name, default dose, stored in Room)
  - Quick-tap: pick a medication → confirm time → submit
  - Optional: status (taken/skipped/late), notes
- [ ] Generic measurement entry:
  - Standard types with autocomplete (blood pressure, temperature, etc.)
  - Custom types (saved per-user for reuse)
  - Value + unit + timestamp
- [ ] Symptom quick-log:
  - Free text + severity picker

**Deliverable:** ability to log everything the founder would otherwise write in a markdown file, from the phone, in under 30 seconds per entry.

### 1.4 — Cord MCP (days 17–21)

- [ ] Python project with `fastmcp>=3`, `httpx`, `pydantic`.
- [ ] OHD client library: async HTTP wrapper around OHD REST API (used by both MCP servers and future CLI tools).
- [ ] Cord MCP server:
  - Auto-generated sub-server via `FastMCP.from_fastapi(ohd_app, include_operations={"GET"})`, mounted at `raw.*`.
  - Hand-written high-level tools mounted at `analysis.*`:
    - `query_latest`, `summarize`, `correlate`
    - `get_medications_taken`, `get_food_log`
    - `find_patterns` (statistical thresholds, LLM layer later)
    - `chart` (returns base64 PNG + chart spec)
  - `dateparser` integration for natural-language timestamps.
- [ ] `fastmcp install` configuration for Claude Desktop.
- [ ] Test: chat with Claude, ask "what's my average glucose this week?", verify correct response with real data.

**Deliverable:** Claude can answer questions about the founder's actual health data.

### 1.5 — Connector MCP (days 22–24)

- [ ] `fastmcp` project with tools:
  - `log_symptom`, `log_medication`, `log_food`, `log_measurement`
  - `log_exercise`, `log_mood`, `log_sleep`, `log_free_event`
- [ ] `fastmcp install` into Claude Desktop.
- [ ] Test: "I have a headache and I just took 400mg of ibuprofen" → two events logged, visible via the Cord MCP.

**Deliverable:** the founder can log events by chatting with Claude on the laptop as an alternative to the phone app.

### 1.6 — Doctor-visit preparation (days 25–28)

- [ ] Simple `cord export pdf` command (or web endpoint): generates a clinical summary PDF for a given time range.
  - Cover page with date range and the founder's name.
  - Medication list (current + adherence).
  - Vitals summary: glucose trends, blood pressure, heart rate ranges.
  - Symptom log.
  - Recent meals (summary, not every meal).
  - Raw data available on request (link back to OHD instance).
- [ ] Dry run: the founder reviews the PDF, makes sure it's useful, iterates.
- [ ] Hand to doctor.

**Deliverable:** a clean, professional-looking PDF the doctor can actually read in 2 minutes.

**Phase 1 total time:** 3–4 weeks of sustained work.

---

## Phase 2 — Multi-user, hardening, better UX (weeks 5–10)

The MVP works for the founder. Now make it work for anyone.

### 2.1 — Real auth (OIDC)

- [ ] Pick initial OIDC providers: Google (widely available), GitHub (dev-friendly).
- [ ] OIDC login flow in OHD Core:
  - `GET /auth/login?provider=google` → redirect
  - `GET /auth/callback` → exchange code, create/get user, issue session token
  - `POST /auth/logout`
  - `POST /auth/refresh`
- [ ] Session storage in Redis (revocable).
- [ ] User table: `(oidc_provider, oidc_subject) → user_uuid`.
- [ ] Scope every existing query by `current_user.id`.

### 2.2 — Grants and audit

- [ ] Grants table.
- [ ] `POST /grants`, `GET /grants`, `DELETE /grants/{id}`.
- [ ] Grant token format: JWT encoding grant ID.
- [ ] Grant-scoped request middleware: resolve token → load grant → enforce scope.
- [ ] Audit log table.
- [ ] Write audit entry on every access (self, grant, emergency).
- [ ] `GET /audit` for self-inspection.

### 2.3 — Personal web dashboard

- [ ] Static SPA (React/Vite or Svelte, whichever is less annoying).
- [ ] Login via OIDC.
- [ ] Views:
  - **Timeline** — reverse-chronological events with type filters.
  - **Charts** — pick metric, pick time range, render.
  - **Grants** — list, create, revoke.
  - **Audit** — recent access.
- [ ] Deploy as static files served by Caddy alongside OHD Core.

### 2.4 — Export / Import

- [ ] Design and document the export format (see `design/data-model.md`).
- [ ] `GET /export` endpoint: generates signed JSON archive.
- [ ] `POST /import` endpoint: accepts and validates archive, commits in transaction.
- [ ] Handle unknown extensions gracefully: move to `metadata._imported_extensions`.
- [ ] Round-trip test: export from instance A, import into instance B, compare.

### 2.5 — Hardening

- [ ] Rate limiting at both Caddy and app level (Redis token bucket).
- [ ] Per-user query quotas.
- [ ] Encryption-at-rest (Postgres volume encryption at minimum; column-level for sensitive fields is 3.x).
- [ ] Backup strategy: nightly `pg_dump` → object storage. Documented restore procedure. Tested.
- [ ] Security audit: OWASP top 10, especially auth flows.

### 2.6 — iOS Connector

- [ ] SwiftUI project.
- [ ] HealthKit read permissions for the same data types as Android.
- [ ] Background sync (same design as Android).
- [ ] Manual logging mirroring the Android app.
- [ ] Local queue, sync worker, config UI.

---

## Phase 3 — Real ecosystem features (weeks 11+)

### 3.1 — Cord doctor's dashboard

- [ ] Separate deployable app: `cord-doctor`.
- [ ] Clinical UX: dense, efficient, keyboard-friendly.
- [ ] Patient roster (grants they hold).
- [ ] Per-patient timeline, charts, medication review.
- [ ] LLM-assisted summaries ("anything to flag?").
- [ ] PDF export.

### 3.2 — LLM dashboard builder

- [ ] Natural-language chart creation.
- [ ] Sandboxed Python execution (subprocess with restricted imports or Pyodide in-browser).
- [ ] Template library (saved, named, shareable).
- [ ] Schedulable dashboards (email me this chart weekly).

### 3.3 — Extended data types and vocabulary

- [ ] Add more standard event types as use cases surface.
- [ ] Attachment support (EKG blobs, lab result PDFs, imaging references).
- [ ] Series events (Phase 2 optimization from `design/data-model.md`) for dense metrics.

### 3.4 — On-device OHD (phone as host)

- [ ] SQLite-based OHD implementation bundled with the Android app.
- [ ] Same API surface as the server, just local.
- [ ] Relay service for external access (Cord apps reach the phone via relay).
- [ ] Interesting, complex, not-MVP.

### 3.5 — Researcher portal

- [ ] Study posting form.
- [ ] User opt-in flow with granular consent.
- [ ] KYC integration.
- [ ] Automatic payouts.
- [ ] Cohort management and export for researchers.

### 3.6 — Hospital / provider deployments

- [ ] Enterprise Docker images with SAML/clinic SSO integrations.
- [ ] Integration bridges to common EHRs (FHIR adapter).
- [ ] Deployment documentation for compliance-sensitive environments.
- [ ] First pilot with a friendly clinic.

---

## Cross-cutting tasks (ongoing from Phase 2)

- **License finalization.** Pick Apache 2.0, write the "spirit of the project" document, apply to all repos.
- **Public release.** Move repos to public. Write announcement blog post. Post to relevant communities (r/selfhosted, Hacker News, health-tech forums).
- **Documentation.** Rewrite this spec as user-facing docs at `docs.ohd.org`. Separate into "for users", "for developers", "for healthcare providers".
- **Community.** Discord or Matrix. GitHub Discussions enabled. CONTRIBUTING.md that reflects the spirit document.
- **Contributor onboarding.** Good-first-issue labels. Monthly office hours (video).
- **Testing.** CI with Postgres + Redis services. Integration tests for each tier. Android instrumentation tests for Health Connect sync.
- **Observability.** Prometheus metrics on OHD Core. Uptime monitoring. Structured logs shipped to a collector.

---

## What we're deliberately NOT doing in Phase 1

Resist the temptation.

- **No multi-user support.** One user (the founder). Multi-user is Phase 2.
- **No real auth.** Hardcoded API key. OIDC is Phase 2.
- **No grants.** Self-access only. Grants are Phase 2.
- **No audit log.** Logs to stdout are fine for MVP. Real audit table is Phase 2.
- **No web dashboard.** MCP + Claude is the interface. Dashboard is Phase 2.
- **No iOS.** Android only (the founder's device).
- **No researcher portal, no payments, no KYC.** Years out.
- **No on-device OHD.** Server-hosted only.
- **No perfect schema.** Postgres with JSONB and one big table. Optimize later.
- **No custom barcode scanner, no proprietary food database.** OpenFoodFacts + ML Kit are good enough.
- **No reminders, no notifications, no alarms.** Passive logging only. Active features are Phase 3+.
- **No automated compression of historical data.** Just keep it raw. Storage is cheap.
- **No fancy data portability yet.** Export/import is Phase 2. For Phase 1, the database is authoritative; if we lose it, we re-sync from Health Connect.

Every item above is a thing the founder might be tempted to build because it sounds interesting. Build Phase 1 first. Ship something. Use it. Then iterate.

## Success criteria

### Phase 0 success

- Domains registered.
- Repos exist.
- Landing page up.
- Spec committed.

### Phase 1 success

- The founder is logging real health data daily without friction.
- Claude can answer non-trivial questions about that data.
- The doctor got a useful PDF at the next visit.
- The founder stopped maintaining the markdown file.

### Phase 2 success

- A second user (friend, family member) is using their own OHD instance end-to-end.
- At least one doctor has used Cord to query a patient's data and said it was useful.
- The code is public on GitHub.
- The export/import round-trip works.

### Phase 3 success

- Someone other than the founder has deployed a fork or extended the project.
- Real external contributors have merged PRs.
- Public awareness (HN post with >100 upvotes is a reasonable bar).
- First external healthcare provider has expressed interest.

## A closing note on pace

The founder has explicitly said: "It's better to build something than build it perfect and never build it perfect."

This roadmap is ambitious but each phase is shippable on its own. **Ship Phase 1 before starting Phase 2.** If Phase 1 takes two months instead of one, fine. If Phase 2 never happens because Phase 1 solved the personal problem, that's also fine. The value is in the progression, not the completion.
