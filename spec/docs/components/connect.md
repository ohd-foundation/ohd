# Component: OHD Connect (and the OHDC protocol)

> This file describes both **OHDC** — the single external protocol of OHD Storage — and **OHD Connect** — the canonical personal app that uses it.

## OHDC — the protocol

OHDC is the only external surface of OHD Storage. Every external consumer (the OHD Connect personal app, the OHD Care professional app, sensor integrations, MCP servers, CLIs) speaks OHDC.

What an authenticated session can actually invoke is determined by its **token scope**, not by the protocol layer. There is one wire format, one set of operations, three auth profiles.

### Wire format

OHDC is a **Connect-RPC** service defined by **Protobuf** schemas. Picked because:

- **Schema-first.** The `.proto` file is the contract. Vendors compile against it; drift is caught at compile time. Critical because OHDC is the single external surface — third parties (CGM providers, lab integrations, hospital EHRs) build against it without coordination.
- **Codegen across every language OHD ships in.** Buf CLI generates typed clients/servers in Rust (the storage core), Kotlin (Android), Swift (iOS), TypeScript (Care web + Connect web), and Python (tooling, conformance harness, MCP servers). One source of truth.
- **Wire-encoding flexibility.** Each request advertises `Content-Type` — `application/proto` (binary, default in production) or `application/json` (debuggable in browsers, curl-friendly). Same schema, two encodings.
- **Streaming first-class.** Server-streaming for `read_samples` over long ranges, sync (cache↔primary), and `audit_query` tail-follow. Client-streaming for large `import` and chunked `attach_blob`. Defined in the `.proto`, not bolted on with SSE/WebSocket workarounds.
- **HTTP-native.** Plain HTTP/3 with HTTP/2 fallback. Works through every proxy, load balancer, and CDN. Standard HTTP status codes (`429`, `404`, `500`); structured error details (`OUT_OF_SCOPE`, `INVALID_UNIT`, etc.) in the body.
- **gRPC-compatible.** A Connect server accepts gRPC clients and vice versa, so an integrator who already has a gRPC stack (large hospital EHRs, lab systems) reuses tooling.

### Transport

| Layer | Choice |
|---|---|
| Protocol | Connect-RPC over HTTP |
| HTTP version | HTTP/3 (QUIC) preferred; HTTP/2 fallback |
| Default encoding | `application/proto` (Protobuf binary) |
| Debug encoding | `application/json` (same schema, JSON wire) |
| TLS | TLS 1.3 required, terminated by Caddy on the operator side, terminated end-to-end through OHD Relay |
| Path prefix | `/ohdc.v0.OhdcService/<Method>` |

Mobile clients use platform-native HTTP/3 stacks (URLSession on iOS, Cronet on Android). The Rust core uses `hyper` + `quinn`.

### Release artifacts

For each OHDC version, the project publishes:

- The `.proto` schema files in the `ohd-protocol` repo (`proto/ohdc/v0/*.proto`).
- Generated client libraries: `ohdc-client-rust`, `ohdc-client-kotlin`, `ohdc-client-swift`, `ohdc-client-ts`, `ohdc-client-python`.
- A reference server stub used by the storage core and by integrators to test against.
- The conformance corpus (input event sequence + expected query outputs + binary sample-block fixtures); see [`../design/storage-format.md`](../design/storage-format.md).
- Buf Schema Registry-published API documentation at `buf.build/openhealth-data/ohdc`.

### Operations

| Category | Operation | Notes |
|---|---|---|
| Write | `put_events` | Append a batch of events atomically. Returns wire ULIDs. |
| Write | `attach_blob` | Upload a sidecar attachment (raw ECG bytes, image, PDF). |
| Read | `query_events` | Iterate events with filters (event type, time range, channels, source, cursor). |
| Read | `get_event_by_ulid` | Look up one event. Out-of-scope events return "not found." |
| Read | `aggregate` | Numeric aggregates (avg / sum / min / max / quantiles) over a channel, bucketed by time. |
| Read | `correlate` | Temporal correlation between two channels. |
| Read | `read_samples` | Decoded sample stream from a dense series event. |
| Read | `read_attachment` | Download a sidecar blob. |
| Grant lifecycle | `create_grant`, `list_grants`, `revoke_grant`, `update_grant` | Grants. |
| Audit | `audit_query` | Inspect the audit log. |
| Pending | `list_pending`, `approve_pending`, `reject_pending` | Approval queue for grant-submitted writes. |
| Export / Import | `export`, `import` | Portable data movement. |
| Diagnostics | `whoami`, `health` | Identity and health-check. |

