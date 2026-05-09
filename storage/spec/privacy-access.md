# Design: Privacy & Access Control

> Identity, permissions, audit trails — the substrate that makes OHD actually safe.

This document is the conceptual privacy/access spec. The bit-level schema (rule tables, `pending_events`, audit columns, etc.) lives in [`storage-format.md`](storage-format.md).

## Principles recap

From [`../02-principles.md`](../02-principles.md):

- No central identity in the core protocol.
- Access is explicit, scoped, time-limited, and audited.
- Users can revoke access instantly.

OHD's privacy contract is what makes the data ownership story real. If anything in this document conflicts with a deployment's behaviour, the deployment is wrong.

## Identity

### What the protocol stores

OHD Storage knows:

- A user's opaque ULID (`_meta.user_ulid`).
- Which OIDC provider authenticates them (`google`, `keycloak.example.org`, a hospital SSO, etc.).
- Their subject identifier from that provider (an opaque string the provider issued).

OHD Storage does **not** know:

- Names, emails, addresses, phone numbers.
- Any PII beyond the OIDC subject identifier.

### What a deployment may additionally store outside the protocol

A hospital deployment will have a table mapping OHD user ULIDs to their patient records. A SaaS deployment may have a billing table mapping user ULIDs to payment methods. These are **outside** the core protocol and live in deployment-specific code.

The separation matters: a leak of the OHD storage protocol layer reveals health events attached to opaque ULIDs; a leak of the deployment's identity layer reveals "who is user 12847392." The protocol's design ensures that the high-value leak (events + identity together) requires breaching both layers.

### Pseudonymous operation

Users who want maximum privacy can:

- Use a privacy-focused OIDC provider (self-hosted Authentik with a fake name, an alias-aware provider).
- Never share their real identity with any consumer.
- Run their own OHD Storage instance (on-device or self-hosted), where they're also the operator.

This gives them full functionality (logging via OHD Connect, sharing via grants, in-person sharing via OHD Relay pairing, exports) with zero PII exposure to OHD.

## The three auth profiles

Every external operation authenticates under one of three profiles. They share the same OHDC protocol surface; what they can do is bounded by their profile.

The full self-session mechanics — OAuth 2.0 Authorization Code with PKCE, OIDC delegation, token wire formats (`ohds_…`, `ohdr_…`), system-DB tables (`oidc_identities`, `sessions`, `pending_invites`), operator-configurable account-join modes, on-device Mode A/B, multi-identity linking — live in [`auth.md`](auth.md). What follows here is the conceptual three-profile summary.

### 1. Self-session

The user authenticated as themselves via OIDC. Full scope on their own data — read everything, write everything, manage grants, view the full audit log, export, import.

- **Token shape**: opaque `ohds_<base64url>` access (1h TTL) + `ohdr_<base64url>` refresh (30d, rotated on use). Server-side state in `sessions` (system DB); SHA-256-hashed at rest. Revocation is immediate (set `revoked_at_ms`).
- **Issued via**: OAuth Authorization Code + PKCE for browser/MCP clients, Device Authorization Grant for CLI. OIDC providers verify identity; OHD Storage acts as the OAuth Authorization Server. See [`auth.md`](auth.md).
- **Used by**: OHD Connect personal app (mobile, web, CLI), OHD Connect MCP, any user-facing tool the user authenticates to.

### 2. Grant token

Issued by the user (via `create_grant`) to a third party — a doctor, a researcher, a family member, a delegate. Scope is bounded by the grant's structured rules in `grants` and the rule tables (`grant_event_type_rules`, `grant_channel_rules`, `grant_sensitivity_rules`, `grant_time_windows`, `grant_write_event_type_rules`, `grant_auto_approve_event_types`).

- **Read scope**: which event types / channels / sensitivity classes / time windows.
- **Write scope**: which event types the grantee can submit (default: empty — read-only grant).
- **Approval policy**: `approval_mode` ∈ {`always`, `auto_for_event_types`, `never_required`}; per-event-type auto-approval list when `auto_for_event_types`.
- **Lifecycle**: `expires_at_ms`, `revoked_at_ms`, optional rate limits (`max_queries_per_day` / `_per_hour`), `notify_on_access`, `aggregation_only`, `strip_notes`, `require_approval_per_query`.

