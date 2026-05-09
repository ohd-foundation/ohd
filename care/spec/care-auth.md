# Design: OHD Care — Operator Auth & Grant Vault

> How OHD Care authenticates clinicians, holds per-patient grant tokens, talks to each patient's storage, and binds operator identity into the audit trail. Covers all auth mechanics specific to the operator side; doesn't try to spec Care's full clinical workflow (that's in [`../components/care.md`](../components/care.md)).

## What Care is, from an auth perspective

Care is essentially a **per-patient grant-token vault with a clinical UI on top**. The clinician logs into Care; Care holds N patient grants (one per patient who's granted the operator access); when the clinician opens a patient, Care uses that patient's grant token to talk OHDC against the patient's storage.

Two distinct auth layers, often confused:

| Layer | Who's authenticating | Who's verifying |
|---|---|---|
| **Operator → Care** | The clinician (operator) logging into Care | The Care app, via the clinic's OIDC SSO |
| **Care → patient's storage** | Care itself, on behalf of the clinician, per patient | The patient's storage, via grant token |

The patient's storage **never sees the clinician's identity directly** — it only sees the grant token. The clinician's identity is operator-side metadata that Care binds into its own audit. Together with the patient-side audit (which records `grant_id`), it's possible to answer "which clinician did what" by joining the two. This is the audit-binding contract; see "Two-sided audit" below.

## Operator authentication into Care

The clinician logs into Care once per shift / device / session. Care uses **OIDC** for this, same primitives as patient self-session (see [`auth.md`](auth.md)) — different tenant, different intent.

### OIDC providers per Care deployment

A Care deployment chooses which OIDC providers to enable based on the operator's identity infrastructure:

| Care deployment | Typical providers |
|---|---|
| Hospital department | Hospital ADFS / Entra (clinic SSO) |
| Small clinic | Google Workspace, Microsoft 365 |
| Solo practitioner | Personal Google / Apple |
| Mobile / ambulance crew | Operator-issued SSO (EMS station, dispatch service) |
| Clinical trial site | Sponsor-supplied SSO, or per-site Authentik / Keycloak |
| Direct-pay / boutique | Whatever the practice already uses for its other software |

The Care-side OIDC provider list is **operator-configurable** in Care's deployment config — same shape as the patient-side provider catalog from [`auth.md`](auth.md), but pointed at the clinic's identity infrastructure. The OHD project doesn't dictate which providers a Care deployment supports.

OHD Cloud running an OHD Care offering for small clinics ships a default catalog (Google Workspace, Microsoft 365, plus Authentik for self-hosted alternatives); larger clinics pin a specific in-house IDP.

### Operator session

The clinician completes OIDC against the clinic's IDP; Care's auth server issues a **Care operator session token** — same opaque-prefix-base64url shape as patient sessions (`ohdo_…` for "operator session", to keep them distinguishable), with sessions tracked in Care's own system DB:

```sql
CREATE TABLE care_operator_sessions (
  id                    INTEGER PRIMARY KEY AUTOINCREMENT,
  operator_user_id      INTEGER NOT NULL REFERENCES care_operator_users(id),
  access_token_hash     BLOB NOT NULL UNIQUE,
  refresh_token_hash    BLOB NOT NULL UNIQUE,
  oidc_provider         TEXT NOT NULL,
  oidc_subject          TEXT NOT NULL,
  device_label          TEXT,
  ip                    TEXT,
  ua                    TEXT,
  issued_at_ms          INTEGER NOT NULL,
  access_expires_ms     INTEGER NOT NULL,
  refresh_expires_ms    INTEGER NOT NULL,
  last_used_ms          INTEGER,
  last_active_grant_id  INTEGER,                   -- the patient currently in active context
  revoked_at_ms         INTEGER,
  revoked_reason        TEXT
);
```

Care operator sessions are **shorter-lived** than patient sessions by default — 30 minutes access, 8 hours refresh — because clinical environments have higher session-hijack risk (shared workstations, unattended browsers, stolen tablets). Operators can extend in deployment config; clinics with strong device management often dial it back further.

### Operator user table

Care maintains its own user table, separate from any patient identity:

```sql
CREATE TABLE care_operator_users (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  display_name      TEXT NOT NULL,                  -- "Dr. Anna Novak — primary care"
  email             TEXT,                            -- operator-side, full plaintext (operator's own data)
  oidc_provider     TEXT NOT NULL,
  oidc_subject      TEXT NOT NULL,
  role              TEXT NOT NULL,                   -- 'clinician' | 'nurse' | 'admin' | 'auditor'
  active            INTEGER NOT NULL DEFAULT 1,
  joined_at_ms      INTEGER NOT NULL,
  left_at_ms        INTEGER,                         -- on staff turnover
  UNIQUE (oidc_provider, oidc_subject)
);
```

Note that operator-side identity is **not anonymized** the way patient-side identity is. The operator's OIDC subject maps to a real human in the clinic's records — that's the whole point. The patient-side privacy contract still holds: the patient's storage never learns the clinician's identity directly.

The `role` field gates Care's UI / API: `auditor` can read but not submit; `nurse` may submit observations but not prescriptions; `clinician` has full grant-write scope; `admin` can configure Care but doesn't see patient data without a clinician role. Role enforcement is Care-side; OHDC's grant-write rules (in the patient's storage) enforce again from the other side.

