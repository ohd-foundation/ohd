# Roadmap — Work Packages

> Work-package structure (no time-based phases). Each package is a self-contained unit of work with explicit dependencies. Packages whose dependencies are met can be worked in parallel by different contributors.

## Dependency graph

```
                              ┌──────────────────────────────┐
                              │  0 — Project setup            │
                              │  (repos, infra, OIDC, Fulcio) │
                              └────────────┬─────────────────┘
                                           │
                ┌──────────┬───────────────┼────────────────┬──────────────┬──────────┐
                ▼          ▼               ▼                ▼              ▼          ▼
        ┌─────────────┐ ┌──────────┐ ┌──────────────┐ ┌──────────────┐ ┌────────┐ ┌──────────┐
        │  1 — Storage│ │  2 (also │ │  3 — MCP     │ │  4 — Connect │ │ 6 —    │ │ 8 —      │
        │  library    │ │  needs 1)│ │  servers     │ │  Android     │ │ Care   │ │ Emergency│
        │  (Rust core)│ │ — Storage│ │  (Connect /  │ │  (mobile)    │ │ design │ │ design   │
        │             │ │  server  │ │   Care MCP)  │ │              │ │        │ │          │
        └──────┬──────┘ └─────┬────┘ └──────────────┘ └──────┬───────┘ └────┬───┘ └────┬─────┘
               │              │                              │              │           │
               │              ▼                              ▼              ▼           ▼
               │        ┌──────────┐                  ┌─────────────┐  ┌─────────┐ ┌──────────┐
               │        │  5a —    │                  │  5b — Relay │  │ 7 — Care│ │ 9 —      │
               │        │  Relay   │                  │  test on    │  │ app     │ │ Emergency│
               │        │  binary  │                  │  Android    │  │ (also   │ │ app (also│
               │        └──────────┘                  │  (also      │  │ needs 1)│ │ needs 1) │
               │                                      │  needs 5a)  │  └─────────┘ └──────────┘
               │                                      └─────────────┘
               │
               └──────────► [also blocks 7 and 9 via storage-lib dependency]

Direct dependency edges:
  0 → 1, 2, 3, 4, 6, 8     (project setup blocks everything)
  1 → 2, 3, 4, 7, 9        (storage lib blocks anything that consumes it)
  2 → 5a                   (relay needs the server protocol)
  4 → 5b                   (relay-on-Android needs the Android app)
  5a → 5b                  (Android relay test needs the relay binary)
  6 → 7                    (Care design before Care implementation)
  8 → 9                    (Emergency design before Emergency implementation)
```

The "ready to start" set at any moment = all packages whose dependencies are completed.

---

## 0 — Project setup

The shared substrate every other package builds on.

**Scope:**

- Register / confirm domains: `openhealthdata.org`, `ohd.dev`.
- Create GitHub org `openhealth-data` (private until public release).
- Create skeleton repos (one per package below): `ohd`, `ohd-protocol`, `ohd-storage`, `ohd-connect-mcp`, `ohd-care-mcp`, `ohd-connect-android`, `ohd-relay`, `ohd-care-app`, `ohd-emergency-app`. Each gets a stub README + LICENSE + NOTICE + SPIRIT.md from the `ohd` repo.
- Commit this spec to `openhealth-data/ohd`.
- Stand up a placeholder landing page on a Hetzner VM behind Caddy at `openhealthdata.org`.
- Deploy infrastructure prerequisites:
  - **OHD Account OIDC provider** (Authentik or Keycloak) at `accounts.ohd.dev`.
  - **OHD Fulcio instance(s)** at `fulcio.ohd.dev` per [`design/emergency-trust.md`](design/emergency-trust.md).
  - **OHD Rekor instance** at `rekor.ohd.dev`.
- Apply for an IANA Private Enterprise Number for OHD-specific X.509 OIDs.
- Set up CI infrastructure (the conformance corpus runner, Buf cloud or self-hosted Buf Schema Registry).

**Dependencies:** none.

**Blocks:** all other packages.

**Deliverable:** real-but-empty project skeleton; every other package can `git clone` its repo, find its place, and start.

---

## 1 — Storage library (`ohd-storage`)

The Rust core. Where most of the protocol lives.