- **Token shape**: opaque random or signed envelope; resolves to a `grants` row by `ulid_random`. The exact format is an API-layer choice; the storage format requires only that the token resolves uniquely to a grant.
- **Used by**: OHD Care, researcher portals, family / delegate access, anything else with per-recipient grants.

### 3. Device token

A specialized grant: write-only, no expiry, attributed by `device_id`. Issued during one-time pairing (QR code, OAuth-style consent screen, push-to-confirm, NFC tap). Modelled in the schema as a row in `grants` with `kind='device'` and write-only scope.

- **Token shape**: long-lived, write-only, revocable per device.
- **Stored**: in platform secure storage on the device side (Keychain, Keystore, encrypted secrets manager).
- **Used by**: sensor / CGM integrations (Libre, Dexcom), lab providers, pharmacy systems, hospital EHRs pushing data, the OHD Connect mobile app's Health Connect / HealthKit bridge service component, any other write-only integration.
- **Damage radius**: forging events under the device's attribution. Cannot read history, cannot list grants, cannot revoke.

## Grants

A grant is the universal access primitive. Read-only grants, read+write grants, write-only grants (devices), emergency grants, delegate grants — all are rows in `grants` with different scope and policy fields.

### Creating a grant

```http
POST /ohdc/v0/grants
Authorization: Bearer <user-self-session-token>

{
  "grantee_label": "Dr. Smith — primary care",
  "grantee_kind": "human",
  "purpose": "Quarterly review",
  "default_action": "deny",
  "rolling_window_days": 365,
  "read_rules": {
    "event_types_allow": ["glucose", "heart_rate", "blood_pressure_systolic", "blood_pressure_diastolic", "medication_dose"],
    "sensitivity_deny": ["mental_health", "substance_use", "sexual_health", "reproductive"]
  },
  "write_rules": {
    "event_types_allow": ["lab_result", "clinical_note", "medication_prescribed"],
    "approval_mode": "always"
  },
  "policy": {
    "expires_at_ms": 1735689600000,
    "max_queries_per_day": 200,
    "notify_on_access": true
  }
}
```

Response includes the grant ID, the bearer token, and a shareable URL / QR for handing to the grantee.

### Scope dimensions

**Read** — what the grantee can query:

- **Event types**: allowlist or denylist.
- **Channels**: more granular than types (e.g. allow `meal` but deny `meal.nutrition.notes`).
- **Sensitivity classes**: deny entire categories regardless of type/channel rules.
- **Time windows**: absolute (`from`/`to`) or rolling (last N days).
- **Aggregation only**: grantee gets only aggregates, never raw events.
- **Strip notes**: the `notes` column is replaced with NULL.

**Write** — what the grantee can submit:

- **Event types**: allowlist for what the grantee can submit (default: empty = read-only grant).
- **Approval mode**: `always` (every submission queued for review), `auto_for_event_types` (pre-authorized types auto-commit), `never_required` (all writes auto-commit). Trust-tiered.
- **Auto-approve list**: which event types skip the queue under `auto_for_event_types`.

**Policy** — operational bounds:

- **Expiration**: hard deadline.
- **Rate limits**: per day, per hour.
- **Approval per query**: every query triggers a push to the user, who approves or denies. Extreme-privacy mode.
- **Notifications**: user is notified on each access (push, email, in-app).
- **Revocation**: always immediate, synchronous RPC.

### Write-with-approval

Grants with write scope can route submissions through an approval queue. Submitted events go to `pending_events`; the user reviews via OHD Connect; on approval the event commits to canonical storage with the same ULID. See [`storage-format.md` "Write-with-approval"](storage-format.md#write-with-approval) for the bit-level flow.

The patient sees what the doctor wants to add before it lands in their record. The doctor sees their submissions' status (pending / approved / rejected) but never the user's review reasoning. Audit log preserves both submission and review for full traceability.

Trust-tiered policy lets a primary doctor relationship auto-commit routine writes (`lab_result`, `clinical_note`) while still queueing high-stakes ones (`prescription`). New / one-off relationships default to `always`.

### Emergency access (break-glass)

Emergency access is a real, productized feature — not an aspirational reservation. It lets first responders access a curated subset of the user's data without a pre-issued grant, mediated by **certified emergency authorities**, with the user retaining audit visibility and revocation rights.

#### The actors

- **Patient phone** running OHD Connect. Broadcasts a low-power BLE beacon while the feature is enabled. Holds the patient's emergency-template grant (the user's emergency profile).
- **Bystander** — anyone with OHD Connect installed and internet connectivity, *including* the responder if they're nearby. Their device acts as a BLE-to-internet transport proxy. Forwards TLS-encrypted bytes; sees nothing of the patient's data.
- **Responder** (paramedic, EMS staff, ER triage, etc.). Authenticated to their **station's certified relay**. Views the data via their station-side authentication. Mechanically a bystander + a station-authenticated user.
- **Station's certified relay** — an OHD Relay deployment running in emergency-authority mode, holding an authority cert that the patient's phone trusts. Signs emergency-access requests. Routes responder traffic. The trust boundary is institutional (the station), not individual (the responder).

