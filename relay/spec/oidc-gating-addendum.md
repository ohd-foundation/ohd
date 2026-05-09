# Addendum: Per-OIDC Registration Gating

> **Status**: implemented in `relay/` (this crate). Awaits incorporation
> into the canonical `relay-protocol.md` (which is read-only here).
>
> **Spec authority**: this addendum is informative until folded into the
> canonical wire spec. The implementation is the source of truth in the
> meantime.

## Why

`relay-protocol.md` "Storage registration" describes the registration
RPC as bearer-token-gated by a one-time code obtained out-of-band from
the relay's setup web UI. That works for hobbyist self-hosting but
doesn't fit:

- **OHD Cloud relay** — the cloud relay is operated by the OHD project
  itself; users authenticate via `accounts.ohd.org` (an OHD-operated
  OIDC IdP). The "go to setup web UI, copy a code" loop is
  fingers-crossed-paste-the-right-thing UX.
- **Clinic-run relay** — clinic IT operates the relay on prem; staff
  register their device storage as part of onboarding. The clinic's
  existing SSO (Okta / AzureAD / hospital ADFS) is the source of truth
  for "who is allowed to register a relay account". One-time codes
  printed in a setup UI lose to the SSO every time.
- **Public relay** — kept permissive: no allowlist, no IdP, anyone who
  reaches the URL can register. Same as today's behavior.

The gap between these three deployments is small once the relay
understands "OIDC issuer X is trusted; require an id_token from X to
register".

## Scope

**This addendum specifies**: registration-time OIDC gating only —
i.e. who is allowed to call `POST /v1/register`. It does NOT touch:

- the long-lived registration credential (still the source of truth for
  subsequent calls — heartbeat, deregister, tunnel-open, etc.)
- end-user / consumer auth (still grant-token via OHDC)
- the inner TLS handshake (still self-signed-cert + pin)

The OIDC identity is recorded alongside the registration as an
**audit-only** field. Subsequent RPCs do NOT re-check the OIDC token
(its lifetime is typically way shorter than the registration's;
re-checking on every heartbeat would be operationally awful and not
materially safer).

## Configuration

Operator-side, via `[auth.registration]` in `relay.toml`:

```toml
[auth.registration]
allowed_issuers = [
  { issuer = "https://accounts.ohd.org",    expected_audience = "ohd-relay-cloud" },
  { issuer = "https://accounts.google.com", expected_audience = "ohd-relay-cloud" },
]
jwks_cache_ttl_secs = 3600
require_oidc        = true
```

Three deployment shapes:

| Shape | `allowed_issuers` | `require_oidc` | Effect |
|---|---|---|---|
| **Permissive** (default) | empty / unset | n/a | Anyone can register, `id_token` ignored. Backwards-compatible with the original spec. |
| **Hard-gated** | one or more | `true` | `id_token` required; verified against the allowlist. Matches the OHD Cloud / clinic deployments. |
| **Soft-gated** (rollout / migration) | one or more | `false` | `id_token` verified when present; registration still works without one. Useful while a clinic is migrating staff onto SSO-bound onboarding. |

## RPC additions

### `POST /v1/register` — body

```json
{
  "user_ulid": "...",
  "storage_pubkey_spki_hex": "...",
  "push_token": ...,
  "user_label": "...",
  "id_token": "eyJhbGc..."   // NEW (optional)
}
```

`id_token` is a compact-encoded JWT. When present, the relay:

1. Decodes the header (`kid`, `alg`).
2. Decodes the payload to extract `iss` (unverified at this step).
3. Looks up the matching allowlist entry by `iss` (exact-match).
4. Resolves the issuer's JWKS via the standard OIDC discovery flow:
   `GET <issuer>/.well-known/openid-configuration` → `jwks_uri` →
   `GET <jwks_uri>`.
5. Verifies the JWT signature, `exp`, `nbf`, and `aud` (must include
   the configured `expected_audience`).