**Scope:**

- Rust workspace; `cargo new --lib ohd-storage`.
- Implement on-disk schema from [`design/storage-format.md`](design/storage-format.md): events, channels, samples, attachments, grants, grant_cases, cases, case_filters, audit, peer_sync, _meta, trusted_authorities.
- Embedded standard channel registry (`registry/v1.json`); load on file creation.
- Storage library API: typed Rust API (`PutEvents`, `QueryEvents`, `Aggregate`, `AttachBlob`, etc.).
- Sample-block encoders/decoders (encoding 1 mandatory, encoding 2 strongly recommended).
- Grant resolution + case scope resolution (filter union + asymmetric inheritance).
- SQLCipher integration; per-user-key encryption.
- Sync primitives (rowid watermarks, ULID dedup, `origin_peer_id`).
- Conformance corpus: input fixtures + expected outputs, runner. The corpus passes against the library.
- Bindings:
  - **`uniffi`** — Kotlin (`.aar`) and Swift (`.xcframework`) bindings; not yet consumed by 4/9, but compile-tested.
  - **`PyO3`** — Python wheel for tooling and the conformance harness.

**Dependencies:** 0.

**Blocks:** 2, 3, 4, 7, 9.

**Deliverable:** `ohd-storage` crate + bindings + conformance corpus, all green in CI.

---

## 2 — Storage server

The Connect-RPC HTTP service that wraps the library.

**Scope:**

- Rust HTTP server (using `tonic` or a Connect-RPC-native framework) wrapping the storage library.
- `.proto` files in `ohd-protocol` repo per [`design/ohdc-protocol.md`](design/ohdc-protocol.md). Buf config; CI publishes generated client libraries.
- OAuth Authorization Server endpoints from [`design/auth.md`](design/auth.md): `/.well-known/oauth-authorization-server`, `/authorize`, `/token`, `/oidc-callback`, `/device`, `/oauth/register`.
- OIDC integration (Google + OHD Account at minimum; rest configurable).
- System DB (SQLite for small / Postgres for large): `oidc_identities`, `sessions`, `pending_invites`, `push_tokens`, `notification_*`, `storage_relay_registrations`.
- Caddy front; Docker Compose; deployment to Hetzner per [`design/deployment.md`](design/deployment.md).
- Health endpoint, Prometheus metrics, structured logs.

**Dependencies:** 0, 1.

**Blocks:** 5a.

**Deliverable:** production OHDC instance reachable on the public internet under self-session OIDC; verified end-to-end via `buf curl`.

---

## 3 — MCP servers (Connect MCP, Care MCP)

LLM-tool surfaces for personal and clinical use.

**Scope:**

- Python project with `fastmcp>=3` and the OHDC Python client (from `ohd-protocol` codegen).
- **Connect MCP**: per [`components/connect.md`](components/connect.md) tool list — `log_symptom`, `log_food`, `log_medication`, `log_measurement`, `log_exercise`, `log_mood`, `log_sleep`, `log_free_event`, `query_latest`, `summarize`, `correlate`, `find_patterns`, `chart`, `get_medications_taken`, `get_food_log`, plus grants/pending/audit ops.
- **Care MCP**: per [`components/care.md`](components/care.md) — multi-patient (`switch_patient`, `current_patient`), read scoped to active patient's grant, write-with-approval submissions.
- OAuth Device Authorization Grant for first-run.
- `dateparser` for natural-language timestamps.
- `fastmcp install` to Claude Desktop; manual smoke test.

**Dependencies:** 0, 1.

**Blocks:** nothing.

**Deliverable:** Claude Desktop shows OHD tools; calls work end-to-end against a deployed (or local) storage server.

---

## 4 — Connect Android (mobile)

The personal-side app.

**Scope:**