The trust model: the patient's phone has a list of **trusted authority roots**. An emergency request must be signed by an authority cert whose chain terminates at a trusted root. Responder identity is the station's operational concern; the patient's phone trusts the station, not each individual responder.

For v1.0, the OHD project maintains a default trust root and signs sub-certs for partner emergency services (per-country EMS organizations, hospital networks, etc.). Patients can add or remove trust roots. Per-country governance comes when there's a partnership story.

#### The emergency-template grant

The user pre-configures their emergency profile as a **template grant** (`grants.is_template=1`, `grantee_kind='emergency_template'`). Its rules tables define what's in scope when break-glass fires:

- **Default read scope** (allowlist): allergies, active medications, blood type, advance directives, recent vitals (HR, BP, SpO2, temperature, glucose if relevant), current diagnoses.
- **Default deny by sensitivity**: `mental_health`, `substance_use`, `sexual_health`, `reproductive` — unless the user explicitly opts to include them. Some emergencies need reproductive context (pregnancy, body-anatomy concerns); the user can flip this per category.
- **Per-channel granularity**: the user can fine-tune which specific channels are visible (e.g., share `glucose` but not `sleep`).

The user edits this template via OHD Connect's settings; under the hood, edits are CRUD on the template grant's rule tables.

#### The break-glass flow

1. **Discovery**: patient's phone broadcasts an opaque BLE beacon. Responder (or bystander) discovers it.
2. **Request**: responder, via their station's relay, sends a signed emergency-access request. Transport may be direct internet (if patient phone has connectivity), or BLE → bystander → internet → station relay.
3. **Verification**: patient's phone verifies the signature against trusted authority roots. If valid, accepts the request.
4. **Dialog**: phone shows the dialog above the lock screen — OHD logo, the authority's certified label ("EMS Prague Region"), countdown timer (default 30s, configurable 10–300s), [Approve] [Reject] buttons. Vibrates / rings.
5. **Resolution**:
   - **Approve** → flow continues immediately.
   - **Reject** → request denied; logged.
   - **Timeout** → action depends on `_meta.emergency_default_allow_on_timeout` (default: allow, for unconscious users; user can flip to deny).
6. **On approve (interactive or auto-granted)**:
   - Storage clones the emergency-template grant into a fresh active grant. The new grant's `grantee_kind='emergency'`, `is_template=0`, `grantee_label` = the authority's certified name, `grantee_ulid` = the authority's identity.
   - A new `cases` row is opened with `case_type='emergency'`, the new grant attached as `opening_authority_grant_id`.
   - The grant is bound to the new case via a `grant_cases` row.
   - Historical context window is applied per `_meta.emergency_history_hours` (default 24h).
   - An `audit_log` row records the access. If the timeout path was taken, `auto_granted=1` so the user's audit view renders it distinctly.
   - The grant token is issued back through the relay to the responder.
7. **Subsequent OHDC**: responder queries through their station relay using the issued grant token; reads return data per the rules; writes from the responder are tagged with the case_id (e.g., EKG measurements from the ambulance device flow into the case).

#### Settings (in OHD Connect)