### Auth profiles

Three token kinds, all flowing through the same OHDC API. Storage validates the token kind and scope on every call.

#### Self-session token

Issued to the user authenticated as themselves via OIDC. Full scope on their own data — read, write, manage grants, view audit, export.

Used by: the OHD Connect personal app, the OHDC CLI in personal mode, the OHD Connect MCP server.

#### Grant token

Issued by the user (via `create_grant`) to a third party — a doctor, a researcher, a family member, a clinical app, a delegate. Bounded by the grant's structured rules:

- **Read scope**: which event types, channels, sensitivity classes, time windows are visible.
- **Write scope**: which event types the grantee can submit (default: empty).
- **Approval policy**: whether writes go to a pending queue (`require_user_approval_for_writes`), or auto-commit, or per-event-type auto-approval.
- **Standard policy**: expiry, rate limits, notify-on-access, aggregation-only, strip-notes.

Used by: OHD Care, researcher portals, family/delegate access, anything else with a per-recipient grant.

#### Device token

A specialized grant: write-only, no expiry, attributed to a `devices` row. Issued during one-time pairing (QR code, OAuth-style consent, push-to-confirm, NFC tap). Used by:

- The OHD Connect mobile app's Health Connect / HealthKit bridge service component
- Sensor / CGM integrations (Libre, Dexcom, Garmin, etc.)
- Lab providers / pharmacy systems / hospital EHRs pushing data
- Other health apps that integrate with OHD as their canonical store

The damage radius is intentionally bounded: a leaked device token can forge events under that device's identity but cannot exfiltrate history. This is what makes "Libre's backend service as an OHD writer" feasible — low-blast-radius auth for low-blast-radius operations.

### Idempotency

Every write event submitted carries a `(source, source_id)` pair. Storage enforces uniqueness on this pair. Retries from a flaky network, redelivery from a Health Connect change-token replay, batch reprocessing — none produce duplicates.

### Validation

Every event is validated against the channel registry on insert:

- The `event_type` must be registered (or resolve through `type_aliases`).
- Each channel value must reference a registered channel for that type (or alias).
- Value types must match the channel's declared `value_type` (real / int / bool / text / enum / group).
- Enum values must be in the channel's `enum_values` list.
- Required channels must be present.

Failures reject the event at the boundary; nothing partial is written.

### Write-with-approval

When a grant has `require_user_approval_for_writes = true` and the grantee submits an event:

1. Storage allocates a ULID and writes the event into `pending_events` (not into `events`).
2. The user's personal app gets a notification.
3. User reviews in OHD Connect: "Dr. Smith would like to add: lab result, glucose 5.4 mmol/L, 2026-05-07 09:00. [Approve] [Reject]".
4. On approve: event committed to `events` with the same ULID; pending row's status flips to `approved` and links to the canonical `events` row. Audit log records both submission and approval.
5. On reject: pending row stays with status `rejected` + optional reason; audit log records both.
6. Auto-expire after a configurable window (default 7 days) → status `expired`.

Trust-tiered policy on each grant tunes how aggressive the approval flow is:

- `approval_mode='always'` — every write requires user review (default for new clinical relationships).
- `approval_mode='auto_for_event_types'` — pre-authorized event types auto-commit; others queue. Used for established relationships ("Dr. Smith can auto-write `lab_result` and `clinical_note`; everything else queues").
- `approval_mode='never_required'` — all writes auto-commit (used for trusted long-term grants and emergency / break-glass cases where queueing would be malpractice).

The grantee always sees the pending status of their submission and can poll for it. The grantee never sees the user's review; the user always sees what was submitted whether or not they review it.

### Validation, scope intersection, and audit

Every read and every write runs through the resolution algorithm in `../design/storage-format.md` "Privacy and access control":

1. Resolve the token to its kind and scope (self / grant / device).
2. For grant tokens: intersect the requested operation with the grant's rules. For self-session: skip (full scope). For device tokens: write-only check.
3. Execute the operation. For reads, strip channels per grant rules; for writes, route through the approval queue if required.
4. Append an `audit_log` row with `actor_type`, `grant_id`, `query_kind`, `rows_returned`, `rows_filtered`, `result`.

