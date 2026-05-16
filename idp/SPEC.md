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

A **first-party** OpenID Provider. `accounts.ohd.dev` is itself an OIDC
provider an OHD user holds an account with — they sign in with an **email
and password**. The IdP resolves that account to a stable OHD
`profile_ulid` through the SaaS account store and mints an OHD-issued
`id_token`. So `accounts.ohd.dev` can be "the OIDC provider" for an OHD
deployment with no external dependency — the simple, self-contained path.

It is **also** designed to broker: a later additive feature lets the IdP
federate to an upstream OIDC provider (Google, Microsoft, …) so a user
can sign in with an existing account instead. Federation needs
provider-side setup (registering an OAuth client), so it is deferred; the
email/password path ships first and is always available.

Passwords are held only as **argon2id hashes**, alongside the
recovery-code hashes already in the SaaS store — OHD never holds a
plaintext or reversible credential. The recovery code remains the
account-recovery path: a forgotten password is reset through it.

The OHD identity an `id_token` carries is the `profile_ulid` — the same
ULID the SaaS already mints. So an app that trusts `accounts.ohd.dev`
gets a stable, cross-app user identity for free.

## Scope

In scope:

1. **`ohd-idp`** — a Rust/axum service: the OIDC OP endpoints, the login +
   sign-up UI, RS256 signing-key management, the RP-client registry.
2. **Email/password auth** — the first-party path: an OHD account is an
   email + an argon2id-hashed password, held in the SaaS account store.
   Self-service sign-up and sign-in. The primary, always-available method.
3. **Recovery-code auth** — sign in with an OHD recovery code, and the
   password-reset path, both validated against the SaaS store.
4. **SSO sessions** — a bounded browser session at the IdP so a second RP
   login does not re-prompt.
5. **RP onboarding** — config-driven client registry (CORD, Connect, …);
   optionally RFC 7591 dynamic registration later.
6. **Upstream federation (later phase)** — `ohd-idp` becoming an OIDC RP
   of a configured upstream provider; deferred because it needs
   provider-side OAuth-client setup. Reuses the relay's verifier pattern.

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

[signup]
open = true                 # allow self-service email/password sign-up

[recovery]
enabled = true              # allow "sign in with a recovery code"

# Upstream identity providers the IdP federates to — OPTIONAL, later phase.
# ohd-idp is an OIDC RP of each. With none configured the IdP runs purely
# on its own email/password + recovery-code auth, which is the default.
# [[upstream]]
# id = "google"
# issuer = "https://accounts.google.com"
# client_id = "..."
# client_secret_env = "OHD_IDP_UPSTREAM_GOOGLE_SECRET"
# scopes = ["openid", "email", "profile"]

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
3. The user signs in with their **OHD email + password** — or picks
   "recovery code", or (once configured) an upstream provider. The
   `/login` page links to **sign-up** (`/signup`) when `signup.open`.
4. **Email/password:** the IdP looks the email up in the SaaS account
   store and verifies the password against its argon2id hash.
   **Recovery code:** the user enters it; the IdP checks it against the
   SaaS store. **Upstream (later):** the IdP — itself an RP — redirects to
   the upstream provider's `/authorize`, then exchanges + **verifies** the
   returned `id_token` at `/upstream/callback` (issuer allowlist, JWKS,
   signature, `exp`, `aud`, `nonce`).
5. The authenticated account already maps to a `profile_ulid` —
   email/password and recovery-code accounts resolve directly; an upstream
   identity resolves via `(upstream_iss, upstream_sub)`, minting a new
   profile on first sight. The IdP sets the SSO session cookie.
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

## Local → server identity: the ULID handoff

OHD is local-first: an on-device user already has a `profile_ulid` minted
on the phone, and never needs an IdP. Identity only enters when that user
moves to **server-side** storage — and the goal is that they keep the
*same* ULID, so their existing data carries over under one account.

So when a local-first user first creates an `accounts.ohd.dev` account
(or first signs in to server-side storage), the request **may carry a
preferred ULID** — the one the device already uses. The IdP adopts it as
the account's `profile_ulid` **iff** it is a well-formed ULID and not
already in use. Only on a collision is the user assigned a fresh ULID
(and would have to re-point their data). A ULID is 128 bits — a collision
is astronomically unlikely, so in practice the local ULID is always
preserved and the "forced to switch" path effectively never fires.

The carry mechanism (an RP threading the device's ULID through sign-up)
ships with the local→server migration feature; this section fixes the
rule the IdP applies when it does.

## Security

- **Passwords are stored only as argon2id hashes** in the SaaS account
  store — never plaintext, never reversible. Login compares against the
  hash; the hash never leaves the store.
- **Recovery codes** are likewise stored hashed; a recovery code both
  signs in and authorises a password reset.
- **Upstream `id_token`s** (when federation is later configured) are
  verified, never trusted on presentation — issuer allowlist, JWKS
  signature, `exp` / `aud` / `nonce`.
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

**Phase 2 — email/password login end to end.** `/authorize` → login +
sign-up UI → email/password verified against the SaaS account store → OHD
authorization code → `/token` → `id_token` → `/userinfo`. After this,
CORD logs in through `accounts.ohd.dev` for real (pointed at it by
config) — replacing CORD's placeholder OIDC. Upstream federation is a
later additive phase, not part of this build.

**Phase 3 — recovery-code auth + SSO.** The recovery-code login +
password-reset path; the bounded SSO session so a second RP login is
promptless; RP-Initiated Logout.

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
