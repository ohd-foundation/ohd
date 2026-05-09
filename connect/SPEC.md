# OHD Connect — Implementation Spec

> Implementation-ready spec for OHD Connect, the personal-side reference
> application of OHD. Distilled from
> [`../spec/docs/components/connect.md`](../spec/docs/components/connect.md)
> and pinned snapshots in [`spec/`](spec/). This document is the contract
> between the design phase and the implementation phase; if it conflicts with
> the canonical spec, the canonical spec wins and this file is wrong.

## Scope

Connect is the **only** OHDC consumer that runs under self-session auth. It is
**not** an OHDC server, never holds grant tokens issued to it (it issues them
to others), and never holds device tokens (it pairs them out to sensors).

In scope:

1. Logging — Health Connect / HealthKit bridge, manual entry (measurements,
   medications, meals, symptoms, mood, sleep, exercise), barcode food via
   OpenFoodFacts, voice / free-text.
2. Personal dashboard — recent activity, charts, timelines, saved views,
   simple cross-channel correlation.
3. Grant management — create, list, revoke, inspect grants; per-grant audit.
4. Pending review — review and approve/reject grant-submitted writes.
5. Audit inspection — full audit, with auto-granted-emergency entries
   visually distinct.
6. Cases — list active and closed cases; force-close; retrospective grants.
7. Emergency settings — break-glass feature toggle, BLE beacon, approval
   timeout + default action, history window, sensitivity-class toggles,
   trusted authority roots, bystander-proxy role; **patient-side only** (the
   responder UX is owned by the OHD Emergency component).
8. Export / portability — full lossless OHD export, doctor-PDF, migration
   between deployment modes.
9. Notification handling — wake on push from FCM/APNs/Web Push, fetch
   actual content via OHDC under self-session, render OS notifications.
10. Connect MCP server — LLM-driven personal use of the same OHDC surface.
11. CLI — terminal interface to the same OHDC surface.

Out of scope:

- The OHDC server (lives in OHD Storage).
- The Care app, the Emergency app, the Relay, sensor integrations.
- The Care MCP and Emergency MCP (separate components).
- Issuance of grant or device tokens at the protocol level — Connect calls
  `Grants.CreateGrant` / `Grants.IssueDeviceToken`; the actual minting is
  storage-side.
- Cert-chain validation for emergency authority — that's
  `../spec/docs/design/emergency-trust.md`; Connect just calls the verifier.

## Form factors

