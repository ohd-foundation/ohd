# OHD Identity — Implementation Spec

> Implementation-ready spec for **OHD Identity**, the OHD project's OpenID
> Connect provider. It is the `ohd-idp` service, deployed at
> `accounts.ohd.dev`, and it is what OHD CORD, OHD Connect, and future
> OHD apps authenticate users against. Peer to OHD Storage, OHD Relay,
> OHD SaaS, and OHD CORD. If this file conflicts with a canonical spec in
> `../spec/docs/`, the canonical spec wins.

## Why this exists

OHD has identity machinery, but no single OpenID Provider an app can be a
relying party of:

- **OHD SaaS** (`api.ohd.dev`) owns the account store — `profile_ulid ↔
  (oidc_issuer, oidc_subject)` bindings, recovery-code hashes, plan. It is
  the user *directory* but exposes no OIDC OP surface.
- **OHD Storage** acts as an OAuth AS for *self-session* — per-instance,
  for the on-device / self-hosted models (`connect/spec/auth.md`).
- **OHD CORD** is already a complete OIDC *relying party* — it just has
  nothing OHD-run to point at, so it ships with placeholder OIDC config.
- The **relay** has a battle-tested OIDC *verifier* (JWKS cache).

What is missing is the OP itself: a discovery document, a JWKS endpoint,
an `/authorize` + `/token` flow, RS256 `id_token` minting, a login UI, and
an RP registry. `ohd-idp` is that — and once it exists, **CORD and Connect
become plain relying parties of `accounts.ohd.dev`**, sharing one OHD
identity.

## What OHD Identity is

