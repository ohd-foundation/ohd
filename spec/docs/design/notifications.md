# Design: Notification Delivery

> How OHD wakes the user's phone for time-sensitive events: a clinician submitted a write, a grant was just used, an emergency-access request fired, a sync failed, a relay tunnel needs the phone awake, etc.

This is the deployment-side push-notification infrastructure — separate from the OHDC RPC surface. The OHDC layer says "the user should be notified about X"; the notification layer figures out *how* and ships the push.

## Why this exists separately

Several OHD flows need to wake or surface things to the user proactively:

| Flow | Source | Default urgency |
|---|---|---|
| Grant write submitted to pending queue | OHDC `PutEvents` under a grant with `approval_mode=always` (or any non-auto-commit case) | Normal |
| Grant query fired with `notify_on_access=true` | OHDC read RPCs under a grant with notify enabled | Low |
| **Emergency-access request** (about to render dialog) | `OhdcService.DeliverEmergencyRequest` post-cert-verification | **Critical** (APNs critical-alert; FCM high-priority) |
| **Emergency access granted** (interactive or auto via timeout) | After dialog resolves with grant issuance | Normal |
| **Emergency access rejected** | After dialog resolves with reject | Low |
| **Emergency case auto-closed** (inactivity sweep on case) | Background case lifecycle pass | Normal |
| **Emergency case handed off** (predecessor → successor) | Handoff RPC | Normal |
| **Authority cert revoked** (one of user's trusted roots distributed a deny entry, when v1.x revocation lever ships) | Post-poll | Low (informational) |
| **Trust root unreachable for >7 days** (storage couldn't refresh trusted-roots metadata) | Background poll failure threshold | Low |
| Pending grant approaching expiry | Background scheduler on storage | Low |
| Sync error from a cache | Cache itself surfaces locally + can also push if degraded long enough | Low |
| Relay tunnel wake-up (consumer attaching while phone is asleep) | Relay → push → Connect re-establishes tunnel | Silent (data-only push, no UI surface) |
| Suspicious activity flag | Operator-side anomaly detector | Normal |
| Audit retention/purge happened | Background pass | Optional, off by default |

These all funnel through the same delivery layer.

## Architecture

```
                 OHDC operation
                      │
                      ▼
            ┌──────────────────────┐
            │  ohd-storage         │
            │                      │
            │  Notification        │
            │  trigger             │
            │  (e.g. pending       │
            │   event submitted)   │
            └──────────┬───────────┘
                       │
                       ▼
            ┌──────────────────────┐
            │  Notification queue  │  System DB table
            │  (per-user, typed)   │
            └──────────┬───────────┘
                       │
                       ▼
            ┌──────────────────────┐
            │  Notification        │  Background dispatcher
            │  dispatcher          │  inside ohd-storage
            └──────────┬───────────┘
                       │
            ┌──────────┴──────────┬──────────────┐
            ▼                     ▼              ▼
       FCM (Android)         APNs (iOS)      Email (fallback)
            │                     │              │
            ▼                     ▼              ▼
       User's phone          User's phone     User inbox
            │                     │
            ▼                     ▼
       Connect mobile        Connect mobile
       (taps / wake)         (taps / wake)
            │                     │
            ▼                     ▼
       Fetches details         Fetches details
       via OHDC                via OHDC
       (the push payload
        contained no PHI)
```

## System-DB tables

### `push_tokens`

One row per (user, device).

```sql
CREATE TABLE push_tokens (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  user_ulid       BLOB NOT NULL,
  platform        TEXT NOT NULL,           -- 'fcm' | 'apns' | 'web_push' | 'email'
  token           TEXT NOT NULL,           -- the platform-issued token (or email address)
  device_label    TEXT,                    -- "Jakub's iPhone 15"
  registered_at_ms INTEGER NOT NULL,
  last_used_ms    INTEGER,
  invalid_at_ms   INTEGER,                 -- set when push platform reports invalid (uninstall, token rotated)
  invalid_reason  TEXT,
  UNIQUE (platform, token)
);

CREATE INDEX idx_push_user ON push_tokens (user_ulid) WHERE invalid_at_ms IS NULL;
```

Connect mobile registers tokens on every launch (FCM tokens rotate; APNs less so). Web Connect can register Web Push if the user opts in.

### `notification_preferences`

Per-user preferences for which trigger categories produce pushes.

```sql
CREATE TABLE notification_preferences (
  user_ulid          BLOB PRIMARY KEY,
  pending_writes     TEXT NOT NULL DEFAULT 'push',     -- 'push' | 'email' | 'silent' | 'off'
  notify_on_access   TEXT NOT NULL DEFAULT 'silent',
  emergency_access   TEXT NOT NULL DEFAULT 'critical', -- 'critical' (loud) | 'push' | 'silent'
  grant_expiring     TEXT NOT NULL DEFAULT 'push',
  sync_errors        TEXT NOT NULL DEFAULT 'off',
  suspicious_activity TEXT NOT NULL DEFAULT 'push',
  quiet_hours_start  INTEGER,                          -- minutes from midnight (local TZ)
  quiet_hours_end    INTEGER,                          -- if both set, non-critical pushes deferred to end
  email_address      TEXT                              -- optional fallback delivery
);
```

Defaults err toward "tell the user enough to be safe, not so much they tune out." Emergency access is always at least `push`; user can elevate to `critical` (which uses iOS critical-alert and Android high-priority FCM) but cannot disable.

### `notification_queue`

Pending and delivered notifications. The dispatcher reads from here.

```sql
CREATE TABLE notification_queue (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  user_ulid           BLOB NOT NULL,
  category            TEXT NOT NULL,        -- 'pending_write' | 'access' | 'emergency' | 'grant_expiring' | etc.
  urgency             TEXT NOT NULL,        -- 'silent' | 'low' | 'normal' | 'critical'
  payload_json        TEXT NOT NULL,        -- structured data for Connect to fetch / render after wake
  created_at_ms       INTEGER NOT NULL,
  scheduled_for_ms    INTEGER NOT NULL,     -- = created_at_ms unless deferred for quiet hours
  delivered_at_ms     INTEGER,
  delivery_status     TEXT,                 -- 'pending' | 'sent' | 'failed' | 'expired'
  attempts            INTEGER NOT NULL DEFAULT 0,
  last_error          TEXT
);

CREATE INDEX idx_notif_due ON notification_queue (scheduled_for_ms) WHERE delivered_at_ms IS NULL;
```

Retention: 7 days after `delivered_at_ms`, then purged.

## Push payload contract (privacy invariant)

**Push payloads contain no PHI.** This is a hard rule.

The push payload is the smallest amount of data that lets Connect render an actionable notification — typically a category + opaque reference + maybe a 1–2 word context label. Never the actual event content, vital values, medication names, or anything that could leak through an OS notification preview.

Example payloads:

```json
// Pending write submitted by Dr. Smith
{ "category": "pending_write", "ref_ulid": "01HF...", "operator_label": "Dr. Smith" }
```

```json
// Emergency access fired
{ "category": "emergency", "ref_ulid": "01HF...", "operator_label": "EMS Prague Region" }
```

```json
// Relay tunnel wake-up (silent — no UI surface)
{ "category": "tunnel_wake", "ref_ulid": "01HF..." }
```

When Connect receives a push, it taps into OHDC under self-session to fetch the actual content — pending event details, audit row, etc. The push is a wake-up signal; the actual disclosure happens over an authenticated channel.

Notification text shown to the user (the "title" / "body" the OS renders on the lock screen) is composed by Connect *after* wake — so even on devices with notification-content delivery disabled, the user sees "OHD: Dr. Smith would like to add an event" rather than "OHD: lab_result glucose 5.4 mmol/L".

The operator-label string is the one place a name might surface in the push. Connect can mask this in lock-screen previews if the user opts in to "private notifications."

## Provider integration

### FCM (Android)

`ohd-storage` includes an FCM HTTP v1 client. Operator provides FCM credentials in deployment config:

```yaml
notifications:
  fcm:
    project_id: "your-firebase-project"
    service_account_path: /run/secrets/fcm_service_account.json
```

Self-hosted operators bring their own Firebase project (free tier covers any reasonable user count). OHD Cloud uses the project's own Firebase.

For the silent tunnel-wake path: data-only message (no `notification` key, only `data`), `priority=high`, `apns-priority: 10`. iOS-side handled by APNs.

### APNs (iOS)

`ohd-storage` includes an APNs HTTP/2 client. Operator provides APNs credentials:

```yaml
notifications:
  apns:
    team_id: "ABC123"
    key_id: "XYZ789"
    bundle_id: "org.ohd.connect"
    key_path: /run/secrets/apns_p8
    environment: production            # or 'development'
```

Apple's APNs token-based auth (signed JWT per request).

For critical alerts (emergency-access only): set `apns-push-type: alert`, `apns-priority: 10`, and the alert payload includes `interruption-level: critical` and a registered critical-alert sound. Requires the user to have opted into critical alerts for the OHD app once, OS-prompted.

### Web Push

Standard W3C Web Push (VAPID). Connect web registers a service worker that subscribes; storage holds the subscription endpoint + VAPID keys + auth secret in the `push_tokens` row.

### Email fallback

If a user has no live push token (e.g. uninstalled, never installed Connect mobile, web-only) and they've configured `email_address`, the dispatcher falls back to email via SMTP. Operator config:

```yaml
notifications:
  email:
    smtp_host: smtp.example.com
    smtp_port: 587
    smtp_user: "noreply@your-domain.org"
    smtp_password_ref: env:SMTP_PASSWORD
    from: "OHD <noreply@your-domain.org>"
```

Email content follows the same no-PHI rule. Subject: "OHD: [category]". Body: a short context line + a deep link to open Connect. No vital values, no medication names, no clinical content.

## Quiet hours and deferral

Non-critical notifications during the user's `quiet_hours_start` to `quiet_hours_end` are deferred to `quiet_hours_end` (rescheduled in the queue, not dropped). Critical notifications (emergency access) bypass quiet hours always.

The dispatcher honors per-user TZ from `_meta.user_tz_name`; if not set, falls back to UTC quiet-hours.

## Failure handling

- Push platform returns "invalid token" → mark `push_tokens.invalid_at_ms`, fall through to next token / email.
- All channels failed → leave in queue with `delivery_status='failed'`, retry per backoff (1m → 5m → 30m → 2h → 12h, then expire).
- All tokens for a user invalid for 30+ days → notify the user via email (if configured) "Connect doesn't seem to be installed; please reinstall to continue receiving notifications."

Audit: significant delivery events (failures, expirations, opt-out detected) get a system-level audit row.

## What this doc deliberately doesn't cover

| | Where it lives |
|---|---|
| The triggers themselves (when an OHDC operation produces a notification) | Per-RPC behavior in [`../components/connect.md`](../components/connect.md) and OHDC `.proto` (Task #8) |
| Connect mobile's notification handling and rendering | App-side concern; not specced here |
| Critical-alert UX in iOS for emergency access | Emergency component design in `components/emergency.md` |
| The system-DB schema for sessions, OIDC identities, etc. | [`auth.md`](auth.md) |

## Cross-references

- Auth and system-DB tables: [`auth.md`](auth.md)
- Privacy / threat model: [`privacy-access.md`](privacy-access.md)
- Pending events flow: [`storage-format.md`](storage-format.md) "Write-with-approval"
- Care-side audit (which doesn't push to clinicians — operators have their own tools): [`care-auth.md`](care-auth.md)
- Emergency alerts: [`../components/emergency.md`](../components/emergency.md)
- Relay tunnel wake-up: [`../components/relay.md`](../components/relay.md) "Persistence"
