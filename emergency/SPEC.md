# OHD Emergency — Implementation Spec

> Implementation-ready spec for the OHD Emergency component, distilled from [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) and [`../spec/docs/design/emergency-trust.md`](../spec/docs/design/emergency-trust.md). Pinned to the four sub-projects this directory builds: `tablet/`, `dispatch/`, `mcp/`, `cli/`. Where a deeper detail is needed, this doc cites the global spec rather than restating it — the global spec is the single source of truth.

## 0. Out of scope (read first)

To prevent drift with the canonical spec:

- **Relay binary, Fulcio integration, Rekor, OIDC for orgs, root CA ceremony.** Lives in `../relay/`. Emergency is the consumer.
- **Patient-side break-glass dialog UX, BLE beacon broadcast on patient phone, bystander proxy role.** Lives in OHD Connect (`../connect/`).
- **OHD Storage write/read internals, channel registry, sample blocks.** Lives in `../storage/`.
- **OHDC `.proto`, codegen pipeline.** Owned by the OHDC layer (under `../storage/` or a shared protos dir).
- **NEMSIS / HL7 export, CAD integration, billing, HR, scheduling.** Out of OHD Emergency entirely.

The contracts Emergency consumes:

- **`EmergencyAccessRequest`** — Protobuf schema in [`spec/emergency-trust.md`](spec/emergency-trust.md) "Signed emergency-access request". Built and signed by the relay; Emergency populates the informational fields (`responder_label`, `scene_context`, `operator_label`, optional GPS) and hands the structured request to the relay's signing endpoint.
- **OHDC RPCs** — once the relay's emergency-authority flow has issued a grant, Emergency speaks ordinary OHDC over HTTP/3 with the case-bound grant token. See [`../spec/docs/design/ohdc-protocol.md`](../spec/docs/design/ohdc-protocol.md).

## 1. Tablet — paramedic app

### 1.1 Form factor

- Android-first (Kotlin + Jetpack Compose). Tablet-optimized; large fonts, single-column scrollable layout, gloved-finger-friendly hit targets.
- iOS (Swift / SwiftUI) deferred to a later phase; not in this skeleton.
- Authenticated to the operator's relay via the operator's identity system (clinic SSO / Okta / etc., handled by the relay; the tablet holds an OAuth bearer or operator-issued device token).

### 1.2 Functions (what the tablet must do)

