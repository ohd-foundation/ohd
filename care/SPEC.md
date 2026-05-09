# OHD Care — Implementation Spec (v0.1)

> Implementation-ready spec for the OHD Care reference clinical app, distilled from [`spec/docs/components/care.md`](../spec/docs/components/care.md) and [`spec/docs/design/care-auth.md`](../spec/docs/design/care-auth.md). This document is the contract the `web/`, `mcp/`, `cli/`, and `deploy/` directories implement against.
>
> Where this spec is silent, defer to the canonical spec. Where the canonical spec is silent, follow conservative-by-default conventions (deny over allow, queue over auto-commit, audit-everything).

## 1. Overview

OHD Care is a **per-patient grant-token vault with a clinical UI on top**. Operators (clinicians, nurses, admins, auditors) authenticate to Care via the operator's OIDC; Care holds N patient grants (one per patient who's granted the operator access); when an operator opens a patient, Care uses that patient's grant token to talk OHDC against that patient's storage.

Three forms ship in this repo:

- **`web/`** — operator-facing SPA (Vite + React + TypeScript).
- **`mcp/`** — Care MCP (Node + TypeScript, `@modelcontextprotocol/sdk`) for LLM-driven multi-patient workflow.
- **`cli/`** — `ohd-care` (Python + click, packaged with `pyproject.toml`).

All three share the same auth and grant-vault model defined here; v1 has per-form copies of the wiring rather than a shared library.

## 2. Auth model

### 2.1 Two layers

| Layer | Who authenticates | Verifier |
|---|---|---|
| **Operator → Care** | The clinician (operator) logging into Care | Care's auth, via the clinic's OIDC SSO |
| **Care → patient's storage** | Care itself, on behalf of the operator, per patient | The patient's storage, via grant token (`ohdg_…`) |

Patient's storage **never sees the clinician's identity directly** — only the grant token. Operator identity is operator-side metadata that Care binds into its own audit. See `care-auth.md` "Two-sided audit".

### 2.2 Operator session

- Token shape: opaque `ohdo_…` prefix, base64url body. Distinct from `ohds_…` patient sessions.
- Default lifetime: **30 min access, 8 h refresh** (clinical-environment hijack risk is high; deployments may shorten further).
- Stored in Care's system DB: `care_operator_sessions` (schema in `care-auth.md` §"Operator session").
- Each session pins **one active patient grant** at a time via `last_active_grant_id`. Switching patient is an explicit operator action.

### 2.3 Operator user table