The user can inspect their own audit log to see exactly what every grant has done — what they queried, what was filtered out (silently to the grantee), what they wrote, what's pending review.

## OHD Connect — the personal app

Reference personal app: Android, iOS, web. Configured with a self-session token; speaks OHDC against the user's storage (local in-process if on-device deployment, HTTP/3 if remote).

### Functions

- **Logging** (writes via OHDC):
  - Health Connect / HealthKit bridge — automatic background sync of authorized health data.
  - Manual entry — barcode food (with OpenFoodFacts resolution producing parent meal + child food_items), medications (user's "things at home" list with one-tap dose logging), custom measurements with user-defined types.
  - Voice / free-text input parsed to structured events.
  - Symptoms, mood, sleep with quality.
- **Personal dashboard** (reads via OHDC):
  - Recent activity, latest measurements per channel, adherence summaries.
  - Timeline view, chart builder, saved views.
  - Cross-channel correlation (the LLM-driven case lives in the MCP).
- **Grant management**:
  - Create grants from templates (typical patterns: "primary doctor", "specialist for one visit", "spouse / family", "researcher with study", "emergency break-glass").
  - View active grants, what they can see, what they've queried.
  - Revoke any grant immediately (synchronous RPC against the primary; not part of sync).
- **Pending approvals**:
  - Notification when a grant submits a write that requires review.
  - Side-by-side preview: "current value vs. submitted value" for measurements; full structured preview for clinical content.
  - Approve / reject / approve-and-add-this-type-to-auto-approval.
- **Audit inspection**:
  - Per-grant audit view: what each grantee has queried, when, with what filters, and how many rows were silently filtered out.
- **Export / portability**:
  - Full portable export (lossless OHD format, signed by the source instance).
  - Doctor-PDF for in-person sharing.
  - Migration assistant for moving between deployment modes (on-device → SaaS, etc.).
- **Cases**:
  - List of ongoing cases prominently surfaced (so the user is aware: "EMS Prague Region — open since 14:23").
  - Closed-cases history with full audit (who accessed what, when, what was written).
  - Tap into any case to see its events, audit, handoff chain, current authority, retrospective grant management.
  - Force-close any case at any time (revokes the active authority's grant).
  - Issue retrospective case-scoped grants (specialist consult, insurer billing review).
- **Emergency / break-glass settings** (full UX detail in `screens-emergency.md`):
  - Feature on/off (default off — opt-in).
  - Approval timeout (default 30s; range 10–300s).
  - Default action on timeout — Allow vs. Deny — with explanatory copy: *Allow gives access if you can't respond; better for unconscious users. Deny refuses access on timeout; better against malicious requests when you're nearby and unaware.*
  - BLE beacon on/off (default on when feature enabled; broadcasts opaque ID only).
  - Lock-screen behaviour: full dialog above lock (default), or "basic info only on lock" (shoulder-surfer mode).
  - Location share opt-in.
  - History window (0h / 3h / 12h / 24h) — how much vital-signs context the responder gets.
  - Per-channel emergency profile — toggle which channels are visible in emergencies.
  - Sensitivity-class toggles — by default hides mental_health / substance_use / sexual_health / reproductive; user can flip per category.
  - Trusted authority roots — list of emergency authority CAs the phone accepts; OHD project default + per-locale roots; user can add or remove.
- **Pending review (incoming writes)**:
  - When a grant submits a write that requires approval, the user reviews here. Includes emergency-case writes if the user has flagged them for review.
- **Audit transparency for emergency**:
  - Auto-granted (timeout-default-allow) accesses are visually distinct in the audit view (different color / icon).
  - User can review what was accessed during a break-glass after the fact, and dispute / refine settings.

### Form factors

- **Android** — Kotlin / Compose, links the OHD Storage Rust core via uniffi for local-primary deployments. HTTP/3 client for remote-primary.
- **iOS** — Swift / SwiftUI, same Rust core via uniffi.
- **Web** — for browser access against a remote storage. (Local-primary on web is not supported — browsers don't host services well.)

### OHD Connect CLI (`ohd-connect`)

Terminal interface to OHDC with a self-session token. For power users, scripts, and automation.

```
$ ohd-connect log glucose 145 --unit=mg/dL --at="2025-01-15T14:32:00"
$ ohd-connect log food "banana" --quantity=120g --started="14:00" --ended="14:05"
$ ohd-connect query glucose --last-week
$ ohd-connect grant create --name="Dr. Smith" --read=glucose,heart_rate --expires=30d
$ ohd-connect audit list --grant <id>
$ ohd-connect pending list
$ ohd-connect export --format=ohd > backup.ohd
```

Configured with the storage URL and a self-session token (acquired via OIDC device flow on first run). Distributed as a pip package, a Homebrew formula, and a curl-to-bash installer.

### OHD Connect MCP (`ohd-connect-mcp`)

Personal-LLM context. Exposes OHDC operations as MCP tools to a chat assistant, configured with a self-session token.

Tools:

- **Logging**: `log_symptom`, `log_food`, `log_medication`, `log_measurement`, `log_exercise`, `log_mood`, `log_sleep`, `log_free_event`
- **Reading**: `query_events`, `query_latest`, `summarize`, `correlate`, `find_patterns`, `get_medications_taken`, `get_food_log`, `chart`
- **Grants**: `create_grant`, `list_grants`, `revoke_grant`
- **Pending review**: `list_pending`, `approve_pending`, `reject_pending`
- **Cases**: `list_cases`, `get_case`, `force_close_case`, `issue_retrospective_grant`
- **Audit**: `audit_query`, including the `auto_granted` flag on emergency-timeout accesses

The user's chat assistant uses this MCP to log new entries ("I just took my metformin"), answer questions about their data ("how was my sleep last week?"), manage grants ("revoke Dr. Smith's access"), and review pending submissions.

## Third-party integrations (OHDC with device tokens)

Any external service can become an OHDC consumer with a device-token grant. The reference protocol stays OHDC; only the auth profile differs.

Examples:

- **CGM / sensor providers** (Libre, Dexcom, Garmin) — provider-side service holds per-user device tokens, pushes samples on schedule.
- **Lab providers** — push lab results as `lab_result` events when results are released.
- **Pharmacy systems** — push `medication_prescribed` events when prescriptions are filled.
- **Hospital EHRs** — mirror discharge notes, procedures, vaccinations into the patient's OHD on discharge.
- **Other health apps** (MyFitnessPal, Strava, Gentler Streak) — bridge their data via per-user device tokens.

The project provides:

- The OHDC protocol spec (this document + `../design/storage-format.md`).
- Reference client libraries: Kotlin, Swift, Python, TypeScript.
- A pairing / consent UX integrators embed.

The project does not provide the integrations themselves. Each is run by the third party against the protocol.

## What an OHDC consumer must do correctly

1. **Authenticate** with the appropriate token kind.
2. **Translate to OHD events** — never push vendor-specific payloads raw; map to registered types and channels (custom under `com.<vendor>.*` namespaces if needed).
3. **Set source attribution** — `metadata.source` (logical: `health_connect:com.x.y`, `manual:android_app`, `libre:cgm-bridge`) and `source_id` (idempotency key from upstream).
4. **Preserve event time** — `timestamp_ms` is when the measurement happened, not when the consumer synced it.
5. **Handle offline gracefully** — queue locally; sync when available; never silently drop.
6. **Respect rate limits** — back off on `429`.
7. **Report state** — surface "last sync" / "queue depth" / "errors" / "pending review" to the user. Failure should be visible.

## Security

- **Token storage**: Android EncryptedSharedPreferences / Keystore; iOS Keychain; CLI/services 0600 config or secrets manager. Never in shell history, never in logs.
- **OIDC device flow** for first-time self-session pairing. No password ever touches an OHDC consumer.
- **Token-kind enforcement**: storage validates token kind on every call. Device tokens cannot read; grant tokens cannot write outside their scope; self-session tokens cannot be issued except via OIDC.
- **Certificate pinning** for mobile clients targeting our SaaS or known operators. Falls back to PKI for self-hosted operators with arbitrary domains.
- **Source signing** (optional, for high-trust integrations): Libre, lab providers may sign their submissions with a per-integration key; storage records the signature; user sees signed-by indication. Protects against leaked-token attackers forging integration writes.

## Open design items

- **Family / delegate access** — a grant kind where one user acts on behalf of another (parent for child, caregiver for elderly parent). Modeled as `grants.kind='delegate'`; full or scoped authority TBD.
- **Source signing scheme** — optional integration-level signing (key management, signature format, surface in UI).
- **Approval-policy templates** — common bundles ("primary doctor", "specialist for one visit", "research participant", "emergency contact") shipped as defaults to lower the bar for grant creation.