| Function | Notes |
|---|---|
| **BLE patient discovery** | Scan for the OHD beacon service UUID. List nearby beacons with RSSI + time-since-discovered. Manual entry as fallback. Concrete BLE service UUID + characteristic IDs are deferred (open item in [`spec/emergency-trust.md`](spec/emergency-trust.md) "Open items"). |
| **Break-glass initiation** | Single-tap from the patient row. Tablet POSTs to the operator's relay `/emergency/initiate` (relay-internal endpoint, not OHDC) with `(beacon_id, scene_context, optional GPS, responder cert if used)`. Relay signs the `EmergencyAccessRequest` and dispatches it. Tablet polls / streams status: `waiting_for_patient` → `approved` / `rejected` / `auto_granted` / `timeout_denied`. |
| **Patient view** | Once a grant is issued, fetch the emergency-template-cloned slice via OHDC `query_events` with the case-bound grant. Layout per [`spec/screens-emergency.md`](spec/screens-emergency.md) "Patient view (paramedic tablet)": critical info card (allergies, blood type, advance directives) at top, active medications, vitals + sparklines, diagnoses, observations. |
| **Intervention logging** | Fast-entry forms for vitals (HR, BP, SpO2, temperature, GCS), drugs (name, dose, route, time), observations (chief complaint, LoC, skin color, free text). Each submission is `put_events` over OHDC against the active case-bound grant; the tablet stamps `case_id` automatically. |
| **Case timeline** | Chronological merged view of (a) data the case opened with and (b) interventions the crew logged. Single OHDC query bound to the case grant. |
| **Handoff** | End-of-call action. Picks receiving facility (autocomplete from operator's "typical destinations" config + manual entry). Tablet calls a relay-mediated handoff endpoint that opens a successor case under the receiving operator's authority, transitions current grant to read-only on the case span, sets `predecessor_case_id`. UI returns to discovery / next call. |
| **Offline operation** | Ambulance is often in dead zones. Tablet caches: (a) the active case's data slice on grant, (b) the queue of pending intervention writes. On connectivity restore, the queue flushes. Conflict mode: append-only — every queued event flushes with its original `timestamp_ms`; storage's idempotency keys prevent dupes. The tablet does **not** locally re-issue grants offline; if the case grant has expired and connectivity is gone, intervention logging continues into a local buffer and the operator-side records DB is the authoritative record until OHDC catches up. |

### 1.3 Auth model on the tablet

- **Operator OIDC** for the responder (paramedic) at shift-in. Standard OAuth2 / OIDC against the operator's IdP. Token lives in Android EncryptedSharedPreferences / iOS Keychain.
- **Optional responder cert** — if the operator runs the per-shift responder-cert layer (per [`spec/emergency-trust.md`](spec/emergency-trust.md) "Per-responder cert"), the tablet exchanges the operator OIDC token for a 1–4h responder cert and includes it in `cert_chain_pem` of every `EmergencyAccessRequest`. Cert key is generated on device; private key never leaves the secure element.
- **Active case grant tokens** — issued by the patient phone after break-glass, scoped to one case, expire on case close. Memory-only; not persisted to disk on the tablet.

### 1.4 Tablet device-management expectations (operator's responsibility)

Per [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) "Security":

- Full-disk encryption.
- MDM / remote wipe.
- Biometric or PIN lock.
- Rapid roster sync at shift-out (revoke OIDC tokens, kill responder certs).

The tablet app does NOT enforce these — it expects them from the OS / operator's MDM. The app does provide a "panic logout" action that drops in-memory grants and operator OIDC tokens.

### 1.5 Skeleton in this repo

The current `tablet/` skeleton renders a single placeholder Compose screen. Reference layouts for discovery / patient view / intervention forms are pinned in [`spec/screens-emergency.md`](spec/screens-emergency.md) and the actual UI is to be built in subsequent implementation phases.

## 2. Dispatch console — operator-side web app

### 2.1 Form factor

- Vite + React + TypeScript SPA. Served by Caddy alongside the relay on the operator's domain.
- Operator-OIDC authenticated; same IdP the tablets use.
- Talks to: (a) the operator records database (Postgres) directly via a small backend (TBD; out of scope for the SPA skeleton), (b) the relay's audit endpoint for break-glass logs, (c) OHDC for read access to active-case event timelines (using the dispatcher's grant or read-only access via the operator's authority on the case span).

### 2.2 Routes / screens

| Route | Purpose | Spec ref |
|---|---|---|
| `/active` | Active case board: every case in flight, crew, location, status, time-elapsed. | [`spec/screens-emergency.md`](spec/screens-emergency.md) "Dispatch console" |
| `/active/:caseId` | Case detail: timeline, audit, current crew, reopen-token issuance, escalation. | same |
| `/roster` | Crew roster: who is on duty, who's in a case, who's available. | same |
| `/audit` | Filterable audit log of every break-glass event by the operator. Shows responder, target patient (anonymized to a per-case label), initiated/granted/denied status, what was accessed, what was written. | [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) "Audit visibility" |
| `/records` | Operator-side records DB UI: search past cases, view incident records, billing detail, follow-up actions. Read access depends on operator role. | [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) "Operator-side records" |

### 2.3 Reopen tokens

When a case auto-closes from inactivity (the responder lost connectivity, forgot to handoff, etc.) and the crew needs back in, the dispatcher issues a **case reopen token** (term defined in [`../spec/docs/glossary.md`](../spec/docs/glossary.md)) from `/active/:caseId`. The token is a relay-issued bearer with TTL; the field crew enters it on the tablet to resume the case without re-running break-glass. Issuance and validation live on the relay; the dispatch UI is just the issuer-side surface.

## 3. Emergency MCP

### 3.1 Form factor

- Node + TypeScript, using `@modelcontextprotocol/sdk`. Streamable HTTP transport (deployed by the operator alongside the relay) and stdio transport (for local responder-laptop installs).
- Authenticated by an operator-side OIDC bearer (or by a pre-shared key for stdio installs). Per-call, the MCP attaches the **active case grant** the operator selects via `set_active_case(case_id)` (analogous to Care MCP's `switch_patient`).

### 3.2 Tools (intentionally narrow — emergencies are time-critical)

Per [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) "OHD Emergency MCP":

| Tool | Purpose | OHDC operations under the hood |
|---|---|---|
| `find_relevant_context_for_complaint(complaint: string)` | Pulls the data slices that matter for the chief complaint. "Chest pain" → recent BP/HR + cardiac meds + cardiac history. "Possible OD" → drugs taken + history + allergies. | Several `query_events` calls scoped by event_type / channel against the case grant. |
| `summarize_vitals(window: string)` | Aggregate vitals across a recent time window: averages, trends. | OHDC aggregate / summarize endpoint over the case grant. |
| `flag_abnormal_vitals()` | Flag readings outside normal ranges given patient's baseline + known conditions. | `query_events` for vitals + a small server-side classifier. |
| `check_administered_drug(drug_name: string, dose: string)` | Check the candidate drug against the patient's current medications and known allergies. Returns interactions, contraindications, or "no flags". | `query_events` for current meds + allergy events + a drug-interaction lookup (operator-provided dataset). |
| `draft_handoff_summary()` | Produces a structured handoff summary for the receiving ER. | Aggregates the case timeline (events recorded by the crew + initial profile) into a structured summary. |

The MCP does NOT expose generic OHDC `query_events` / `put_events`. That's deliberate scope — the LLM is a triage assistant, not an exploratory analytics tool.

### 3.3 No-PHI-to-external-LLMs config knob

Per [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) "Security":

- Operator can set `OHD_EMERGENCY_MCP_ALLOW_EXTERNAL_LLM=false`. When false, the MCP server refuses tool calls coming from clients whose origin is outside the operator's allowed list (typically: only self-hosted local LLMs).
- Default: false (conservative). Operators who want to use Claude / GPT must explicitly opt in and accept that PHI may flow to the external LLM.

## 4. CLI — `ohd-emergency`

### 4.1 Form factor

- Rust binary with `clap` v4 derive API.
- Distributed alongside the dispatch / relay deployment for the operator's sysadmins.
- Mostly used for: cert lifecycle (refresh, rotation, audit), roster operations, audit log queries, case archive export for legal review.

### 4.2 Subcommand surface (initial stubs)

| Subcommand | Purpose | Notes |
|---|---|---|
| `ohd-emergency cert refresh` | Trigger a manual refresh of the org's daily Fulcio cert. Normally automatic. | Calls the relay's cert-refresh endpoint. |
| `ohd-emergency cert show` | Print the current authority cert chain (subject, issuer, validity). | Reads from the relay's `/healthz/cert` or equivalent. |
| `ohd-emergency cert rotate-key` | Rotate the org's daily-refresh keypair. | Generates new keypair, registers with Fulcio, drops old. |
| `ohd-emergency roster list` | List currently-on-duty responders. | Queries the operator's IdP via the relay. |
| `ohd-emergency roster revoke <user>` | Force-revoke a responder's access (departed, lost device). | |
| `ohd-emergency audit query --since <ts> --responder <id>` | Query the operator-side audit log. | |
| `ohd-emergency case-export <case_id> --out <file>` | Export a case (events + audit + operator records) for legal / regulatory review. | Writes a tar.gz with the case's OHDC-side events, the operator-records-side rows, and the audit. |

The current `cli/` skeleton wires these as `clap` stubs — they print "not yet implemented" and exit 0. The real implementations land alongside the relay and operator records DB work.

## 5. Operator-side records — data model placeholder

Per [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) "Operator-side records":

- The operator's database (Postgres in the reference deployment) holds the EMS station's own records of cases — separate from the patient's OHD record. Stays **outside** the OHD protocol.
- Persists independently of the OHD case after it ends. Subject to the operator's retention policy and regulatory regime (HIPAA / GDPR / etc.). Patient revocation of the OHD grant does NOT retroactively delete operator records.

A documented schema for the reference deployment is an open item. Sketch (placeholder; the actual schema is to be designed during implementation):

```sql
-- operator_records.cases — one row per OHD case the operator was the authority for
CREATE TABLE cases (
  id                BIGSERIAL PRIMARY KEY,
  ohd_case_ulid     BYTEA NOT NULL UNIQUE,        -- the OHD-side case_ulid (for cross-ref)
  patient_label     TEXT,                         -- operator's local label for the patient
  opened_at         TIMESTAMPTZ NOT NULL,
  closed_at         TIMESTAMPTZ,
  authority_label   TEXT NOT NULL,                -- e.g. "EMS Prague Region"
  predecessor_id    BIGINT REFERENCES cases(id),
  successor_id      BIGINT REFERENCES cases(id),
  auto_granted      BOOLEAN NOT NULL DEFAULT FALSE,
  scene_lat         DOUBLE PRECISION,
  scene_lon         DOUBLE PRECISION,
  destination       TEXT,                         -- receiving facility on handoff
  billing_status    TEXT
);

-- operator_records.responders — who was on duty for which case
CREATE TABLE case_responders (
  id           BIGSERIAL PRIMARY KEY,
  case_id      BIGINT NOT NULL REFERENCES cases(id),
  responder_id TEXT NOT NULL,                     -- operator-IdP subject
  responder_label TEXT,                           -- mirrored from responder cert CN at the time
  joined_at    TIMESTAMPTZ NOT NULL,
  left_at      TIMESTAMPTZ
);

-- operator_records.interventions — operator's local copy of intervention events
-- (the canonical record is in OHD Storage; this is the redundant operator copy)
CREATE TABLE interventions (
  id             BIGSERIAL PRIMARY KEY,
  case_id        BIGINT NOT NULL REFERENCES cases(id),
  ohd_event_ulid BYTEA NOT NULL,
  kind           TEXT NOT NULL,                   -- 'vital' | 'drug' | 'observation' | 'note'
  payload_jsonb  JSONB NOT NULL,
  recorded_at    TIMESTAMPTZ NOT NULL,
  recorded_by    TEXT NOT NULL                    -- operator-IdP subject
);

-- operator_records.audit — every break-glass / read / write the operator's responders did
CREATE TABLE audit (
  id           BIGSERIAL PRIMARY KEY,
  case_id      BIGINT REFERENCES cases(id),
  responder_id TEXT NOT NULL,
  action       TEXT NOT NULL,                     -- 'break_glass_initiated' | 'read' | 'write' | 'handoff' | 'reopen' | ...
  details_jsonb JSONB,
  ts           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- operator_records.reopen_tokens — tracker for issued reopen tokens
CREATE TABLE reopen_tokens (
  id          BIGSERIAL PRIMARY KEY,
  case_id     BIGINT NOT NULL REFERENCES cases(id),
  token_hash  BYTEA NOT NULL UNIQUE,
  issued_by   TEXT NOT NULL,
  issued_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  expires_at  TIMESTAMPTZ NOT NULL,
  used_at     TIMESTAMPTZ
);
```

This is a **placeholder** — the final schema will be pinned during implementation, alongside the dispatch console's records UI and `ohd-emergency case-export` formatting. The intent is to document that an operator-side DB exists and is part of the deployment, not to fix its exact shape now.

Encryption-at-rest expectations: per [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) "Security", Postgres on top of LUKS or equivalent.

## 6. Trust boundary

| Capability | Where it lives |
|---|---|
| Authority cert (operator's daily Fulcio-issued X.509) | `../relay/` (in HSM / secure storage) |
| Responder cert (per-shift, optional) | `../relay/` issues; `tablet/` holds in secure-element-backed keystore |
| Active case grant tokens | `tablet/` and `mcp/` memory only; never disk |
| Operator records DB credentials | `dispatch/` backend + `cli/` (separately scoped roles) |
| Operator OIDC tokens for responders | `tablet/` Android EncryptedSharedPreferences / iOS Keychain |
| Patient OHDC data (event payloads) | NEVER persisted by Emergency. Cached in tablet RAM during active case; flushed on case close. Operator records DB holds ONLY the operator's intervention copies. |

The tablet's compromise blast radius: lost / stolen tablet exposes (a) operator OIDC token (revocable on roster), (b) any in-memory case grants (limited to currently-active case), (c) cached active-case data (current case only). It does NOT expose: other patients, historical cases, the operator's authority cert.

The relay's compromise blast radius: any cert-signed emergency request the relay can produce up to the cert's natural 24h expiry. Mitigation: stop refreshes; wait 24h. See [`spec/emergency-trust.md`](spec/emergency-trust.md) "Revocation" for the rare "kill cert now" lever.

## 7. Security checklist (what Emergency must enforce vs. delegate)

Cross-referenced against [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md) "Security":

| Concern | Enforced by | Notes |
|---|---|---|
| Authority cert protection | `../relay/` (HSM-backed) | Out of Emergency's scope. |
| Roster integrity (departed responders) | `../relay/` + operator IdP | Emergency's CLI + dispatch console expose roster-management UX, but the source of truth is the relay's roster sync. |
| Operator-side records encryption at rest | Operator infra (LUKS / cloud volume encryption) | Emergency assumes encrypted volumes; doesn't add app-level encryption. |
| LLM exposure (Emergency MCP) | `mcp/` | Default-deny external LLMs; explicit `OHD_EMERGENCY_MCP_ALLOW_EXTERNAL_LLM=true` to opt in. |
| Tablet device management | Operator MDM | Emergency app exposes a "panic logout" that drops in-memory grants and OIDC tokens. |
| Patient OHDC data not persisted on tablet | `tablet/` | Memory-only cache; flushed on case close, app background past N minutes, panic logout. |
| Audit independence | Both sides | Operator-side audit lives in operator records DB; patient-side audit lives in OHD Storage. They are NOT reconciled — they're independent witnesses. |

## 8. Smoke tests this skeleton is committed to

- `cli/`: `cargo build && cargo run -- --help` — must always pass. Verified at scaffold time.
- `tablet/`, `dispatch/`, `mcp/`, `deploy/`: smoke-test commands documented per-subdir; not run by the scaffold (operator must `npm install` / set up Android SDK / `docker compose` themselves).

## 9. Cross-references

- Component spec: [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md)
- Emergency trust (cert chain, signed requests, verification): [`spec/emergency-trust.md`](spec/emergency-trust.md) (local copy of `../spec/docs/design/emergency-trust.md`)
- Patient-side + responder-side screens: [`spec/screens-emergency.md`](spec/screens-emergency.md) (local copy of `../spec/design/screens-emergency.md`)
- Relay (emergency-authority mode): [`../spec/docs/components/relay.md`](../spec/docs/components/relay.md)
- OHDC protocol: [`../spec/docs/design/ohdc-protocol.md`](../spec/docs/design/ohdc-protocol.md)
- Glossary: [`../spec/docs/glossary.md`](../spec/docs/glossary.md)