- Care maintains its own `care_operator_users` table — separate from any patient identity.
- `role ∈ {clinician, nurse, admin, auditor}`. Care UI gates by role; OHDC's grant-write rules in patient storage enforce again from the other side. Custom per-deployment roles are deferred to v2.
- Operator email / display name is stored plaintext (operator's own data; nothing private about it relative to OHD).
- Staff turnover: row goes `active=0`, sessions revoked, patient grants **stay** (they belong to the operator/clinic, not the individual). Audit history retained against the (now-inactive) operator id.

### 2.4 Patient grant vault

Per-patient grants live in `care_patient_grants` (schema in `care-auth.md` §"Per-patient grant-token vault"). Key fields:

- `patient_label` — operator-typed local identifier ("Alice (DOB 1985-04-12)"). Never sent to patient storage.
- `patient_storage_url` — rendezvous URL (relay-mediated) or direct.
- `storage_cert_pin` — SHA-256 of expected TLS cert when self-signed via relay.
- `grant_token_encrypted` — `ohdg_…` encrypted with Care's deployment KMS. **Mandatory; no plaintext at rest.**
- `case_ulids_remote_json` — when the grant is case-bound, the remote case ULIDs the grant references.
- `revocation_detected_ms` — set on first 401 after patient revoke; row stays for audit.

KMS choices by deployment: cloud KMS (AWS / GCP / Vault Transit) for cloud Care; HSM for in-clinic; PBKDF2-from-passphrase local key file for solo-practitioner Care. The OHD project does not dictate; the contract is "key lives outside the application database."

### 2.5 Grant import paths

1. **Patient-issued share (standard)** — patient generates share artifact in Connect (URL or QR with `(token, rendezvous_url, cert_pin, optional case ULIDs)`); operator pastes/scans into Care; Care decrypts, runs `Auth.WhoAmI`, persists.
2. **Patient-curated case grant** — patient prepares a case at home (e.g., "Headaches — visit Dr. Smith 2026-05-15"), links relevant events, issues a case-bound grant. Care's per-patient view explicitly signals "Curated visit — N events linked", showing the contents up front.
3. **Operator-initiated request** — deferred to v2 (`future-implementations/operator-grant-request.md`).

### 2.6 Token lifecycle

Care does **not** refresh grant tokens. The token is valid until patient storage says otherwise:

- `401 EXPIRED` — Care marks the row, prompts the operator to ask the patient for a renewed grant. Care surfaces 7-day expiry warnings.
- `401 CASE_CLOSED` — case-bound grant; case has closed.
- `401 REVOKED` — patient revoked from Connect; `revocation_detected_ms` set; cache for that grant evicted; UI shows "Patient revoked your access".
- `429 RATE_LIMITED` — transient; back off and retry.

Care **never silently drops tokens** — the row stays in the vault as a record of the relationship.

### 2.7 Cache policy

- Keyed by `(grant_id, query_hash)`.
- Default TTL: 60 s when patient panel is open; 0 when closed.
- Encrypted at rest with the same KMS as tokens.
- Dropped immediately on revocation detection.
- Per-deployment knob to set TTL = 0 globally for strict-no-cache deployments.

## 3. Roster and per-patient view (web)

### 3.1 Roster

- Lists every patient in `care_patient_grants` with `revocation_detected_ms IS NULL`.
- Status indicators: last-visit timestamp, recent flags ("missed doses", "BP trending up"), grant expiry warning.
- Search by `patient_label`. No cross-patient queries return PHI.
- "Add patient" button → paste/scan share artifact (path 1 above).

### 3.2 Per-patient view

Header (always visible while in patient context):

- **Active patient name + label** — prominent, on every screen, in every modal. Distinct visual treatment so an operator never accidentally writes against the wrong patient.
- Active grant scope (read scope, write scope, approval mode, expiry).
- Active case (if any) — start time, current authority, predecessor chain link.

Tabs (driven by query against the active patient's grant; rows silently filtered get a count badge):

- Timeline, Vitals, Medications, Symptoms, Foods, Labs, Imaging, Notes.

Visit panel:

- Previous-visit summary.
- Assessment input (free text + structured fields).
- Write-back (queues per the grant's `approval_mode`).
- Visit-prep brief at top: time series + meds + recent symptoms + flags.

Audit panel:

- What queries the operator has made on this patient (cross-references the patient-side audit via shared `query_hash`).
- Explicitly notes "the patient sees this same audit on their side."

### 3.3 Multi-patient context safety (mandatory)

These rules are non-negotiable for v1:

1. **Active patient prominently displayed everywhere** — UI header, MCP system prompt, CLI prompt prefix.
2. **Patient switch requires explicit operator action.** No automatic switch from search results, LLM intent, or URL navigation.
3. **Submission tools include a confirmation step** showing "Submitting to <Active Patient> — confirm?" with the patient's label rendered as the operator typed it.
4. **Every cross-patient action is audited.** Switches, rapid-switch sequences, and submission attempts all land in `care_operator_audit`.

Failing any of these is a release blocker.

## 4. Cases

Cases are first-class. The Care UI and MCP both surface cases. Every patient view shows whether there's an active case.

### 4.1 Case operations

- **`open_case`** — operator opens a case at the start of an encounter. Picks `case_type` (admission / outpatient / ongoing-therapy / emergency-inherited), sets a label, optionally sets `predecessor_case_ulid` (e.g., the EMS case that brought the patient in). Storage records a `case_started` marker.
- **`close_case`** — operator closes the case (discharge, end-of-shift handoff, end of outpatient visit). Storage records a `case_closed` marker. Operator retains read-only access to the case's span (filters at close time) for records / billing / follow-up.
- **`handoff_case`** — convenience wrapper: opens a successor case for the next authority with the current case set as `predecessor_case_ulid`, then closes the current case. Used for EMS → admission, shift handoff, transfer to specialty.
- **`list_cases`** — list cases for the active patient.

### 4.2 Predecessor / parent inheritance

(Per spec; intrinsic to case linkage, not a grant-level flag.)

- **Successor → predecessor**: asymmetric. A successor case (`predecessor_case_ulid` set) automatically reads the predecessor's scope. Handoff context flows forward.
- **Parent → children**: a parent case automatically reads its children's scopes. Sub-task results roll up (e.g., EKG referral results visible to the doctor who ordered it).
- **Children do NOT see parent's broader scope.** Predecessors do **not** see successors.
- **To pass parent scope down to a child**: link the same case as both `parent_case_ulid` and `predecessor_case_ulid`.

### 4.3 Case-resolved writes

Operator-submitted events are written into `events` normally — no `case_id` tagging on the write. The case's filters pull them in at read time, typically via `device_id_in: [operator_device]` filter plus the case's time range. This means: the EMS EKG flows into the hospital admission case automatically because the admission case has the EMS case as predecessor; the predecessor chain pulls those events in.

### 4.4 Auto-close + reopen

If the operator forgets to close a case, it auto-closes after the inactivity threshold for its `case_type` (admission default 30 d). Care surfaces an auto-close warning in the operator's view N hours before the threshold. On auto-close the operator gets a **reopen token** (default TTL 24 h) that lets them reopen without re-running break-glass / re-asking the patient for a grant when the case wasn't really finished.

### 4.5 Patient force-close

The patient can force-close any case from OHD Connect at any time. This revokes the operator's grant for that case scope. Care detects on the next call (`401 REVOKED` or `401 CASE_CLOSED`), evicts cache, marks the grant row revoked, and surfaces the change to the operator.

### 4.6 Retrospective access

Once a case is closed, the patient can issue case-scoped grants to specialists / insurers / researchers for review. Care imports these the same way as any case-bound grant; the per-patient view headers "Retrospective: <Case Label>" with the curated-events badge.

## 5. Visit prep

When the operator opens a patient (web UI) or invokes `find_relevant_context_for_complaint(...)` (MCP), Care:

1. Pulls relevant time series (most recent + trend) for likely-related channels.
2. Pulls recent medication adherence.
3. Pulls recent related symptoms / food / activity.
4. Pulls the previous-visit summary, if any.
5. Surfaces flags: "BP trending up over last month", "missed 4 of last 14 doses", "new symptom in last 48h".

Renders as a one-screen brief at the top of the patient view. The brief is cached per `(grant_id, query_hash)` per §2.7.

## 6. Write-with-approval

### 6.1 Mechanics

Each submission is a typed event with proper channels (`clinical_note`, `lab_result`, `medication_prescribed`, `referral`, etc.) and goes through OHDC `put_events` against the active patient's grant. The grant's `approval_mode` policy determines what happens:

| Mode | Behavior |
|---|---|
| `always` | Every submission queues for patient review (default for new grants). |
| `auto_for_event_types` | Pre-authorized types auto-commit (e.g., `lab_result`, `clinical_note`); others queue. Trust-tiered for established relationships. |
| `never_required` | All submissions auto-commit (used for trusted long-term grants and emergency / break-glass). |

Submitted events appear in Care's "pending" status until the patient approves (or the policy auto-commits).

### 6.2 Source signing

Each operator submission is signed with the operator's identity (clinic-level + individual). The patient's pending-review UI shows "signed by <operator>". Forgery requires operator key compromise — operator-side concern.

### 6.3 Submission UI / MCP / CLI contract

All three forms must:

1. Render the active patient's label in the confirmation step ("Submitting to Alice (DOB 1985-04-12) — confirm?").
2. Show the grant's approval mode and expected outcome ("Will queue for patient approval" vs. "Will auto-commit").
3. Audit on submit attempt (success or rejection) into `care_operator_audit`.

## 7. Two-sided audit

### 7.1 Patient-side audit

Standard OHDC audit — `actor_type='grant'`, `grant_id` recorded. Patient sees in Connect: *"Dr. Smith's clinic queried your data 12 times today: 8 reads (47 events returned, 3 filtered), 3 writes pending review."*

The patient does **not** see *which clinician* did the access — only that the operator's grant was used.

### 7.2 Operator-side audit (`care_operator_audit`)

Schema in `care-auth.md` §"Operator-side audit". Records `(ts_ms, operator_user_id, patient_grant_id, ohdc_action, ohdc_query_hash, result, rows_returned, rows_filtered, http_status, ip, ua)`.

This is what answers "who did what" inside the clinic. Compliance / auditors / security review.

### 7.3 Joining the two

The `query_hash` is the canonical request hash and matches across both audit logs. Operators can produce the join when subpoenaed or when the patient asks. The patient's audit pre-existed and is unforgeable from the operator's side.

## 8. Talking to patient storage

- Each `care_patient_grants` row carries `patient_storage_url`. Relay-mediated patients have a rendezvous URL `https://<relay>/r/<rendezvous_id>`; directly-reachable storage has its own URL.
- Care uses the same `Authorization: Bearer ohdg_...` and Connect-RPC for both. Only the URL differs.
- For relay-mediated patients: the patient's phone has to be reachable (foreground) or wakable (push-token registered). First call may take a few seconds while the tunnel re-establishes; subsequent calls within the wake window are fast. Care surfaces a "syncing…" indicator on the first request.
- If the patient's phone is offline: surface "Patient is offline; data may be stale" and serve the cache (within TTL).
- `cert_pin` from the grant artifact is enforced by Care's HTTP client; mismatch fails closed with a security warning to the operator.

## 9. OHDC operations Care uses

Subset of `Auth.*` callable by grant tokens (vs. self-session-only):

- **`Auth.WhoAmI`** — first import, populate metadata (display name, scope mirror).
- **`Auth.PutEvents`** (via grant token) — write-with-approval submissions.
- **`Auth.QueryEvents` / `Auth.QueryLatest` / `Auth.Summarize` / `Auth.Correlate` / `Auth.FindPatterns` / `Auth.Chart`** — read tools, per grant scope.
- **`Auth.OpenCase` / `Auth.CloseCase` / `Auth.ListCases`** — case operations.

Care does **not** get `Auth.ListSessions`, `Auth.LinkIdentity`, etc. — those are self-session-only.

Care-internal operations (don't go to patient storage):

- `Operator.ImportGrant(share_artifact)` — validates artifact, decrypts token, runs `Auth.WhoAmI`, persists.
- `Operator.RevokePatientGrant(grant_row_id)` — Care decided to drop the patient (the patient hasn't revoked); marks local row revoked-by-operator.
- `Operator.RotateOperatorSession` — refresh operator session token.

## 10. Care MCP — tool catalog

The MCP runs alongside the web app. Operator authenticates via OAuth proxy; the MCP holds the operator session token and routes per-patient operations through the active grant.

### 10.1 Patient management

- `list_patients` → `[{label, last_visit_ms, grant_status, active_case_label?}]`.
- `switch_patient(label)` → sets the active grant in the MCP session. Idempotent; no auto-switch from intent.
- `current_patient` → diagnostic; returns the active patient label + grant scope.

### 10.2 Read tools (active patient, gated by read scope)

- `query_events(filter)`
- `query_latest(event_type, count)`
- `summarize(event_type, period, aggregation, from_time?, to_time?)`
- `correlate(event_type_a, event_type_b, window_minutes, from_time?, to_time?)`
- `find_patterns(event_type, description, from_time?, to_time?)`
- `chart(description)` → `{image_base64, chart_spec, underlying_data}`
- `get_medications_taken(from_time?, to_time?, medication_name?)`
- `get_food_log(from_time?, to_time?, include_nutrition_totals?)`

### 10.3 Write-with-approval tools (active patient, gated by write scope)

Each of these renders a confirmation in the LLM session before invoking storage:

- `submit_lab_result(result_data)`
- `submit_measurement(measurement_type, value, unit, timestamp?)`
- `submit_observation(observation_data)`
- `submit_clinical_note(note_text, about_visit?)`
- `submit_prescription(medication, dose, schedule, duration)`
- `submit_referral(specialty, reason, referred_to?)`

### 10.4 Workflow tools

- `draft_visit_summary()` → patient-readable summary the operator reviews and submits.
- `compare_to_previous_visit()` → narrative diff.
- `find_relevant_context_for_complaint(complaint)` → pulls visit-prep slices.

### 10.5 Case tools

- `open_case(case_type, label, predecessor_case_ulid?, parent_case_ulid?)`
- `close_case()`
- `handoff_case(to_authority, case_type, label)`
- `list_cases()`

### 10.6 MCP-specific safety rules

- Active patient **must** appear in the system prompt and in every tool result for orientation.
- `switch_patient` is the only tool that changes active context. No other tool may infer or change it.
- All write tools render `Submitting to <Active Patient> — confirm?` and require an explicit affirmative tool-call argument before invoking storage.

## 11. CLI surface

`ohd-care` is the terminal interface for scripts and operators who prefer the keyboard:

```sh
ohd-care patients
ohd-care use alice
ohd-care temperature --last-72h
ohd-care submit observation --type=respiratory_rate --value=18
ohd-care submit clinical-note --about="visit 2026-05-07" < notes.txt
ohd-care pending list
```

Same auth model — operator session via `oidc-callback` flow into `~/.config/ohd-care/`; switches active grant via `use <label>` (writes `last_active_grant_id` to local state).

## 12. Trust boundary

OHD Care holds **grants**, not patient data. When a patient revokes:

- Local cache for that patient is evicted immediately.
- The `care_patient_grants` row stays (audit history) with `revocation_detected_ms` set.
- The operator's local cache (anything still on disk in encrypted form) becomes the operator's HIPAA / GDPR responsibility from that point — snapshot from a moment when access was authorized; retained per the operator's posture.

OHD Care does **not** reach into the patient's storage; it speaks OHDC. The patient's revocation is immediate; no cached secret persists at the operator's side beyond the grant's life.

## 13. What Care does NOT do (deliberate scope boundaries, not deferred)

- **No billing / coding / claims.**
- **No scheduling / calendar.**
- **No HL7 / FHIR mapping suite.** Bridges to / from FHIR are separate components.
- **No insurance / payer integration.**
- **No DICOM imaging viewer** — image attachments referenced; rendered by external viewer.
- **No prescription delivery to pharmacies** — Care submits a `medication_prescribed` event; pharmacy integration is a separate OHDC consumer.
- **No cohort / population queries.** Per-patient only in v1; cohort-grant model deferred.
- **No operator-to-operator handoff (warm referral)** in v1 — both providers temporarily holding grants is supported, but UX-level "introduce operator B with the patient's blessing" is deferred.
- **No localization** in v1 — English only; locale-aware versions deferred.

A practice that needs these either (a) runs their existing system alongside Care, or (b) builds the missing piece as a separate OHDC consumer.

## 14. Security posture

- **Operator authentication**: OIDC against the operator's IDP. No password storage in Care.
- **Patient grant tokens**: encrypted at rest with deployment KMS. Lost-laptop scenario is the operator's responsibility (device management, encryption at rest, remote wipe).
- **Multi-patient isolation**: every operation runs against the active patient's grant; cross-patient operations require explicit re-context. No "patient roster query that returns multiple patients' data".
- **LLM exposure** (Care MCP): the deployment chooses its LLM. For sensitive deployments, self-hosted models keep PHI in-house. Care exposes a `no_phi_to_external_llms` config knob; deployments running on cloud LLMs accept the tradeoff for their patient population.
- **Source signing for clinical writes**: per §6.2.

## 15. Open design items (deferred to v2+)

Tracked in this section so v0.1 doesn't accidentally close them off:

- **Operator-side caching policy** beyond the simple TTL — stale-while-revalidate for offline operation, cache invalidation when the patient submits new events, etc.
- **Cross-patient features** — population-level queries across the operator's panel ("show me adherence trends across my diabetic patients"). Different scope model needed.
- **Operator-to-operator handoff** UX — warm-referral flow.
- **Break-glass UX** — when Care needs to onboard an emergency-inherited case (handoff from OHD Emergency). Mostly handled by the predecessor-case mechanism, but the entry path needs explicit Care UI.
- **Localization** — clinical UI and terminology.
- **Per-tool MCP auth** — fine-grained scopes ("this tool requires `submit_lab_result` capability").
- **Operator anomaly detection** — patterns like "unusual volume of queries at 3 AM" detectable from `care_operator_audit`. Tooling layer.
- **Custom operator role catalog** — beyond clinician / nurse / admin / auditor.

## 16. Out of scope (won't ever be Care's job)

- Billing, scheduling, claims, payer integration, DICOM viewer, pharmacy delivery (per §13).
- Replacing Epic / Cerner / large enterprise EHRs.
- Being a generic CRUD app over OHDC — Care is purpose-built for the clinical workflow shape.

## 17. Cross-references

- Component spec: [`../spec/docs/components/care.md`](../spec/docs/components/care.md)
- Operator auth & vault (full schemas): [`../spec/docs/design/care-auth.md`](../spec/docs/design/care-auth.md)
- OHDC wire spec: [`../spec/docs/design/ohdc-protocol.md`](../spec/docs/design/ohdc-protocol.md)
- Privacy & access (grant scope rules): [`../spec/docs/design/privacy-access.md`](../spec/docs/design/privacy-access.md)
- Storage format (grants, event_case_links, audit_log): [`../spec/docs/design/storage-format.md`](../spec/docs/design/storage-format.md)
- Relay path: [`../spec/docs/design/relay-protocol.md`](../spec/docs/design/relay-protocol.md)
- Emergency handoff: [`../spec/docs/components/emergency.md`](../spec/docs/components/emergency.md)
- MCP catalog (Care-relevant sections): [`./spec/mcp-servers.md`](./spec/mcp-servers.md)
- Glossary: [`../spec/docs/glossary.md`](../spec/docs/glossary.md)
- UX vocabulary: [`../ux-design.md`](../ux-design.md)