6. Records `(iss, sub)` alongside the registration row.

### `POST /v1/register` — error responses

| HTTP status | `code` | When |
|---|---|---|
| `401` | `OIDC_REQUIRED` | `require_oidc=true` and no `id_token` was presented. |
| `401` | `OIDC_VERIFY_FAILED` | `id_token` was presented but failed verification (issuer not allowed, signature invalid, expired, audience mismatch, etc.). The body's `error` field carries a human-readable reason for logs / dev UX. |

### `GET /v1/auth/info` — discovery

Public; no auth needed.

```json
{
  "registration_oidc_required": true,
  "allowed_issuers": [
    { "issuer": "https://accounts.ohd.org",    "expected_audience": "ohd-relay-cloud" },
    { "issuer": "https://accounts.google.com", "expected_audience": "ohd-relay-cloud" }
  ]
}
```

Surface only what's useful to storage's relay-discovery flow: the
required-OIDC bit and the issuer URLs + audiences. Operator-internal
info (e.g. who specifically is allowed) is NOT leaked here — that's
the IdP's job to enforce.

## JWKS cache

- Per-issuer cache, keyed by issuer URL.
- TTL configured via `jwks_cache_ttl_secs` (default 3600).
- On `kid` miss within TTL: one forced JWKS refresh; if the `kid` is
  still missing after that, reject with `OIDC_VERIFY_FAILED`. Standard
  OIDC key-rotation handling.
- On network failure during refresh while a cached set is present and
  not yet TTL-expired: the cached set is reused (degraded mode).

## Persistence

`registrations` SQLite table gains two nullable columns:

```sql
ALTER TABLE registrations ADD COLUMN oidc_iss TEXT NULL;
ALTER TABLE registrations ADD COLUMN oidc_sub TEXT NULL;
```

Migration is idempotent (uses `pragma_table_info` lookup) so existing
databases upgrade transparently. Both fields are NULL for permissive /
soft-gated registrations that arrived without a token.

## Security notes

- The relay does NOT issue or rotate the `id_token` itself — that's the
  IdP's job. The relay is purely a verifier.
- The relay does NOT use the `id_token` for subsequent RPCs. It runs
  the `long_lived_credential` hash check from registration onward,
  same as before. The OIDC identity is audit metadata.
- The `nonce` claim is NOT validated (this isn't an interactive
  login). `acr` / `amr` / `at_hash` are likewise not gated — operators
  who need AAL2 enforcement layer it on the IdP side.
- HMAC algorithms (`HS256` etc.) and `none` are explicitly rejected.
  The supported set is RS256/384/512, PS256/384/512, ES256/384, EdDSA
  — whatever `jsonwebtoken::DecodingKey::from_jwk` accepts.

## Storage-side discovery flow

Storage's relay-onboarding UX:

1. User pastes the relay URL → storage hits `GET /v1/auth/info`.
2. If `registration_oidc_required=true`, surface a "log in with X"
   button per allowed issuer (where the IdP's name comes from a
   storage-side issuer→display-name mapping; failing that, the issuer
   URL itself).
3. After OIDC dance completes, the IdP returns an `id_token` to
   storage (storage runs the OIDC flow, not the relay).
4. Storage then calls `POST /v1/register` with the `id_token` field
   set. On 201, save the rendezvous + credential as before.
5. On `OIDC_REQUIRED` / `OIDC_VERIFY_FAILED`, surface the relay's
   error message and offer to retry the OIDC dance.

## Cross-references

- Implementation: `relay/src/auth/oidc.rs`, `relay/src/server.rs`,
  `relay/src/config.rs`, `relay/src/state.rs`.
- Tests: `relay/tests/end_to_end_oidc_gating.rs` plus unit tests in
  `auth/oidc.rs`.
- Operator config example: `relay/deploy/relay.example.toml` →
  `[auth.registration]` block.