### Staff turnover

When a clinician leaves the operator's organization:

1. Their `care_operator_users` row is set `active=0`, `left_at_ms` recorded.
2. All their `care_operator_sessions` are revoked (`revoked_reason='staff_left'`).
3. Patient grants the operator holds are **not revoked** — those belong to the operator (the clinic), not to the individual clinician. Other clinicians at the operator continue using them.
4. The departed clinician's audit history stays attached to their (now-inactive) `care_operator_users.id` — historical "who did what" is preserved.

If a patient wants to revoke when their specific clinician leaves (rather than the operator entirely), that's a regular grant-revocation from Connect — orthogonal to staff turnover on the operator's side.

## Per-patient grant-token vault

Care holds one grant token per patient who has granted the operator access. The token vault is the heart of Care's auth surface.

### Storage

Per-patient grants are stored in Care's system DB:

```sql
CREATE TABLE care_patient_grants (
  id                       INTEGER PRIMARY KEY AUTOINCREMENT,
  patient_label            TEXT NOT NULL,             -- operator-supplied: "Alice (DOB 1985-04-12)"
  patient_storage_url      TEXT NOT NULL,             -- where the grant resolves (rendezvous URL or direct)
  storage_cert_pin         BLOB,                       -- SHA-256 of expected TLS cert (when self-signed via relay)
  grant_token_encrypted    BLOB NOT NULL,             -- ohdg_... encrypted with Care's KMS
  grant_id_remote          BLOB NOT NULL,             -- the grant's ulid_random; informational
  case_ulids_remote_json   TEXT,                      -- if case-bound: JSON array of case ULIDs; empty/NULL = open-scope
  imported_at_ms           INTEGER NOT NULL,
  imported_by              INTEGER NOT NULL REFERENCES care_operator_users(id),
  expires_at_ms            INTEGER,                    -- mirror of patient-side expiry; informational
  last_used_ms             INTEGER,
  last_check_at_ms         INTEGER,                    -- last time Care verified the token still works
  revocation_detected_ms   INTEGER,                    -- set when patient revokes; Care saw 401 on next call
  notes                    TEXT                        -- operator's notes about this patient relationship
);

CREATE INDEX idx_care_grants_patient ON care_patient_grants (patient_label);
CREATE INDEX idx_care_grants_active  ON care_patient_grants (revocation_detected_ms) WHERE revocation_detected_ms IS NULL;
```

Tokens are **encrypted at rest** with Care's deployment KMS (AWS KMS, GCP KMS, HashiCorp Vault, hardware HSM, or — for self-hosted Care — a local key file with appropriate permissions). The encryption is mandatory; tokens never sit on disk in plaintext. Decryption happens on-demand inside the request handler when Care dispatches an OHDC call to the patient's storage.

`patient_label` is whatever the operator types — DOB, room number, internal patient ID, full name, etc. It's the operator's local-side identifier, never sent to the patient's storage. The storage knows the patient by their `user_ulid`, period; the label is for the clinician's brain.

### How grants get into the vault

Three paths:

#### 1. Patient-issued share (the standard path)

The patient creates a grant in OHD Connect, gets a share artifact (URL / QR code containing `(token, rendezvous_url, cert_pin, optional list of case ULIDs)` — see [`privacy-access.md`](privacy-access.md) "Creating a grant"), and hands it to the operator (in person, by email, by SMS, by reading aloud). The operator's clinician imports it into Care:

```
Care → Patients → Add patient → "Paste share URL or scan QR"
```