- Android Studio project; Kotlin + Jetpack Compose; min SDK 29.
- Link `ohd-storage` via `uniffi` (`.aar` from package 1).
- OAuth + OIDC self-session login (Custom Tabs); secure token storage in EncryptedSharedPreferences.
- Health Connect SDK + permission flow + rationale activity.
- In-app device-token bridge (Model 3 from [`future-implementations/device-pairing.md`](future-implementations/device-pairing.md)) — `Auth.IssueDeviceToken` for the Health Connect sync worker.
- Read implementations for the v1 record types (BloodGlucoseRecord, HeartRateRecord, WeightRecord, …).
- Translation layer: Health Connect record → OHD event with appropriate type / channels per [`design/data-model.md`](design/data-model.md).
- WorkManager periodic sync (30 min, change tokens).
- Local queue (Room) for offline; flush on network.
- Manual logging UIs: barcode + OpenFoodFacts food, medication quick-tap, generic measurement entry, symptom quick-log.
- Pending review UI (write-with-approval flow).
- Cases tab + emergency settings tab per [`design/screens-emergency.md`](../design/screens-emergency.md).
- Sync status UI: last sync, per-type counts, error state.
- Backfill (90 days) on first install.
- Sideload debug APK to founder's phone for testing.

**Dependencies:** 0, 1.

**Blocks:** 5b.

**Deliverable:** APK collecting real Health Connect data into the founder's OHD instance, with manual logging working, in daily use.

---

## 5a — OHD Relay binary

The bridging service.

**Scope:**

- Rust binary implementing [`design/relay-protocol.md`](design/relay-protocol.md): tunnel framing, session multiplexing, registration, heartbeat, push-wake.
- HTTP/3 server (using `quinn`).
- Registration / session / pairing tables.
- LAN fast-path discovery (mDNS, `_ohd._tcp.local`).
- Caddy front; Docker Compose; deployable independently of storage or alongside.
- Push-token delivery client (FCM + APNs) for tunnel-wake events.
- Bandwidth metering, rate limiting per registration.

**Dependencies:** 2 (needs the server protocol it's relaying; the relay knows nothing about OHDC content but the integration tests rely on the server being live).

**Blocks:** 5b.

**Deliverable:** Relay running publicly reachable; tested with the server's tunnel registration RPCs end-to-end (storage instance behind a NAT'd box can be reached through the relay).

---

## 5b — Relay test on Android

Wire Connect Android into the relay so phone-hosted storage is reachable.

**Scope:**