- **Feature on/off** (default: off — opt-in).
- **BLE beacon on/off** (default: on when feature on; broadcasts opaque ID only, no health data).
- **Approval timeout** (default: 30s; configurable 10–300s).
- **Default action on timeout** (default: allow; configurable to deny). UI shows tradeoff copy: *allow = better for unconscious users; deny = better against malicious actors who might trigger break-glass when you're nearby and unaware*.
- **Lock-screen visibility**: full dialog above lock screen (default), OR "basic info only on lock screen" sub-option (shoulder-surfer mode).
- **Location share** (default: off; opt-in).
- **History window**: 0h / 3h / 12h / 24h of recent vitals visible to the responder (default: 24h).
- **Per-channel emergency profile**: which channels are in scope. Default profile is the allergies/meds/vitals set above; user can add or remove channels.
- **Sensitivity classes**: which sensitivity classes are allowed in emergency (default: all general; deny mental_health/substance_use/sexual_health/reproductive). User can toggle per class.
- **Trusted authority roots**: list of emergency authorities whose certs the phone accepts. Default: OHD project root + any country-specific roots pre-installed for the user's locale. User can add (paste cert) or remove.

#### Bystander as transport proxy

Any OHD Connect installation can serve as a transport proxy automatically — no opt-in by the bystander, no payload exposure to the bystander. The bystander's device:

- Listens for BLE-encapsulated emergency requests addressed to OHD beacons in proximity.
- Relays the encrypted bytes to the destination station relay over the bystander's internet connection.
- Returns the response bytes back over BLE.
- Sees nothing of the patient's data (TLS terminates at patient phone and station relay).

Battery and bandwidth cost is negligible (sporadic emergency events, short-lived sessions). The bystander can opt out of the proxy role in their own settings if they want; default is on, treating "OHD Connect installed" as implicit consent to act as a good-Samaritan transport.

#### Operator-side records

When an emergency authority records data into the patient's OHD during a case (vitals, drugs administered, observations, clinical notes), the authority typically also keeps a copy in their own infrastructure for clinical safety, regulatory retention (HIPAA / GDPR / national equivalents), billing, and operational continuity. This is consistent with how healthcare data flows already work — every doctor visit produces records the doctor's employer keeps independently of any patient-side record.

This duplication is **outside OHD's protocol scope**:

- OHD provides the patient-side canonical record, the audit trail of accesses, and lifecycle control (revocation stops *future* OHDC reads).
- OHD does **not** synchronize, manage, or police the operator-side copies. The operator's copies are governed by their own regulatory regime.
- Revocation of an OHD grant stops future OHDC access; it does not retroactively delete records the operator legitimately processed under a valid grant.

This is a feature, not a bug: clinical safety requires operators to have continuous access to the records they're working with, even if the patient's OHD goes offline. The data-ownership inversion still holds for *new* data flows; existing operator records are subject to existing healthcare data law.

The reference OHD Emergency app demonstrates a deployable pattern that includes operator-side record-keeping (alongside the OHDC integration) — see [`../components/emergency.md`](../components/emergency.md).

### Cases — episodes of care

Cases are labeled, curated containers of events. They serve different purposes than grants:

| | Case | Grant |
|---|---|---|
| Purpose | "What's in this episode" | "Who can read/write what" |
| Defines | Filter expressions over events | Access rules + optional case binding |
| Standalone? | Yes (patient organizes cases for self) | Yes (open-scope grants for ongoing access) |
| Composable? | Linked to other cases (predecessor / parent) | Bound to zero, one, or many cases |

A case has **filters** (which events fall in scope), **lifecycle** (start, end, auto-close), and **linkage** (parent / predecessor) — but no access rules. Grants own all access logic.

A grant can reference zero, one, or many cases via `grant_cases`. Open-scope grants see all the user's events (subject to grant rules). Case-bound grants see the union of their referenced cases' scopes (subject to grant rules).