A **brokering** OpenID Provider. It does not store passwords. It
authenticates a user by federating to an upstream identity provider
(Google, Microsoft, Apple, …) or by an OHD **recovery code**, resolves
that to a stable OHD `profile_ulid` through the SaaS account store, and
mints an OHD-issued `id_token`. This continues the model OHD already uses
(`connect/spec/auth.md`: storage's AS "redirects to that provider's
`/authorize`") — OHD never holds a credential it could leak.

The OHD identity an `id_token` carries is the `profile_ulid` — the same
ULID the SaaS already mints. So an app that trusts `accounts.ohd.dev`
gets a stable, cross-app user identity for free.

## Scope

In scope:

1. **`ohd-idp`** — a Rust/axum service: the OIDC OP endpoints, the login
   UI, RS256 signing-key management, the RP-client registry.
2. **Upstream federation** — `ohd-idp` is itself an OIDC RP of one or more
   configured upstream providers; it verifies their `id_token`s (reusing
   the relay's verifier pattern).
3. **Recovery-code auth** — the no-upstream path: sign in with an OHD
   recovery code, validated against the SaaS store.
4. **SSO sessions** — a bounded browser session at the IdP so a second RP
   login does not re-prompt.
5. **RP onboarding** — config-driven client registry (CORD, Connect, …);
   optionally RFC 7591 dynamic registration later.

Out of scope:

- The account store itself — that stays OHD SaaS. `ohd-idp` is a *consumer*
  of it.
- Storage's per-instance self-session AS — unchanged; that path serves the
  on-device / self-hosted models and is not replaced.
- Authorization / grants — that is OHDC's job. The IdP only does
  *authentication* (who the user is), never *what they may read*.

## Deployment model

Like every OHD component, `ohd-idp` is deployable, not a single product
instance:

| Operator | `accounts.<domain>` | Upstream providers |
|---|---|---|
| **OHD Cloud** | `accounts.ohd.dev` | Google, Microsoft, Apple + OHD recovery code |
| **Clinic / employer** | their domain | their own IdP (often a single corporate one) |
| **Self-host** | optional | the on-device / self-hosted models keep storage's own AS; running `ohd-idp` is only needed when several OHD apps must share one login |

Everything operator-specific is config. The same image runs
`accounts.ohd.dev` and a hospital's on-prem identity service.

## Architecture

```
                accounts.ohd.dev  (Caddy: TLS, HTTP/3)
                        │
            ┌───────────┴───────────┐
            │        ohd-idp        │   Rust / axum
            │  ┌─────────────────┐  │
   RP ──────┼─▶│ OIDC OP surface │  │   /authorize /token /jwks
 (CORD,     │  │ login UI (SSR)  │  │   /.well-known/openid-configuration
  Connect)  │  │ upstream RP     │──┼──▶ Google / Microsoft / Apple …
            │  │ RP registry     │  │
            │  │ signing keys    │  │
            │  └────────┬────────┘  │
            └───────────┼───────────┘
                        │  account resolution
                        ▼
                   OHD SaaS account store
                   (profile_ulid ↔ oidc identity, recovery codes)
```

One crate, `ohd-idp`, in a new top-level `idp/` directory (peer to
`saas/`, `relay/`, `cord/`):

- **`ohd-idp`** — axum HTTP, SQLite via `rusqlite` for IdP-local state
  (auth codes, sessions, signing keys), `jsonwebtoken` + `rsa` for
  `id_token` signing, `reqwest` for upstream OIDC. The login UI is
  server-rendered HTML — no SPA; it is just a provider picker plus a
  recovery-code field.

### Account store: shared, not duplicated

`ohd-idp` resolves `(upstream_iss, upstream_sub) → profile_ulid` and
recovery codes through the **OHD SaaS account store**. For the OHD Cloud
single-host deployment the pragmatic v1 is a **shared SQLite database**
(`ohd-idp` and `ohd-saas` co-deployed, one volume). The clean longer-term
boundary is an internal SaaS HTTP API the IdP calls; the spec keeps that
open. Either way, `ohd-idp` never becomes a second source of truth for
accounts.

### Why Rust, why standalone

Rust matches the rest of the stack and lets the upstream-OIDC verifier be
shared with the relay. Standalone — rather than folding the OP into the
SaaS — because the OP surface (a stateful authorize/token state machine,
a login UI, signing-key rotation, an RP registry) is a different shape
from the SaaS's small REST CRUD, the SaaS is explicitly "tiny by design",
and a custom deployment may want the IdP without OHD's billing service.

## Configuration

A single `idp.toml`, env-overridable (`OHD_IDP_*`), mirroring the
`cord.toml` / `relay.toml` pattern.

```toml
[server]
listen = "0.0.0.0:8447"
issuer = "https://accounts.ohd.dev"     # the OIDC `iss` — must be exact
data_dir = "/var/lib/ohd-idp"

[store]
# v1: the shared SaaS SQLite database.
saas_db = "/var/lib/ohd-saas/ohd-saas.db"

[keys]
# RS256 signing key; generated + persisted on first launch, published at
# /jwks. Rotation keeps prior public keys in the JWKS for the overlap.
signing_key_file = "/var/lib/ohd-idp/signing-key.pem"
rotation_overlap_days = 7

[session]
sso_ttl_hours = 12          # browser SSO session at the IdP
code_ttl_secs = 120         # OHD authorization-code lifetime

# Upstream identity providers the IdP federates to. ohd-idp is an OIDC RP
# of each. At least one, or recovery-code-only.
[[upstream]]
id = "google"
issuer = "https://accounts.google.com"
client_id = "..."
client_secret_env = "OHD_IDP_UPSTREAM_GOOGLE_SECRET"
scopes = ["openid", "email", "profile"]

[recovery]
enabled = true              # allow "sign in with a recovery code"

# Relying parties. Static registry for v1.
[[client]]
id = "cord-web"
redirect_uris = ["https://cord.ohd.dev/v1/auth/callback"]
client_secret_env = "OHD_IDP_CLIENT_CORD_SECRET"

[[client]]
id = "connect-web"
redirect_uris = ["https://connect.ohd.dev/auth/callback"]
public = true               # PKCE-only, no client secret
```

## The OIDC OP surface

Standard OpenID Connect, Authorization Code + PKCE. All RP-facing.

| Method | Path | Purpose |
|---|---|---|
| GET | `/.well-known/openid-configuration` | Discovery — endpoints, supported algs, `issuer`. |
| GET | `/jwks` | The RS256 public keys (current + rotation overlap). |
| GET | `/authorize` | Authorization endpoint. Validates the RP `client_id` + `redirect_uri` + PKCE challenge, then runs the login (below). |
| GET | `/login` | The login UI — provider picker + recovery-code field. (Internal to the authorize flow.) |
| GET | `/upstream/callback` | Where a federated upstream provider redirects back; the IdP verifies the upstream `id_token` here. |
| POST | `/token` | Exchanges the OHD authorization code (+ PKCE verifier) for `id_token` + access token + refresh token. |
| GET | `/userinfo` | OIDC userinfo (bearer access token) — `sub`, `email`, `name`. |
| POST | `/logout` | Ends the IdP SSO session (RP-Initiated Logout). |
| GET | `/healthz` | Liveness. |

### The login flow

1. An RP (CORD) sends the browser to `/authorize?client_id=cord-web&
   redirect_uri=…&response_type=code&scope=openid…&state=…&
   code_challenge=…&code_challenge_method=S256&nonce=…`.
2. `ohd-idp` validates the client + redirect URI against the registry.
   - **SSO hit:** a valid IdP session cookie → skip straight to step 6.
   - **SSO miss:** render `/login`.
3. The user picks an upstream provider (or "recovery code").
4. **Upstream:** the IdP — itself an RP — redirects to the upstream
   provider's `/authorize`; the provider redirects back to
   `/upstream/callback`; the IdP exchanges + **verifies** the upstream
   `id_token` (issuer allowlist, JWKS, signature, `exp`, `aud`, `nonce`).
   **Recovery code:** the user enters it; the IdP checks it against the
   SaaS store.
5. The IdP resolves the authenticated identity to a `profile_ulid` via the
   SaaS store — `(upstream_iss, upstream_sub)` lookup, minting a new
   profile on first sight; recovery code resolves directly. It sets the
   SSO session cookie.
6. The IdP issues a one-time OHD **authorization code** bound to
   `(client_id, profile_ulid, redirect_uri, nonce, code_challenge)` and
   redirects the browser to the RP's `redirect_uri?code=…&state=…`.
7. The RP calls `POST /token` with the code + PKCE `code_verifier`. The
   IdP verifies the binding and returns the token set.

### The `id_token`

A JWT, **RS256**, signed with the key in `/jwks`:

```
iss  = https://accounts.ohd.dev
sub  = <profile_ulid>          — the stable OHD identity
aud  = <rp client_id>
exp, iat, auth_time
nonce                          — echoed from the RP
email, email_verified, name    — carried from the upstream provider
```

`sub` is the `profile_ulid`. Every RP that trusts `accounts.ohd.dev`
therefore identifies a user the same way OHD SaaS already does.

## Signing keys

An RS256 keypair, generated and persisted on first launch. The public key
is published at `/jwks` with a stable `kid`. **Rotation:** a new keypair
is generated; both public keys stay in the JWKS for
`rotation_overlap_days` so `id_token`s signed under the old key still
verify; then the old key is dropped. RPs (CORD, the relay's verifier)
already handle a `kid` miss by refetching the JWKS — no RP change needed.

## Security

- **No passwords.** The IdP federates; the only OHD-held credential is the
  recovery-code *hash*, which already lives in the SaaS store.
- **Upstream `id_token`s are verified**, never trusted on presentation —
  issuer allowlist, JWKS signature, `exp` / `aud` / `nonce`.
- **PKCE mandatory** for every RP, public or confidential.
- **Authorization codes** are single-use, short-lived (`code_ttl_secs`),
  and bound to the client + redirect URI + PKCE challenge.
- **Redirect URIs** are exact-matched against the registry — no wildcards.
- The IdP issues authentication, not authorization: an `id_token` says who
  the user is, never what data they may reach. Grants stay in OHDC.

## Relying-party migration

- **CORD** — already an OIDC RP (`cord-server/src/oidc.rs`). Migration is
  one config change: point `[[auth.provider]]` `issuer` at
  `https://accounts.ohd.dev`, register `cord-web` in the IdP. The
  placeholder OIDC config CORD ships with today is exactly this slot.
- **OHD Connect (web / Android)** — today uses storage self-session. In
  OHD Cloud mode it becomes an RP of `accounts.ohd.dev`. The on-device /
  self-hosted models keep storage's own AS — Connect picks the auth path
  by deployment mode. This is the larger migration and is its own phase.

## Implementation roadmap

Phased, each shippable independently — the way CORD was built.

**Phase 1 — service skeleton.** `idp/` workspace, `ohd-idp` axum binary,
`idp.toml` config, signing-key generation + `/jwks`,
`/.well-known/openid-configuration`, the RP-client registry, `/healthz`.
Docker image + compose service + Caddy `accounts.ohd.dev` route.

**Phase 2 — federated login end to end.** `/authorize` → login UI →
upstream OIDC RP flow → `/upstream/callback` verification → profile
resolution via the SaaS store → OHD authorization code → `/token` →
`id_token`. After this, CORD logs in through `accounts.ohd.dev` for real
(pointed at it by config) — replacing CORD's placeholder OIDC.

**Phase 3 — recovery-code auth + SSO.** The recovery-code login path; the
bounded SSO session so a second RP login is promptless; RP-Initiated
Logout.

**Phase 4 — Connect as a relying party.** Migrate OHD Connect (web, then
Android) to authenticate against `accounts.ohd.dev` in cloud mode, keeping
the self-hosted / on-device path on storage's AS.

**Deploy `accounts.ohd.dev`.** Config + signing key + upstream client
secrets, compose profile, Caddy route.

## Cross-references

- CORD (first relying party): [`../cord/SPEC.md`](../cord/SPEC.md)
- SaaS account store: [`../saas/SPEC.md`](../saas/SPEC.md)
- Self-session auth (storage's AS, the model this extends): [`../connect/spec/auth.md`](../connect/spec/auth.md)
- The upstream-OIDC verifier pattern to reuse: `relay/src/auth/oidc.rs`
- Deployment modes: [`../spec/docs/deployment-modes.md`](../spec/docs/deployment-modes.md)