- Connect Android registers with relay on first launch (using user's OIDC credentials).
- Background heartbeat + reconnect-with-backoff (per relay-protocol "Persistence").
- FCM push-wake handler that re-establishes the tunnel.
- iOS work deferred to a later iteration (when iOS app exists).
- End-to-end test: laptop with another OHD client (or `buf curl`) connects to the phone-hosted storage through the relay and successfully reads/writes events.

**Dependencies:** 4, 5a.

**Blocks:** nothing.

**Deliverable:** demonstrable "phone is reachable through relay" — a laptop browser hits `https://relay.ohd.dev/r/<rendezvous_id>` and gets OHDC data from the phone.

---

## 6 — Care app design (UX, Pencil)

Designer-driven; not code.

**Scope:**

- Pencil files for OHD Care:
  - Patient roster
  - Per-patient view (header with active grant scope, tabs for timeline / vitals / medications / symptoms / foods / labs / imaging / notes)
  - Visit panel (previous-visit summary, assessment input, write-back queue)
  - Chart builder
  - Audit transparency view (matching the patient-side audit UI for symmetry)
  - Operator login + roster management
  - Care MCP context UI (which patient is active)
  - Cases UI (list of cases, case detail, handoff initiation, predecessor chain visualization)
  - Patient-curated case display (per [`design/care-auth.md`](design/care-auth.md) "Patient-issued case grant")
- Iterate with stakeholder review.

**Dependencies:** 0.

**Blocks:** 7.

**Deliverable:** Pencil files in `spec/design/` (or a dedicated `ohd-care-app/design/` dir); accepted by the implementation team.

---

## 7 — Care app implementation

The reference clinical app.

**Scope:**

- Care backend per [`design/care-auth.md`](design/care-auth.md): operator OIDC, `care_operator_users`, `care_operator_sessions`, `care_patient_grants`, `care_operator_audit`, KMS-encrypted token vault.
- Care web app (TypeScript, framework TBD by 6's design constraints): roster, per-patient view, visit prep, write-with-approval submissions, audit transparency, case-aware UI.
- Care MCP integration (uses Care MCP from 3, scoped to operator session).
- Pilot deployment (friendly clinic).

**Dependencies:** 1, 6.

**Blocks:** nothing.

**Deliverable:** deployable Care instance running in pilot at a friendly clinic; clinician can open patients, see data, submit write-with-approval, see audit.

---

## 8 — Emergency app design

Designer-driven.

**Scope:**

- Pencil files for OHD Emergency:
  - Paramedic tablet — patient discovery (BLE scan), break-glass initiation, patient view, intervention logging, handoff UI.
  - Dispatch console — active case board, crew status, audit visibility, reopen-token issuance.
  - Authority-cert management (operator-side) — onboarding, refresh status, multi-parent cross-signing visibility.
- BLE discovery state diagrams and timing diagrams (operator + patient sides).
- Iterate with stakeholder review.

**Dependencies:** 0.

**Blocks:** 9.

**Deliverable:** Pencil files; accepted by the implementation team.

---

## 9 — Emergency app implementation

Reference EMS / hospital ER tablet + dispatch app.

**Scope:**

- Native Android + iOS tablet apps for paramedics (Compose / SwiftUI).
- Web SPA for dispatch console.
- Authority-cert refresh client (calls OHD Fulcio's `/api/v2/signingCert` daily, OIDC-authenticated).
- Per-responder cert layer (optional per-org).
- Signed `EmergencyAccessRequest` construction per [`design/emergency-trust.md`](design/emergency-trust.md).
- Patient-side `DeliverEmergencyRequest` plumbing (already specced; this package's job is the responder side).
- BLE discovery + signing (concrete BLE service UUID + characteristic shapes finalized here).
- Bystander-mediated transport (when patient phone has no internet) — uses Connect Android's bystander proxy role added in 4.
- Operator-side records layer (optional; orgs can BYO database or use OHD Emergency's reference deployment).
- Emergency MCP for triage / dispatch / handoff drafting.
- Pilot deployment (friendly EMS station).

**Dependencies:** 1, 8.

**Blocks:** nothing.

**Deliverable:** deployable emergency-services instance + cert refresh + at least one demo break-glass run end-to-end against a Connect Android phone.

---

## What's deliberately NOT a v1 work package

- **iOS Connect** — same shape as 4 once iOS is on the table; deferred until the founder has time.
- **Connect web** — web client of OHDC; deferred. Android first.
- **OHD Cloud SaaS** — multi-tenant production-grade. v1 ships the building blocks; OHD Cloud as a hosted business is a separate operational endeavor.
- **Sync (cache↔primary)** — specced in [`design/sync-protocol.md`](design/sync-protocol.md); only needed when a cache + primary topology is in play, which is post-v1 (v1 is server-primary only).
- **End-to-end channel encryption** for sensitive sensitivity classes — open per [`design/encryption.md`](design/encryption.md).
- **Sensor / vendor backend integrations** — deferred per [`future-implementations/device-pairing.md`](future-implementations/device-pairing.md).
- **Researcher portal** — far future.
- **Hospital / FHIR adapter** — separate component, post-pilot.
- **Notification delivery infrastructure** beyond push-wake for tunnels — push-token registration in 2 covers the table; full notification dispatch worker is in 2 but only the triggers actually firing need 4/7/9 to plumb in.

---

## Cross-cutting obligations (per package)

Every package above must:

- Honor the dual license (Apache-2.0 OR MIT) with `LICENSE-APACHE`, `LICENSE-MIT`, `NOTICE`, `SPIRIT.md` propagated from the `ohd` repo.
- Pass the relevant subset of the conformance corpus (where applicable).
- Run CI on every PR.
- Document operational concerns: how to deploy, how to back up, how to monitor.

---

## Closing note on order

The DAG is partial. Many packages can run in parallel:

- 1, 6, 8 can start immediately after 0 (storage lib + Care design + Emergency design).
- 2, 3, 4 can start in parallel as soon as 1 is far enough along to expose API stubs.
- 5a can start as soon as 2 is dogfoodable.
- 5b, 7, 9 are the long-tail integration packages.

Don't wait for 1 to be complete to start 2, 3, 4 — start them with mocked-out storage and swap in the real one as 1 stabilizes. Same pattern for designs into implementations.