| Form factor | Status | OHDC primary deployment | OHDC remote deployment |
|---|---|---|---|
| **Android** | Scaffold; Compose stub | On-device via Rust core (uniffi → Kotlin) | HTTP/3 via Cronet |
| **iOS** | Scaffold; SwiftUI stub | On-device via Rust core (uniffi → Swift) | HTTP/3 via URLSession |
| **Web** | Scaffold; Vite + React | n/a (browsers can't host) | HTTP/3 via Connect-Web fetch |
| **CLI** | Scaffold; Rust + clap | Optional: link Rust core directly | HTTP/3 via tonic + hyper |
| **MCP** | Scaffold; Node + TS | n/a (subprocess of CLI / mobile bridge for local) | HTTP/3 via Connect-Web fetch |

The same `OhdcClient` abstraction (per language, codegen'd by Buf from
[`../storage/proto/ohdc/v0/`](../storage/proto/ohdc/v0/)) powers every form
factor. The on-device deployment swaps the transport for direct in-process
calls into the Rust core; the wire surface is identical.

### Why Rust for the CLI

The legacy `ux-design.md` hints at Python ("pip package"). We override that
for v1: the CLI is **Rust** (`ohd-connect` binary) so it can:

1. Share the `ohdc-client-rust` codegen drop with the storage core directly,
   no shim crate.
2. Optionally link `libohdstorage` for a single-binary on-device REPL when the
   user's data lives in a `.ohd` file on the same machine.
3. Use the same release pipeline as the storage server binary.
4. Stay distributable as a static binary (Homebrew formula, curl-to-bash,
   GitHub Releases) without per-platform Python interpreter assumptions.

Documented in [`STATUS.md`](STATUS.md).

## OHDC client surface Connect needs

Connect calls a subset of the OHDC `OhdcService` defined in
[`../spec/docs/design/ohdc-protocol.md`](../spec/docs/design/ohdc-protocol.md).
Concretely, every form factor needs the methods below grouped by feature.

### Auth (self-session)

| Method | Purpose |
|---|---|
| OAuth `/authorize`, `/token`, `/oidc-callback` | Self-session token acquisition. Browser-based clients use auth code + PKCE; CLI uses device flow; MCP servers use auth code + PKCE. See [`spec/auth.md`](spec/auth.md). |
| `/oauth/register` (RFC 7591) | Dynamic client registration when the operator allows it. |
| `/.well-known/oauth-authorization-server` | RFC 8414 metadata discovery. |
| `Auth.WhoAmI` | Returns `user_ulid`, identity bindings, current session metadata. |
| `Auth.ListIdentities` / `Auth.LinkIdentity` / `Auth.UnlinkIdentity` | OIDC identity binding management. |
| `Auth.ListSessions` / `Auth.RevokeSession` | "Logged in on these devices" view. |
| `Auth.IssueInvite` (admin role only) | If the user is an `invite_admin` on the deployment. |

### Logging (writes)

| Method | Purpose |
|---|---|
| `Events.PutEvents` | Append a batch of events. Connect uses this for every manual log + Health Connect / HealthKit bridge sync. |
| `Events.AttachBlob` | Sidecar attachments (raw photos for symptom logs, ECG bytes from HealthKit). |
| `Registry.ResolveType` / `Registry.ResolveChannel` | Resolve aliases when a Health Connect record type maps to a registered OHD type. |

### Reading (dashboard)

| Method | Purpose |
|---|---|
| `Events.QueryEvents` | Iterate events with filters — backs the timeline view, chart builder, recent-events feed. |
| `Events.GetEventByUlid` | Detail view for one event. |
| `Events.Aggregate` | Avg / sum / min / max / quantiles bucketed by time — vitals dashboard, adherence summary. |
| `Events.Correlate` | Temporal correlation between two channels — meal/glucose-response view. |
| `Events.ReadSamples` | Decoded sample stream for dense series (HR during exercise, ECG playback). |
| `Events.ReadAttachment` | Download sidecar blobs. |

### Grant management

| Method | Purpose |
|---|---|
| `Grants.CreateGrant` | Issue a grant from a template ("primary doctor", "specialist for one visit", etc.). Returns the share artifact (`ohdg_…` token + rendezvous URL if remote). |
| `Grants.ListGrants` | Active + revoked grants. |
| `Grants.RevokeGrant` | Synchronous revocation; not sync-deferred. |
| `Grants.UpdateGrant` | Edit scope / approval policy / expiry. |
| `Grants.GetGrantUsage` | Per-grant: queries fired, rows returned, rows silently filtered, last access time. |

### Pending review

| Method | Purpose |
|---|---|
| `Pending.ListPending` | Inbox of grant-submitted writes awaiting user review. |
| `Pending.ApprovePending` | Approve; storage promotes pending row to `events` with same ULID. Optional: add event type to `auto_for_event_types` allowlist on the source grant. |
| `Pending.RejectPending` | Reject; pending row stays with status `rejected`. |
| `Pending.GetPending` | Detail view: full structured preview + side-by-side current-vs-submitted for measurements. |

### Audit

| Method | Purpose |
|---|---|
| `Audit.AuditQuery` | Per-grant or global audit view. `auto_granted=1` flag on emergency-timeout entries surfaces in the audit UI distinctly. |

### Cases

| Method | Purpose |
|---|---|
| `Cases.ListCases` | Active + closed cases; ordered active-first then recent-closed. |
| `Cases.GetCase` | Case detail: timeline, authorities, audit, handoff chain. |
| `Cases.ForceCloseCase` | User-initiated close; revokes the active authority's grant. |
| `Cases.IssueRetrospectiveGrant` | Issue a case-scoped grant after the fact (specialist consult, insurer billing review). |

### Emergency settings (the patient's emergency-template grant)

| Method | Purpose |
|---|---|
| `Grants.CreateGrant` with `kind='emergency_template'`, `is_template=1` | Initial creation of the emergency-template grant. |
| `Grants.UpdateGrant` on the emergency-template | Each settings change rewrites the template (history window, channels, sensitivity classes, etc.). |
| `Auth.AddTrustRoot` / `Auth.RemoveTrustRoot` / `Auth.ListTrustRoots` | Manage trusted authority CAs (project default + per-locale + user-added). |
| `Settings.SetEmergencyConfig` | Phone-side toggles that don't live on the grant: BLE-beacon on/off, lock-screen-basic-only mode, bystander-proxy role, location share opt-in, approval timeout, default-on-timeout. |

(Some of these are deployment-side and sit in the system DB; some are on the
per-user file inside the emergency-template grant. Storage owns the split.)

### Export / portability

| Method | Purpose |
|---|---|
| `Export.Export` (server-streaming) | Full portable export, signed by the source instance. |
| `Export.GenerateDoctorPdf` | Curated PDF for in-person sharing. |
| `Export.MigrateInit` / `Export.MigrateFinalize` | Migration assistant flow when moving between deployment modes. |

### Diagnostics

| Method | Purpose |
|---|---|
| `Diag.WhoAmI` | Identity / session status. |
| `Diag.Health` | Storage reachability + version. |

## OIDC self-session flow (per form factor)

Per [`spec/auth.md`](spec/auth.md). All flows produce the same `(ohds_…,
ohdr_…)` opaque token pair, server-tracked in the deployment system DB.

### Android / iOS / Web

OAuth 2.0 Authorization Code Flow + PKCE.

1. Client computes PKCE code verifier + challenge.
2. Client opens system browser:
   - Android: Custom Tabs.
   - iOS: ASWebAuthenticationSession.
   - Web: top-level navigation.
3. User picks provider; storage redirects to that provider's `/authorize`.
4. After provider login, redirect lands at storage's `/oidc-callback`.
5. Storage redirects back to the client's registered `redirect_uri` with a
   one-time OHD authorization code.
6. Client `POST /token` with the code + PKCE verifier → receives
   `(ohds_access_token, ohdr_refresh_token, expires_in)`.
7. Tokens stored:
   - Android: EncryptedSharedPreferences + Keystore-wrapped key.
   - iOS: Keychain (kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly).
   - Web: IndexedDB with origin isolation. Refresh token never exposed to
     JS contexts other than the auth module.

For the on-device deployment, the storage instance itself is the AS, addressable
on `127.0.0.1:<port>` for the loopback redirect. See `spec/auth.md` "On-device
sub-modes" for Mode A (OIDC-bound) vs Mode B (anonymous local entity).

### CLI (`ohd-connect`)

OAuth 2.0 Device Authorization Grant (RFC 8628):

```
$ ohd-connect login --storage https://ohd.cloud.example.com
Open https://ohd.cloud.example.com/device on any browser
Enter code:  BCDF-XYZW
Waiting for confirmation… (expires in 10 minutes)
✓ Logged in as user 01HF8K2P… — credentials saved to ~/.config/ohd-connect/credentials
```

CLI polls `/token` with `grant_type=urn:ietf:params:oauth:grant-type:device_code`
until success.

Credentials file layout (`~/.config/ohd-connect/credentials`, mode 0600):

```toml
[storage]
url = "https://ohd.cloud.example.com"

[token]
access = "ohds_..."
refresh = "ohdr_..."
expires_at_ms = 1736000000000

[client]
client_id = "..."   # from dynamic registration
```

### MCP (`ohd-connect-mcp`)

Same as browser-based clients (auth code + PKCE). The MCP host (Claude
Desktop, Claude.ai) opens a browser tab on first install via the standard MCP
auth flow; thereafter the MCP server holds the session in its config and
refreshes silently.

The MCP server only needs the OHD Storage URL (env `OHD_STORAGE_URL`); it
discovers `/authorize`, `/token`, and `/oauth/register` via RFC 8414
metadata.

## Grant management UX

Per `../spec/docs/components/connect.md` "Grant management" + the user's
clinical-data-sharing lens (the project default is **comprehensive sharing**
with privacy controls available; see `MEMORY.md` user note).

### Templates

Connect ships with the following grant templates (resolved by storage; Connect
just picks a `template_id` when calling `Grants.CreateGrant`):

| Template | Default scope |
|---|---|
| **Primary doctor** | All event types, all channels, sensitivity classes ON for general/biometric/lifestyle, OFF for mental_health/substance_use/sexual_health/reproductive (user toggles). Approval mode `auto_for_event_types` for `lab_result`, `clinical_note`, `medication_prescribed`. Expiry: 1 year. Notify-on-access: off. |
| **Specialist for one visit** | Same scope as primary doctor by default. Expiry: 30 days. Approval mode `always` for new specialists; user can downgrade. |
| **Spouse / family** | Read-only on emergency profile + recent vitals. Expiry: indefinite. Notify-on-access: on (low-key). |
| **Researcher with study** | Aggregation-only by default; specific study channels selectable. Strip-notes on. Expiry: per study. Audit visibility: high. |
| **Emergency break-glass** | Modeled as `is_template=1` `kind='emergency_template'`; cloned into a fresh active grant when break-glass fires. Defines the user's emergency profile. |

Connect's grant-creation flow:

1. User taps "Issue grant" → picks a template.
2. Edit screen shows the template's resolved scope as **defaults** (channels,
   approval policy, expiry). The user can flip any toggle but the defaults
   stand for users who don't want to micro-manage.