**Linkage semantics:**
- **Predecessor → successor** (handoff chain, e.g. EMS → admission). Successor inherits forward — it reads the predecessor's scope automatically.
- **Children → parent** (sub-case rolls up, e.g. EKG referral under doctor's visit). Parent reads its children's scopes; the child does **not** see the parent's broader scope.

To get bidirectional inheritance (e.g., child also sees parent), the user explicitly links the same case as both `parent_case_id` and `predecessor_case_id`.

Schema and resolver semantics in [`storage-format.md`](storage-format.md) "Cases" and "Case scope resolution". User-facing flow in [`../components/emergency.md`](../components/emergency.md) and [`../components/care.md`](../components/care.md).

Important properties:
- Events themselves are case-agnostic at the schema level (no `events.case_id` column). Cases find their events via filters at read time. A single event can naturally participate in multiple cases without copying.
- Filter expressions can include time ranges, explicit event-ULID lists, device-id filters, event-type filters, and combinations (full set in [`ohdc-protocol.md`](ohdc-protocol.md) "Filter language").
- Auto-close after inactivity (default 12h for emergency cases; 30 days for admissions; NULL for user-curated cases — no auto-close). After close, new events are not retroactively added to the case for grant scope.
- Reopen tokens let an active authority reopen a recently auto-closed case within a TTL without re-running the break-glass flow.
- Patient can force-close any case at any time from OHD Connect.

### Grants don't chain

The user (self-session) is the only source of grants. Concretely:

- `OhdcService.CreateGrant` requires self-session token; calls under grant or device tokens return `WRONG_TOKEN_KIND`.
- A grantee cannot delegate, transfer, or sub-issue access from their grant.
- A grantee cannot pass the grant to anyone else; the token is bearer but the issuance path is gated to self-session.

This is a load-bearing simplification: the trust graph is one-hop (user → grantee), not transitive. A `delegate` grantee kind exists for "parent acts on behalf of child" / "caregiver acts on behalf of elderly parent" cases — that's an OAuth-level identity-assumption mechanism, not grant chaining.

### Operator-side records and OHD's scope boundary

Stated explicitly because it's a load-bearing scope decision:

**OHD's scope:**

- The patient's canonical record (events, channels, attachments, etc.).
- The OHDC protocol of access (who reads / writes, with what scope, audited, revocable).
- Lifecycle control over future access (revocation stops future OHDC reads).

**Outside OHD's scope:**

- Operator-side copies of data the operator legitimately processed under a valid grant. Hospitals' EHRs, EMS incident records, insurance claims systems, research data warehouses — all keep their own records under their own regulatory regimes.
- Synchronization or consistency between OHD and operator-side copies. Operators chose to copy; operators manage their copies.
- Retroactive deletion of operator-side copies via OHD revocation. Revoking a grant stops *future* reads, not the operator's historical records.

OHD provides the audit trail (the user sees what the operator accessed) and forward-looking control (the user can prevent future accesses). The operator's regulatory obligations (HIPAA in the US, GDPR in the EU, similar elsewhere) govern what they do with their copies — including retention, access by their staff, and patient-requested deletion under jurisdictional law. OHD doesn't try to be a global enforcer of healthcare data law; it provides the patient-controlled spine on top of which operators operate.

### Revocation

Always immediate, synchronous, never sync-deferred:

- **On-device storage**: revocation is local; takes effect on the next grant lookup (next request from the grantee).
- **Remote storage (cache mode)**: revocation is an RPC from cache to primary. Either succeeds (primary commits, replies OK) or fails (network down, primary unreachable). The user sees an error and retries when connectivity returns. No silent buffering.
- **Sync stream is not used** for revocations. Sync replays event creates / corrections / deletes; grant lifecycle changes are out-of-band RPCs. The semantic the user expects — "I just revoked, the doctor cannot read anymore" — only works if revocation is synchronous.

Once committed, the next sync pulls the updated grant row to other instances normally; but the *revocation effect* applies from commit, not from sync arrival.

## Audit

Every operation produces an audit row.

### Schema

The full schema is in [`storage-format.md`](storage-format.md). Conceptually:

- `ts_ms` — when the operation happened.
- `actor_type` — `'self'` / `'grant'` / `'system'`. (Device tokens are recorded as `'grant'` with `grants.kind='device'`.)
- `grant_id` — references the grant the operation came under (NULL for `self` and `system`).
- `action` — `read`, `write`, `delete`, `export`, `import`, `grant_create`, `grant_revoke`, `pending_approve`, `pending_reject`, `login`, `config`.
- `query_kind` and `query_params_json` — what was asked (canonicalized).
- `rows_returned` — what was returned to the caller.
- `rows_filtered` — how many rows matched but were silently stripped by grant rules. The grantee never sees this; the user always does.
- `result` — `success` / `partial` / `rejected` / `error`.

### What gets logged

- Every read operation: query parameters, rows returned, rows filtered, result.
- Every write operation: event IDs created or modified, approval state changes for pending events.
- Every grant lifecycle event: created, used, expired, revoked.
- Every rejected access: the rejection reason.
- Every export and import.

### What the user sees

The personal dashboard surfaces the audit log:

- List of active grants, who they're for, what they can see and write, when they expire.
- Recent access: "Dr. Smith queried glucose data on 2025-01-15 at 14:00, returned 47 events, 0 filtered."
- Filtered-row alerts: "Dr. Smith's last query had 3 rows filtered (channels not in their grant)."
- Pending submissions: events the doctor wants to add, awaiting the user's approval.
- Anomaly flags: access from unexpected IPs, unusually large queries, off-hours access, revocation attempts.

### Audit retention

- Per-user audit lives in the user's storage file. Retention is configurable via `_meta.audit_retention_days` (default forever).
- System-level audit (file created / deleted, key rotated, OIDC events, abuse signals) lives in the deployment's separate system DB so it survives user-file deletion. The boundary: *if a row only makes sense given the user's data, it's per-user; if it must survive when the user is forgotten, it's system-level.*
- GDPR right-to-be-forgotten triggers a documented sequence: anonymize per-user audit, delete the user file, system-level audit retains an anonymized stub for legal traceability.

## Encryption

### In transit

- TLS 1.3 required for all OHDC traffic (handled by the transport layer, not by storage).
- Caddy handles automatic HTTPS via Let's Encrypt for SaaS / custom-provider / self-hosted deployments.
- HTTP/3 over QUIC by default.
- Certificate pinning for OHD Connect mobile clients targeting our SaaS or known operators.

### At rest

- **Storage file**: SQLite + SQLCipher 4 with page-level AES-256. Per-user key derived from a user-held secret (passphrase, biometric-unlocked keystore item, hardware token) plus a per-file salt in `_meta.cipher_kdf`.
- **Sidecar blobs**: encrypted with the same per-user key using libsodium `crypto_secretstream` (or equivalent AEAD). Each blob is independently decryptable; metadata in `attachments.sha256` is the address; integrity checked on read.
- KDF currently PBKDF2-SHA512 with 256k iterations (SQLCipher 4 default). Migration to Argon2id is planned when SQLCipher 5 lands.

### End-to-end channel encryption

For maximum-privacy users: encrypt sensitive-class fields (mental_health, substance_use, sexual_health, reproductive) client-side with a key only the user holds. The storage operator stores ciphertext; grants include wrapped key material so the right grantee gets the right plaintext.

The format reserves room for this; the bit-level details (wrapping format, KDF parameters, ciphertext column shape) are not yet specified. Listed as an open design item.

## Threat model

### Who we protect against

| Threat | Mitigation |
|---|---|
| Passive eavesdropping (ISPs, public Wi-Fi) | TLS 1.3 / HTTP/3 everywhere |
| Lost backup / opportunistic device theft | SQLCipher at rest |
| Compromised OHDC integration (leaked Libre token) | Device tokens are write-only; no exfiltration possible; user revokes |
| Compromised OHD Care client (leaked grant token) | Grant scope bounds disclosure; user audits; user revokes synchronously |
| Compromised OHD Relay operator | Relay sees ciphertext only (TLS end-to-end between storage and client); cannot read |
| Malicious operator (SaaS or third-party) | Per-user encryption; audit visible to user; portable export means user can leave; planned end-to-end channel encryption for sensitive classes |
| Rogue insider at the OHD project | Audit logs, code review, open source, end-to-end encryption for the most sensitive |

### Who we do not fully protect against

- **A compromised user device** (stolen unlocked phone with active session). Mitigated by short session TTLs, biometric re-auth for sensitive operations, remote wipe.
- **Legal subpoena**: if a court orders data disclosure to a deployment, the operator complies. Users who want to resist this should pick on-device or self-hosted deployments and use end-to-end encryption when available.
- **The user themselves sharing their own data unwisely**: they can hand anyone a full export; we can't prevent that.

## Open design items

- **End-to-end channel encryption** — schema reserves space; key-wrapping format and grant-handoff details TBD.
- **Family / delegate grants** — `grants.kind='delegate'` is the marker; full or scoped authority semantics TBD.
- **Notification delivery** — push notifications for "access happened" need infrastructure (FCM, APNS, email). Deployment concern; falls outside the protocol but needs concrete recommendations.
- **Break-glass UX** — pre-issued emergency grants on a QR / wristband / national emergency registry. The OHDC protocol supports this; the operator-side identity verification of paramedics is a separate problem (per-country integration with emergency services).
- **Grant token format** — opaque vs JWT vs signed envelope. Implementation choice; not constrained by the storage format.