Care stores the encrypted token, fetches a `whoami`-style check from the patient's storage to verify the token works and learn the patient's metadata (display name they want to be known by, current allergies as a sanity-check), populates the `care_patient_grants` row. The operator labels the patient with whatever they need locally.

#### 2. Patient-curated case grant (the visit-prep pattern)

A standout v1 workflow: the patient prepares a case at home before the visit, links relevant events to it, then shares a case-bound grant with the operator. The case acts as a curated data subset — the operator sees only what the patient linked, not the entire record.

Example flow:

1. Patient (via Connect) creates a case "Headaches — visit Dr. Smith 2026-05-15."
2. Patient links relevant events: 7 headache symptom logs from the last month, sleep duration over the same window, recent BP readings, the medications they've tried, dietary notes around onset days.
3. Patient creates a case-bound grant referencing the case (single-case grant: `case_ulids: [<the case>]`). Default policies: read-only, expires 7 days after the visit, write-with-approval if they want the doctor to add findings.
4. Patient sends the share URL to the operator (email, secure-message, in-person QR).
5. Operator's clinician imports it into Care; Care stores the grant + the case ULID reference (`case_ulids_remote_json`).
6. When the clinician opens the patient in Care, the read scope is the case — only events the patient linked. No browsing the patient's history; no incidental disclosure of unrelated data.
7. After the visit, the case auto-expires (7 days), or the patient revokes, or the clinician's findings (added via write-with-approval) get linked into the case as additional context.