3. User adds a label (the operator/grantee name) and optional notes.
4. Submit → `Grants.CreateGrant` → returns the share artifact (`ohdg_…`
   token + optional QR + optional rendezvous URL for on-device storage).
5. User shares the artifact (NFC tap, QR scan, copy-paste, email).

### Inspect & revoke

The "active grants" list shows: label, template, expiry, last-used, queries
fired, rows returned, rows silently filtered. Tap a grant → detail view with:

- **Scope** (read-only display of effective rules)
- **Audit** (every read + write under the grant; one row per RPC)
- **Pending writes** (just this grant's pending submissions)
- **Actions**: Edit scope, Revoke, Force-expire-now

Revocation calls `Grants.RevokeGrant`; storage flips `revoked_at_ms` and
returns. Synchronous, not sync-deferred.

## Pending approval UX

Per the canonical spec. Connect surfaces:

1. **Inbox tab** — list of pending events ordered newest-first, grouped by
   submitter. Badge on the tab when count > 0.
2. **Per-event detail** — structured preview:
   - For measurements: side-by-side current value (last known) vs. submitted
     value, plus delta and unit-mismatch warning if any.
   - For clinical notes: full text with submitter / time / source attribution.
   - For prescriptions: medication, dose, schedule, prescriber.
   - For attachments: thumbnail + download.
3. **Actions**:
   - Approve → `Pending.ApprovePending`
   - Approve and trust this type from this grantee →
     `Pending.ApprovePending` + `Grants.UpdateGrant` adding the event_type to
     `auto_for_event_types`.
   - Reject (with optional reason free-text) → `Pending.RejectPending`.
4. **Bulk actions** — select multiple, approve / reject as a batch. Useful for
   a doctor visit producing many similar entries.

Auto-expiry happens server-side after the configurable window (default 7
days). Connect just renders status.

## Emergency settings

Per [`spec/screens-emergency.md`](spec/screens-emergency.md). Connect hosts the
**patient-side** screens. Eight settings sections:

1. Feature toggle (default: off).
2. Discovery (BLE beacon on/off).
3. Approval timing (timeout slider 10–300s, default 30s; default-on-timeout
   radio Allow/Refuse, default Allow).
4. Lock-screen behaviour (full dialog vs basic-info-only).
5. What responders see (history window 0/3/12/24h, per-channel toggles,
   sensitivity-class toggles).
6. Location (share GPS opt-in, default off).
7. Trusted authorities (project default + per-locale + user-added).
8. Advanced (bystander-proxy role, reset-to-defaults).

Plus the **emergency dialog** (above lock screen, full-screen modal with
countdown), the **cases tab** (active + closed cases), and the **post-access
notification** (auto-wake the user after a break-glass with a "review case"
deep link).

The dialog requires platform-specific work:

- **Android**: `Notification.Builder` with `setFullScreenIntent` + an Activity
  that uses `FLAG_SHOW_WHEN_LOCKED` + `FLAG_DISMISS_KEYGUARD`. APNs critical
  alerts have an Android equivalent via FCM `priority=high` + sound channel.
- **iOS**: APNs critical-alert (requires user opt-in once; OS-prompted) +
  `UNNotificationContent` with a registered critical-alert sound + an
  `interruption-level: critical` payload.

The `auto_granted` flag on a break-glass approval (timeout-default-allow) must
be visually distinct in both the audit view and the case detail view per the
designer-handoff doc.

## Connect MCP — tool list

Per [`spec/mcp-servers.md`](spec/mcp-servers.md) "Connect MCP". The v1 tool set
is hand-written (not auto-generated from `OhdcService`) because LLMs do
better with intent-shaped tools than CRUD endpoints.

### Logging tools

| Tool | Maps to | Notes |
|---|---|---|
| `log_symptom` | `Events.PutEvents` | symptom + severity + location + notes; resolves "I have a headache" → structured event. |
| `log_food` | `Events.PutEvents` | description + quantity + start/end + barcode (triggers OpenFoodFacts resolution server-side). |
| `log_medication` | `Events.PutEvents` | name + dose + status (taken/skipped/late/refused) + time. |
| `log_measurement` | `Events.PutEvents` | type + value + unit + time; resolves channels through `Registry.ResolveChannel`. |
| `log_exercise` | `Events.PutEvents` | activity + duration + intensity + start. |
| `log_mood` | `Events.PutEvents` | mood + energy + notes. |
| `log_sleep` | `Events.PutEvents` | bedtime + wake_time + quality + notes. |
| `log_free_event` | `Events.PutEvents` | namespaced custom event_type + structured `data` blob. |

### Reading tools

| Tool | Maps to |
|---|---|
| `query_events` | `Events.QueryEvents` |
| `query_latest` | `Events.QueryEvents` (limit=N, order=desc) |
| `summarize` | `Events.Aggregate` |
| `correlate` | `Events.Correlate` |
| `find_patterns` | `Events.QueryEvents` + statistical post-processing |
| `get_medications_taken` | `Events.QueryEvents(event_type='medication_administered')` + adherence aggregate |
| `get_food_log` | `Events.QueryEvents(event_type='meal')` + nutrition rollup |
| `chart` | `Events.Aggregate` + chart spec generation; returns `{image_base64, chart_spec, underlying_data}` |

### Grants

| Tool | Maps to |
|---|---|
| `create_grant` | `Grants.CreateGrant` |
| `list_grants` | `Grants.ListGrants` |
| `revoke_grant` | `Grants.RevokeGrant` |

### Pending review

| Tool | Maps to |
|---|---|
| `list_pending` | `Pending.ListPending` |
| `approve_pending` | `Pending.ApprovePending` |
| `reject_pending` | `Pending.RejectPending` |

### Cases

| Tool | Maps to |
|---|---|
| `list_cases` | `Cases.ListCases` |
| `get_case` | `Cases.GetCase` |
| `force_close_case` | `Cases.ForceCloseCase` |
| `issue_retrospective_grant` | `Cases.IssueRetrospectiveGrant` |

### Audit

| Tool | Maps to |
|---|---|
| `audit_query` | `Audit.AuditQuery`; emits the `auto_granted` flag on emergency entries. |

### Time handling

The MCP accepts both ISO 8601 and natural phrases ("yesterday", "30 minutes
ago", "last Tuesday"); resolves inside each tool with a date-parser before
hitting OHDC. Per `spec/mcp-servers.md` "Handling time input".

### Transports

- `stdio` — local install (Claude Desktop, Cursor, Continue).
- `Streamable HTTP` — remote (alongside OHD Storage at
  `https://<storage>/mcp/connect/`); auth via OAuth proxy delegating to the
  same OIDC providers OHD Storage uses.

## CLI command surface

Per `../spec/docs/components/connect.md` "OHD Connect CLI". Single binary
`ohd-connect`, clap-driven, sub-command tree:

```
ohd-connect <COMMAND> [args]

COMMANDS:
  login       Acquire a self-session token via OAuth device flow.
  logout      Revoke the current session.
  whoami      Print current identity / session metadata.

  log         Log an event.
    log measurement <channel> <value> [--unit] [--at <ts>] [--note <s>]
    log medication <name> [--dose] [--status taken|skipped|late|refused] [--at]
    log food <description> [--quantity] [--barcode] [--started] [--ended]
    log symptom <name> [--severity] [--location] [--note]
    log mood <mood> [--energy] [--note]
    log sleep [--bedtime] [--wake] [--quality]
    log exercise <activity> [--duration-minutes] [--intensity]
    log free <event_type> --data <json>

  query       Read events.
    query events [--type] [--from] [--to] [--last] [--limit]
    query latest <event_type> [--count]
    query summarize <event_type> [--period] [--agg]
    query correlate <a> <b> [--window-minutes]

  grant       Manage grants.
    grant create [--template] [--name] [--read] [--write] [--expires] [--policy]
    grant list
    grant show <id>
    grant revoke <id>
    grant audit <id>

  pending     Pending approval queue.
    pending list
    pending show <id>
    pending approve <id> [--also-trust-type]
    pending reject <id> [--reason]

  case        Case operations.
    case list
    case show <id>
    case close <id>
    case grant <id> [--name] [--read]   # retrospective grant

  audit       Audit log.
    audit list [--grant <id>] [--from] [--to]
    audit show <id>

  emergency   Emergency settings.
    emergency get
    emergency set <key> <value>
    emergency trust list
    emergency trust add <cert-path>
    emergency trust remove <id>

  export      Export / portability.
    export full [--out] [--format ohd|json]
    export pdf [--out]
    migrate init <new-storage-url>
    migrate finalize

  config      Local config.
    config show
    config set <key> <value>

  version     Print CLI version + protocol version.
```

The v0 binary in [`cli/`](cli/) implements only `--help` and `version`. Every
other subcommand is TBD.

## Notification handling

Per [`spec/notifications.md`](spec/notifications.md). Connect on each platform
must:

1. Register its push token (`fcm` / `apns` / `web_push`) on every launch via
   `Notify.RegisterDevice`. Storage tracks in `push_tokens`.
2. Receive pushes; the payload contains **no PHI** — just `category`,
   `ref_ulid`, `operator_label`.
3. On wake, fetch the actual content via OHDC under self-session
   (`Pending.GetPending` for `pending_write`, `Cases.GetCase` for
   `emergency`, etc.).
4. Compose the OS notification text **after** wake so it reflects fresh data
   and so even devices with notification-content-disabled show a meaningful
   line.
5. For `category=emergency`, route to the full-screen emergency dialog above
   lock screen (Android `setFullScreenIntent`, iOS critical-alert).
6. For `category=tunnel_wake`, do nothing UI-side — re-establish the OHD Relay
   tunnel and stay silent.
7. Honor user-set quiet hours (`notification_preferences` on the storage side
   — Connect just reads it for surfacing user setting in UI).

## Testing surface

For each form factor, the implementation phase is expected to provide:

| Test layer | Android | iOS | Web | CLI | MCP |
|---|---|---|---|---|---|
| Unit | JUnit / Robolectric | XCTest | Vitest | `cargo test` | Vitest |
| Integration (talks to ephemeral storage) | Instrumented test on emulator with Docker-Compose'd storage | XCUITest with Docker-Compose'd storage | Playwright | `cargo test --features integration` | `@modelcontextprotocol/inspector` against ephemeral storage |
| Conformance (against `../spec/docs/design/conformance.md` corpus) | Shared Rust harness via JNI | Shared Rust harness via FFI | Shared TS harness | Direct cargo runner | Same as web |

In this scaffolding phase only the CLI smoke test (`cargo run -- --help`)
runs.

## Cross-references

- Component spec: [`../spec/docs/components/connect.md`](../spec/docs/components/connect.md)
- OHDC wire protocol: [`../spec/docs/design/ohdc-protocol.md`](../spec/docs/design/ohdc-protocol.md)
- On-disk format: [`../spec/docs/design/storage-format.md`](../spec/docs/design/storage-format.md)
- Auth (self-session): [`spec/auth.md`](spec/auth.md)
- Notifications: [`spec/notifications.md`](spec/notifications.md)
- MCP server design + tool list: [`spec/mcp-servers.md`](spec/mcp-servers.md)
- Health Connect bridge (Android): [`spec/health-connect.md`](spec/health-connect.md)
- OpenFoodFacts: [`spec/openfoodfacts.md`](spec/openfoodfacts.md)
- Barcode scanning: [`spec/barcode-scanning.md`](spec/barcode-scanning.md)
- Emergency UX: [`spec/screens-emergency.md`](spec/screens-emergency.md)
- Glossary: [`../spec/docs/glossary.md`](../spec/docs/glossary.md)
- UX design brief: [`../ux-design.md`](../ux-design.md)
