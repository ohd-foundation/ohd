# OHD SaaS — account & billing service

Tiny opt-in service that fronts `api.ohd.dev`. It is **not** an OHDC
storage node — the user's health events never touch it. Its single
responsibility is the profile / plan / billing triangle the user described:

| Relation                    | Cardinality |
|-----------------------------|-------------|
| `profile_ulid` ↔ OIDC link  | 1 ↔ N       |
| `profile_ulid` ↔ plan       | 1 ↔ 1       |
| `profile_ulid` ↔ payment    | 1 ↔ N       |
| `profile_ulid` ↔ recovery hash | 1 ↔ 1     |

A profile starts out fully local (the Android client mints `profile_ulid`
and the 16×8 recovery code, prints them, and stores them encrypted). When
the user opts to "go online" the client uploads:

  - The `profile_ulid`
  - The `sha-256(recovery_code)` hash (never the code itself)
  - The current plan (`free` by default)

…in exchange for an access token. That token is used on every later call.

## Endpoints (v1)

| Method | Path                            | Auth     | Purpose                                                     |
|--------|---------------------------------|----------|-------------------------------------------------------------|
| POST   | `/v1/account`                   | none     | Register / claim a profile_ulid. Idempotent on profile_ulid. |
| GET    | `/v1/account/me`                | bearer   | Current profile, plan, linked identities, created_at.       |
| POST   | `/v1/account/recover`           | none     | Submit a recovery code, receive a new access token.         |
| POST   | `/v1/account/oidc/link`         | bearer   | Link an OIDC identity to the current profile.               |
| DELETE | `/v1/account/oidc`              | bearer   | Unlink (provider, sub).                                     |
| POST   | `/v1/account/oidc/claim`        | none     | "Already have an account?" — find a profile by OIDC sub.    |
| GET    | `/v1/account/plan`              | bearer   | `{plan, retention_days, max_storage_gb}`                    |
| POST   | `/v1/account/plan/checkout`     | bearer   | Stub — returns a placeholder Stripe checkout URL.           |
| GET    | `/v1/account/payments`          | bearer   | Payment history for the current profile.                    |
| GET    | `/healthz`                      | none     | Liveness.                                                   |

### Auth

Bearer tokens are HS256-signed JWTs scoped to a single profile:

```json
{ "sub": "<profile_ulid>", "iat": ..., "exp": ..., "iss": "ohd-saas" }
```

Tokens are minted at `/v1/account` and `/v1/account/recover` and
`/v1/account/oidc/claim` time. Server-side rotation: a token is valid for
90 days; refresh just by re-claiming via recovery or OIDC.

### Plans

| Plan | Retention | Max storage | Sync | Notes                       |
|------|-----------|-------------|------|-----------------------------|
| free | 7 days    | 25 MB       | no   | Local-only on the device.   |
| paid | unlimited | 5 GB        | yes  | OHD Cloud sync + recovery.  |

Tier limits are enforced **client-side** today — the server just hands the
numbers back so the client can render them and the upsell shows the gap.

### What we deliberately do NOT store

- Health events. Period.
- Recovery codes (only their hash).
- OIDC ID tokens (only the issuer + `sub`).
- Names, addresses, emails (except via OIDC at the issuer).

### Payment storage (for taxes / audit)

```text
payment_records {
  ulid TEXT PK,
  profile_ulid TEXT FK,
  created_at TEXT,
  amount_minor_units INTEGER,
  currency TEXT,
  provider TEXT,          -- 'stripe' once wired
  provider_charge_id TEXT,
  status TEXT,            -- 'pending'|'succeeded'|'refunded'|'failed'
  invoice_url TEXT
}
```

The provider-side data (card last4, billing address) stays at the
processor; we keep enough to file taxes and let the user see their
history.

## Status

Today the service is a stub: SQLite-backed, no payment provider wired,
OIDC linking accepts the issuer+sub at face value (no token verification
yet). Real OIDC verification + Stripe integration are tracked separately.

The Android client falls back gracefully when this service is unreachable
— the profile_ulid + recovery code are minted locally first, so the user
is never blocked on the network.
