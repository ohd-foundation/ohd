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

### 1. Self-session

The user authenticated as themselves via OIDC. Full scope on their own data — read everything, write everything, manage grants, view the full audit log, export, import.

- **Token shape**: short-TTL bearer (default 1 hour), with a refresh token for longer sessions. Bound server-side so revocation is immediate.
- **Stored**: server-side in a session store (Redis, or the storage's own session table). Revoked on logout, on password/key change, or by the user from another device.
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
POST /ohdc/v1/grants
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

### Emergency / break-glass grants

A pre-issued grant for emergency responders. Curated for the critical subset:

- **Allowed reads**: active medications, allergies, known diagnoses, blood type, advance directives.
- **Denied sensitivity classes**: mental_health, substance_use, sexual_health, reproductive (unless the user explicitly opts to include them — body-anatomy emergencies can need reproductive context).
- **Approval mode for writes**: `never_required` (queueing emergency writes would be malpractice).
- **Notifications**: always on. The user sees emergency access promptly.

Activation:

- **Pre-emptive**: the user generates a long-lived token, stores it as a QR on their phone's lock screen, on a wristband, in emergency-services' registry. Paramedics scan it.
- **On-demand**: the user is conscious and pairs with the responder's OHD Care via NFC.

No permission system is perfect for emergencies. The goal is to be better than the status quo (paramedics asking the unconscious patient) without locking unconscious patients out of care.

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