Why this pattern matters: it shifts visit prep from "the doctor digs through your history asking questions" to "you and your doctor look at exactly what you wanted to discuss." The privacy contract is stronger than time-windowed grants (which would still surface unrelated events in the same window) and richer than event-type filters (which can't distinguish "the headache symptoms relevant to this visit" from "every headache I've ever logged"). It's *content-curated scope*, made possible by the case primitive.

Care's UI signals this distinctly — when the active patient's grant is case-bound, the patient view headers something like *"Headaches — visit Dr. Smith"* with a curated-events badge, and a side panel listing every event the patient included. The clinician knows up front what they have, what they don't, and what to ask about if they need more.

#### 3. Operator-initiated request (deferred)

In some deployments, the operator wants to *request* a grant from a patient (e.g., new patient onboarding at a clinic — patient hasn't issued a grant yet, the clinic generates an "invite to grant" link). This is patient-side UX (Connect has to receive and act on the request) and out of scope for v1; sketched in `future-implementations/operator-grant-request.md` (TBD).

### Token lifecycle

Care doesn't refresh grant tokens — they don't refresh the way self-session tokens do. A grant token is valid until the patient's storage says it isn't:

- **Expiry**: If the grant has `expires_at_ms` set, the token is rejected after that. Care tracks the expiry mirror locally and warns the clinician 7 days before; when the date passes, attempts get `401 EXPIRED`. The clinician asks the patient for a renewed grant if the relationship continues.
- **Case-close**: If the grant is case-bound and the case closes (clinically, or because the patient closes it from Connect), subsequent attempts get `401 CASE_CLOSED`. Care marks the patient row stale and prompts the clinician.
- **Revocation**: If the patient revokes from Connect, the next call gets `401 REVOKED`. Care marks `care_patient_grants.revocation_detected_ms` and surfaces "Patient revoked your access" in the clinician's view. The local `care_patient_grants` row stays (for audit history), but no further OHDC calls are attempted.
- **Storage-side rate limit**: `429 RATE_LIMITED` is transient; Care backs off and retries.

Care **never silently drops tokens** even when revoked — the row stays in the vault as a record of the relationship having existed. The operator's HIPAA/GDPR posture decides retention from there.

### Cache policy

Care holds an in-memory + on-disk cache of recent OHDC responses for performance (visit prep multi-queries, charting). The cache:

- Is keyed by `(grant_id, query_hash)`.
- Has a default TTL of 60 seconds for active sessions, 0 (no cache) when the patient panel isn't open.
- Is dropped immediately on revocation detection (`revocation_detected_ms` set → cache for that grant_id evicted).
- Lives encrypted at rest with the same KMS that protects the tokens.
- Is operator-configurable (deployments with strict no-cache requirements set TTL=0).

The full cache-policy spec — including deeper considerations (stale-while-revalidate for offline operation, cache invalidation when the patient submits new events, etc.) — is its own design item, called out as "Open" in [`../components/care.md`](../components/care.md). The `auth.md`-relevant bit is just: cache lives in encrypted operator-side storage, dropped on revocation, never surfaces post-revocation data to the clinician.

## Two-sided audit

Every OHDC call from Care to a patient's storage produces audit entries on **both sides**.

### Patient-side audit (what the patient sees)

The patient's `audit_log` row records `actor_type='grant'` plus the `grant_id` of the Care grant. The patient sees in Connect: *"Dr. Smith's clinic queried your data 12 times today: 8 reads (47 events returned, 3 filtered), 3 writes pending review, 1 case-link added."*

The patient does **not** see *which clinician* at the operator did the access — only that the operator's grant was used. This is by design: the patient's privacy contract is with the operator (they granted the clinic, not Anna in particular). Operator-internal staff identity stays operator-internal.

### Operator-side audit (what the clinic sees)

Care records its own audit:

```sql
CREATE TABLE care_operator_audit (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_ms               INTEGER NOT NULL,
  operator_user_id    INTEGER REFERENCES care_operator_users(id),
  patient_grant_id    INTEGER REFERENCES care_patient_grants(id),
  ohdc_action         TEXT NOT NULL,         -- mirrors the OHDC operation: 'query_events', 'put_events', etc.
  ohdc_query_hash     BLOB,                   -- sha256 of the canonical request, for cross-reference with patient audit
  result              TEXT NOT NULL,          -- 'success' | 'partial' | 'rejected' | 'error'
  rows_returned       INTEGER,
  rows_filtered       INTEGER,
  http_status         INTEGER,
  ip                  TEXT,
  ua                  TEXT
);

CREATE INDEX idx_care_audit_time     ON care_operator_audit (ts_ms DESC);
CREATE INDEX idx_care_audit_operator ON care_operator_audit (operator_user_id, ts_ms DESC);
CREATE INDEX idx_care_audit_grant    ON care_operator_audit (patient_grant_id, ts_ms DESC);
```

This is what answers "who did what" inside the clinic. Compliance officers / auditors / security review use this to investigate incidents.

### Joining the two sides

To answer "which clinician at the operator initiated the query that landed in my audit at 14:32?", the patient and operator each hold one half:

- Patient knows: `(ts_ms, grant_id, query_hash, rows_returned, rows_filtered)`.
- Operator knows: `(ts_ms, operator_user_id, patient_grant_id, ohdc_query_hash, …)`.

The `query_hash` matches across both audit logs (same canonical request, same hash). Operators that need to demonstrate accountability to a patient (or a regulator) can produce the join — but not without the patient's knowledge, because the patient initiated the question. A subpoena to the operator can compel disclosure of `operator_user_id`; the patient's audit pre-existed and is unforgeable from the operator's side.

This is the design principle: **operator-internal identity is private to the operator until the patient or regulators demand otherwise; the audit on the patient's side is enough to detect access patterns, and the join enables accountability when needed**.

## Talking to patients' storage — the relay-aware path

Many patients run on-device storage (their phone) or self-hosted storage behind home NAT. Care has to talk to those too. The mechanism is the same as any other OHDC consumer accessing relay-mediated storage; see [`../components/relay.md`](../components/relay.md) for the full relay model.

Operationally for Care:

- Each `care_patient_grants` row carries a `patient_storage_url`. For relay-mediated patients this is the rendezvous URL `https://<relay>/r/<rendezvous_id>` from the grant artifact. For directly-reachable storage (cloud, custom-provider) it's the storage's own URL.
- Care doesn't distinguish between the two at the OHDC layer. Same `Authorization: Bearer ohdg_...`, same Connect-RPC calls. The only HTTP-level difference is which URL is hit.
- For relay-mediated patients, the patient's phone has to be reachable (foreground) or wakable (push-token registered). The clinician may experience first-call latency of a few seconds while the phone's tunnel re-establishes; subsequent calls within the wake window are fast. Care surfaces this transparently: a "syncing…" indicator on the first request, then normal.
- If the patient's phone is off / out of coverage, Care surfaces "Patient is offline; data may be stale" and serves the cache (within TTL). The clinician decides whether to proceed or wait.
- `cert_pin` from the grant artifact is enforced by Care's HTTP client; mismatch → fail-closed with a security warning to the clinician (likely a man-in-the-middle attempt).

## OHDC RPCs Care uses

A subset of the OHDC `Auth.*` RPCs from [`../components/connect.md`](../components/connect.md) is callable by grant tokens (vs. self-session-only). Notably:

- `Auth.WhoAmI` — returns the user's display name (the field they chose to share with grantees) and the grant's effective scope. Used by Care on first import to populate metadata.
- Care does **not** get `Auth.ListSessions`, `Auth.LinkIdentity`, etc. — those are self-session-only.

OHDC RPCs Care needs that aren't covered by self-session/grant/device:

- `Operator.ImportGrant(share_artifact)` — Care-internal, doesn't go to patient storage. Validates the artifact, decrypts the embedded token, fetches `Auth.WhoAmI` from the patient's storage to verify, persists to `care_patient_grants`.
- `Operator.RevokePatientGrant(grant_row_id)` — Care-internal, marks the local row revoked-by-operator (the patient hasn't revoked; the clinic has decided to drop them). The patient is **not** notified through OHDC because Care holds no channel to push to them; the patient sees the lack-of-future-queries from their audit log.
- `Operator.RotateOperatorSession` — refresh the operator session token; standard OAuth refresh flow.

## Token storage on the Care server (encryption)

Grant-token encryption at rest is a deployment concern, but the spec mandates the contract:

- Tokens are encrypted with a key held outside the application database.
- Recommended: cloud KMS (AWS KMS, GCP KMS, Vault Transit) for cloud-deployed Care; hardware HSM for in-clinic Care; PBKDF2-derived local key file (root-readable) for self-hosted-by-practitioner Care. PBKDF2 + a passphrase entered at service start is acceptable for solo deployments.
- The encrypted blob is what's stored in `care_patient_grants.grant_token_encrypted`; decryption happens in-memory only.
- Audit logs include "decrypt-on-demand" events when running under HSMs that meter; otherwise no audit of decryption operations is required.

The operator decides KMS / key-management posture based on their compliance requirements (HIPAA, GDPR, country-specific regulations). The OHD project doesn't dictate a specific KMS, only that *something* outside the app DB is used.

## Open design items

- **Operator-grant-request flow** (Model 3 in "How grants get into the vault") — when the operator wants to invite a patient to grant, before the patient has issued anything. Spec deferred to `future-implementations/operator-grant-request.md`.
- **Operator-to-operator handoff**. When a patient is referred from one provider to another, both temporarily hold grants. Wire-level support for "operator A introduces operator B with the patient's blessing" needs UX + grant-derivation rules. Currently mentioned as open in [`../components/care.md`](../components/care.md).
- **Cohort / population queries** for clinical-research deployments. Today every OHDC call is per-patient. A cohort grant model that lets one query span N patients (with patient-by-patient resolution and aggregation) would unlock real research workflows but needs a different scope model. Open per [`../components/care.md`](../components/care.md).
- **Care MCP auth integration**. Care's MCP server uses operator session tokens; per-tool scope ("this tool requires `submit_lab_result` capability") needs explicit FastMCP per-tool auth wiring. Spec stub; details when MCP work surfaces.
- **Clinic-side operator role catalog**. The `role` field above is intentionally simple (`clinician` / `nurse` / `admin` / `auditor`); real clinics have more granular roles (residents, attending, charge nurse, social worker, etc.). Operator-defined custom roles with explicit capability sets are deferred to deployment-time customization in v1.
- **Operator-side anomaly detection**. Patterns like "unusual volume of queries at 3 AM," "sudden access from a new IP," "queries against patients the clinician hasn't seen recently" — all detectable from `care_operator_audit`. Belongs in a downstream tooling layer; the data is already captured.

## Cross-references

- Patient-side self-session and OIDC mechanics: [`auth.md`](auth.md)
- Three-auth-profile overview (self / grant / device) and grant scope rules: [`privacy-access.md`](privacy-access.md)
- Storage-format schema (`grants`, `event_case_links`, `audit_log`): [`storage-format.md`](storage-format.md)
- OHDC operations callable from Care: [`../components/connect.md`](../components/connect.md)
- Care's clinical workflow (visit prep, write-with-approval, MCP tools): [`../components/care.md`](../components/care.md)
- Relay path for talking to phone / NAS-based patient storage: [`../components/relay.md`](../components/relay.md)
- Emergency / break-glass flow (case-bound grants from authority cert chain): [`../components/emergency.md`](../components/emergency.md)
- Project-level identity-vs-storage commitment: [`../02-principles.md`](../02-principles.md)
