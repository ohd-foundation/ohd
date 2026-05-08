# Design: Authentication & Self-Session

> How users prove who they are to OHD Storage. The OIDC delegation, the OAuth flows, the session tokens, where state lives, what the operator can configure, and what changes for on-device.

This doc covers **self-session** auth — the user authenticating as themselves. Grant tokens, device tokens, Care operator auth, and emergency / break-glass are separate flows specified elsewhere:

- Grant lifecycle and grant-token issuance — see [`privacy-access.md`](privacy-access.md) "Grants" and [`../components/connect.md`](../components/connect.md) (full grant-issuance UX TBD).
- Device tokens (sensors, lab pushers) — see [`privacy-access.md`](privacy-access.md) "Device token" (full device-pairing flow TBD).
- Care operator auth (clinic SSO, operator-bound audit) — see [`../components/care.md`](../components/care.md) (full operator-auth spec TBD).
- Emergency / break-glass — see [`../components/emergency.md`](../components/emergency.md) and [`../../design/screens-emergency.md`](../../design/screens-emergency.md).

## Role split

OHD Storage plays two OAuth roles simultaneously, and getting this clear up front prevents most of the confusion downstream:

| Role | Toward whom | What it does |
|---|---|---|
| **OAuth 2.0 Authorization Server (AS)** | OHD's own clients (Connect mobile, Connect web, Care, the CLI, MCP servers) | Issues OHD session tokens. Hosts `/authorize`, `/token`, `/oidc-callback`, `/.well-known/oauth-authorization-server`. |
| **OIDC Relying Party (RP)** | Identity providers (Google, Apple, Microsoft, GitHub, OHD Account, custom) | Verifies who the user is by delegating to a provider. Consumes the provider's `id_token`, throws it away after extracting `(iss, sub)`. |

The boundary is sharp. OIDC's only output is the answer to "who is this person?" — `(iss, sub)`. From that point on, OHD Storage runs its own access control: its own session tokens, its own grant model, its own audit. The OIDC provider's access tokens, scopes, and refresh tokens never leave the OHD Storage process.

## Provider catalog

OHD Storage ships with built-in support for a default set of OIDC providers. Operators enable a subset; users pick from the operator's enabled list at login.

| Key | Provider | Notes |
|---|---|---|
| `ohd_account` | OHD Account (project-run OIDC) | Free, run by the OHD project at `accounts.ohd.org`. Lets users authenticate without a big-tech account; lets OHD Cloud onboard users natively. From the protocol's view it's just another OIDC provider. |
| `google` | Google | Universal availability. |
| `apple` | Sign in with Apple | Required for App Store distribution; iOS users expect it. |
| `microsoft` | Microsoft / Entra | Office365 / enterprise users. |
| `github` | GitHub | Dev-friendly; useful for self-hosters and contributors. |
| `custom` | Any compliant OIDC issuer | Operator pastes a discovery URL (`https://issuer.example.org/.well-known/openid-configuration`); OHD figures out the rest. Authentik, Keycloak, hospital SSO, etc. |

Operator config example:

```yaml
# ohd-storage.config.yaml
auth:
  providers:
    - key: ohd_account
      enabled: true
    - key: google
      enabled: true
      client_id: <op_client_id>
      client_secret_ref: env:GOOGLE_CLIENT_SECRET
    - key: apple
      enabled: true
      # ... apple-specific config
    - key: custom
      enabled: true
      discovery_url: https://sso.our-clinic.org/.well-known/openid-configuration
      display_name: "Our-Clinic SSO"
```

Disabling a provider does not invalidate existing sessions for users who logged in via it; it only prevents future logins through that provider. Re-enabling restores it. (Linking a second provider to an existing account is supported — see "Multiple identities per user" below.)

**Identity (OHD Account) is free; storage (OHD Cloud) is paid.** These are two separate products operated by the OHD project. Anyone can use OHD Account to authenticate to anyone's OHD Storage; only OHD Cloud charges, and only for storage and compute. This is a project-level commitment in [`../02-principles.md`](../02-principles.md).

## Self-session flows

All self-session flows produce the same artifact: an opaque `ohds_…` access token plus an `ohdr_…` refresh token, server-tracked in the deployment's system DB. Three client shapes call for three OAuth flows.

### Browser-based clients (Connect mobile, Connect web, Care web)

Standard **OAuth 2.0 Authorization Code Flow with PKCE** (RFC 6749 + RFC 7636).

1. Client computes a PKCE code verifier and challenge.
2. Client opens a system browser (Custom Tabs on Android, ASWebAuthenticationSession on iOS, top-level navigation on web) to `https://<storage>/authorize?response_type=code&client_id=<client_id>&redirect_uri=<uri>&code_challenge=<challenge>&code_challenge_method=S256&state=<state>`.
3. OHD Storage renders its login page — branded by the operator, listing only that operator's enabled providers.
4. User picks a provider. OHD Storage redirects them to the provider's `/authorize` endpoint.
5. User completes provider login (with whatever the provider does — password, MFA, FaceID, security key, etc.).
6. Provider redirects back to OHD Storage's `/oidc-callback?code=<provider_code>&state=…`.
7. OHD Storage calls the provider's `/token` endpoint to exchange for the `id_token`.
8. OHD Storage verifies the `id_token` signature against the provider's JWKS, checks `iss`, `aud`, `exp`, and (if present) `nonce`.
9. OHD Storage extracts `(iss, sub)`, looks up `oidc_identities` row → if missing, mints `user_ulid`, creates per-user file, runs first-launch initializer.
10. OHD Storage redirects the client back to its `redirect_uri` with a one-time OHD authorization code: `<redirect_uri>?code=<ohd_code>&state=<state>`.
11. Client exchanges OHD code (plus PKCE verifier) at `/token` for `(ohds_access_token, ohdr_refresh_token, expires_in)`.
12. Client stores tokens in platform secure storage (Keychain, EncryptedSharedPreferences/Keystore, browser's `IndexedDB` with origin isolation).

PKCE is required for every client kind, including confidential clients. Public-client distinction is irrelevant in OHD because OHD Storage isn't trying to authenticate the client — it's authenticating the user, and PKCE guards the redirect.

### CLI clients (`ohd-connect`, `ohd-care`)

CLIs typically run in environments without a usable browser. Standard **OAuth 2.0 Device Authorization Grant** (RFC 8628).

```
$ ohd-connect login --storage https://ohd.cloud.example.com
Open https://ohd.cloud.example.com/device on any browser
Enter code:  BCDF-XYZW
Waiting for confirmation… (expires in 10 minutes)
✓ Logged in as user 01HF8K2P… — credentials saved to ~/.config/ohd/credentials
```

The CLI polls `/token` with `grant_type=urn:ietf:params:oauth:grant-type:device_code` and the device code; once the user has confirmed in the browser, the next poll returns `(ohds_…, ohdr_…)`.

### MCP servers

MCP servers behave the same as browser-based clients (OAuth 2.0 Authorization Code with PKCE). The MCP host (Claude Desktop, etc.) opens a browser tab on first install; thereafter the MCP server holds the session token in its config / OS secure-store and refreshes silently.

The MCP client only needs the OHD Storage URL. It discovers the `/authorize`, `/token`, and registration endpoints via **OAuth 2.0 Authorization Server Metadata** (RFC 8414):

```
GET https://<storage>/.well-known/oauth-authorization-server
→ {
    "issuer": "https://<storage>",
    "authorization_endpoint": "https://<storage>/authorize",
    "token_endpoint": "https://<storage>/token",
    "device_authorization_endpoint": "https://<storage>/device",
    "registration_endpoint": "https://<storage>/oauth/register",
    "code_challenge_methods_supported": ["S256"],
    "grant_types_supported": ["authorization_code", "refresh_token", "urn:ietf:params:oauth:grant-type:device_code"],
    "response_types_supported": ["code"],
    "token_endpoint_auth_methods_supported": ["none", "client_secret_basic"]
  }
```

This is identical for OHD Cloud, custom-provider deployments, and on-device storage reachable through OHD Relay (see [`../components/relay.md`](../components/relay.md)). The MCP client doesn't need to know which deployment shape it's talking to.

### Dynamic client registration

Clients that the operator hasn't pre-registered (custom MCP integrations, third-party apps, the user's own scripts) register themselves at startup using **OAuth 2.0 Dynamic Client Registration** (RFC 7591):

```
POST /oauth/register
{
  "client_name": "ohd-connect-cli",
  "redirect_uris": ["http://127.0.0.1:0/callback"],
  "grant_types": ["authorization_code", "refresh_token"],
  "token_endpoint_auth_method": "none"
}
→ { "client_id": "...", "client_id_issued_at": ... }
```

Operator config can disable dynamic registration and require pre-registered clients only — useful for clinic deployments where the IT team controls the client list.

## Token wire formats

All OHD-issued tokens are opaque random strings with a typed prefix and base64url payload. No JWTs, no claims, no signatures — server-tracked state.

| Token | Prefix | Payload | TTL | Notes |
|---|---|---|---|---|
| Access (self-session) | `ohds_` | 32 random bytes (base64url) | 1 hour | Short-lived; presented as `Authorization: Bearer …` on every OHDC call. |
| Refresh | `ohdr_` | 32 random bytes (base64url) | 30 days | Single-use; rotated on every refresh. |
| Grant (third party) | `ohdg_` | 32 random bytes (base64url) | Per `expires_at_ms` on the grant | Resolves to a `grants.id` via the `grants` table in the per-user file. |
| Device | `ohdd_` | 32 random bytes (base64url) | None (revocable) | Resolves to a `grants.id` with `kind='device'`. |

Why opaque random over JWT:

- **Revocation is instant.** Set a row, done. JWTs need either a denylist (defeats their value) or short TTL + refresh games.
- **No key rotation drama.** Rotating signing keys without breaking valid tokens is a recurring source of incidents.
- **No claim bloat.** A 32-byte token is ~43 bytes after base64url. JWTs in this position would be 700–1500 bytes per request.
- **Audit lookups are cheap.** Hashed token → row → user_ulid is one indexed read.

Tokens are stored as **SHA-256 hashes** server-side, not as plaintext. The plaintext exists only in transit and in the client's secure storage. A breach of the system DB does not leak usable tokens.

The wire encoding is fixed: `<prefix><base64url(32 random bytes)>`. Length is deterministic; no padding character. Prefixes let log scanners and secret-detection tooling spot leaked credentials.

## System-level state

Self-session state lives in the deployment's **system DB** — a separate SQLite file (or Postgres, on larger deployments) outside the per-user `.ohd` files. The boundary rule from [`storage-format.md`](storage-format.md) applies: rows that must outlive the user's data live system-level; rows that only make sense given the user's data live in the per-user file.

### `oidc_identities`

Maps an OIDC provider's identity to an OHD `user_ulid`.

```sql
CREATE TABLE oidc_identities (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  user_ulid       BLOB NOT NULL,                  -- the wire user identity
  provider        TEXT NOT NULL,                  -- 'ohd_account' | 'google' | 'apple' | issuer URL for custom
  subject         TEXT NOT NULL,                  -- provider-issued opaque id (`sub` claim)
  email_hash      BLOB,                           -- sha256(email) at signup, for login-hint matching only
  display_name    TEXT,                           -- user-supplied label for this identity (e.g. "Personal Google")
  linked_at_ms    INTEGER NOT NULL,
  last_login_ms   INTEGER,
  UNIQUE (provider, subject)
);

CREATE INDEX idx_oidc_user ON oidc_identities (user_ulid);
```

The privacy invariant: a leak of the per-user `.ohd` file leaks "user 01HF… has these glucose readings"; a leak of the system DB leaks "user 01HF… is `(google, abc123)`." Both required to deanonymize. Operators that hold both layers (clinics, employers, insurers) accept that responsibility per [`../02-principles.md`](../02-principles.md).

`email_hash` is stored hashed (not plaintext) so that a returning user logging in with a different OIDC provider (e.g. previously Google, now Apple) can be offered "Looks like you've signed up before with Google — would you like to link this account?" without retaining their actual email.

### `sessions`

Tracks issued self-session tokens.

```sql
CREATE TABLE sessions (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  user_ulid           BLOB NOT NULL,
  access_token_hash   BLOB NOT NULL UNIQUE,        -- sha256(plaintext access token)
  refresh_token_hash  BLOB NOT NULL UNIQUE,        -- sha256(plaintext refresh token)
  client_id           TEXT NOT NULL,
  issued_at_ms        INTEGER NOT NULL,
  access_expires_ms   INTEGER NOT NULL,
  refresh_expires_ms  INTEGER NOT NULL,
  last_used_ms        INTEGER,
  device_label        TEXT,                        -- user-editable, e.g. "iPhone 15 — Jakub"
  ip                  TEXT,                        -- nullable; some operators don't log
  ua                  TEXT,                        -- nullable
  revoked_at_ms       INTEGER,
  revoked_reason      TEXT                         -- 'logout' | 'rotated' | 'admin' | 'compromise' | 'expired'
);

CREATE INDEX idx_sessions_user        ON sessions (user_ulid);
CREATE INDEX idx_sessions_access_hash ON sessions (access_token_hash) WHERE revoked_at_ms IS NULL;
CREATE INDEX idx_sessions_refresh     ON sessions (refresh_token_hash) WHERE revoked_at_ms IS NULL;
```

Refresh-rotation: every `/token` call with `grant_type=refresh_token` invalidates the old `(access, refresh)` pair (sets `revoked_at_ms`, `revoked_reason='rotated'`) and inserts a new row. The refresh-after-refresh window of grace is 60 seconds (handles in-flight network races); a refresh used after rotation outside that window is a signal of token theft and triggers `revoked_reason='compromise'` on all of the user's active sessions plus an audit row.

`/auth/logout` revokes the current session. `/auth/logout-everywhere` revokes all of a user's sessions.

### `pending_invites`

Used in `invite_only` registration mode (see "Account-join modes" below).

```sql
CREATE TABLE pending_invites (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  invite_code     TEXT NOT NULL UNIQUE,            -- 8-12 char human-shareable, e.g. "ABCD-WXYZ"
  invited_by      BLOB,                            -- user_ulid of the issuing operator account; nullable for first admin
  email_hash      BLOB,                            -- optional binding; if set, the redeeming OIDC sub's email must match
  created_at_ms   INTEGER NOT NULL,
  expires_at_ms   INTEGER NOT NULL,
  redeemed_at_ms  INTEGER,
  redeemed_user   BLOB,                            -- user_ulid of the user who redeemed
  metadata_json   TEXT                             -- operator-defined: role, deployment-specific tags, etc.
);

CREATE INDEX idx_invites_code ON pending_invites (invite_code) WHERE redeemed_at_ms IS NULL;
```

## Account-join modes

Operators configure who can register on their instance. Three modes:

| Mode | Behavior | Typical operator |
|---|---|---|
| `open` | Any user who completes the OIDC flow gets a fresh `user_ulid` automatically. | OHD Cloud, large public SaaS deployments. |
| `invite_only` | Registration requires a valid `invite_code`. Without one, the OIDC flow succeeds but no `user_ulid` is minted; the user sees "Ask the operator for an invite." | Clinics, family servers, employer wellness, research consortia. |
| `closed` | No new accounts under any condition. Existing users continue to authenticate normally. | Migration-frozen instances, decommissioning operators. |

Config:

```yaml
auth:
  registration:
    mode: invite_only          # or 'open' or 'closed'
    invite_default_ttl_hours: 168
    invite_admin_users:        # who can issue invites (besides the bootstrap admin)
      - user_ulid: 01HF...
```

Invite issuance is itself an OHDC operation (`auth.issue_invite`) callable by users who have the `invite_admin` role in `roles_system` (a separate, simple role table — TBD as part of operator-side admin). A clinic operator generates a code, hands it to a patient (printed, emailed, in-person QR); the patient redeems it during the OIDC flow:

```
GET /authorize?…&invite_code=ABCD-WXYZ → flow proceeds, code consumed on success
```

Invites can optionally be email-bound (`email_hash` set): redeeming requires that the OIDC `id_token`'s email matches. Useful for "I sent this code to alice@example.com; only Alice should be able to use it." Codes are still single-use either way.

The boundary between "joining a deployment" (this section) and "granting access to your data" (grants in `privacy-access.md`) is strict. Joining = you have an account on someone's storage instance. Granting = your storage instance lets a third party read or write a slice of your data. The BLE-proximity self-invite scenario described in the emergency design ([`../components/emergency.md`](../components/emergency.md)) is grant-issuance, not registration.

## On-device sub-modes

When the user picks the on-device deployment topology (their `.ohd` file lives only on their phone or laptop), they pick one of two auth sub-modes at first launch.

### Mode A — On-device with OIDC binding (default)

1. First launch shows the same login page as cloud deployments — list of providers shipped with the on-device build.
2. User picks a provider; the standard OAuth + OIDC flow runs locally (the on-device storage is its own OAuth AS, addressable on `127.0.0.1:<port>` for the loopback redirect).
3. The provider's `(iss, sub)` is recorded in the local file's `_meta.oidc_identities` (a small embedded table, schema-identical to the system-DB version).
4. The per-user encryption key is derived from the device biometric/Keystore unlock (see [`storage-format.md`](storage-format.md) "Encryption" — full key-management flow TBD in encryption spec).
5. From the second launch onward, biometric unlock is sufficient; OIDC isn't re-run unless the user explicitly logs out or migrates.

Migration to cloud: the cloud instance, on first login with the same OIDC provider, sees a matching `(iss, sub)` and offers "We see you have a local OHD instance with this identity — import it now?" The user confirms, the local file uploads, the cloud instance takes over as primary. The on-device file becomes a cache (or is wiped, user choice).

### Mode B — On-device anonymous (local entity)

1. First launch shows a "name your local profile" prompt (e.g. "Jakub's data"). No OIDC flow.
2. App mints a fresh `user_ulid` locally, records `_meta.local_entity_label='<name>'`, `_meta.oidc_bound=false`.
3. Per-user encryption key derived as in Mode A.
4. Subsequent launches: biometric unlock.

Mode B is for users who refuse any cloud presence including OIDC providers' login pages. The tradeoff is loose migration: switching to cloud later requires the cloud instance to accept the file as a one-shot import under a freshly-minted cloud-side identity. The user's existing event ULIDs are preserved (the file is the source of truth); only the identity binding changes.

Mode B users can also later **upgrade in place** to Mode A — register an OIDC binding into the existing local file without changing any data — making the eventual cloud migration easier.

### Recovery in both modes

The encryption-key derivation includes an optional **recovery secret** — a 24-word BIP39 phrase generated at first launch and shown to the user once. The phrase, combined with a fixed app salt, can re-derive the file's encryption key in case the user loses biometric access (factory reset, device replacement before pairing).

If the user loses both biometric access and the recovery phrase, the local file is unrecoverable. This is stated explicitly in the first-launch UX. Mode A users have a fallback (re-auth on cloud and start fresh — losing local-only data); Mode B users have no fallback.

Full key-management spec — recovery details, multi-device pairing crypto, key rotation, export-encryption — lives in the forthcoming encryption design doc (Task #12).

### Multi-device on-device

A user with Mode A on-device storage can pair a second device (laptop, tablet) by running the OAuth flow on the second device against the first device's storage (via OHD Relay; see [`../components/relay.md`](../components/relay.md)). The second device receives a session token plus a wrapped copy of the file's encryption key (wrapped to a one-time ECDH public key the second device generates). Subsequent unlocks on the second device use its own biometric.

A user with Mode B can also pair, but the pairing flow doesn't involve OIDC; it's a direct device-to-device handshake (NFC tap or QR + passphrase). Same wrapped-key-handoff mechanism.

## Putting it together — what an operator implements

A reference OHD Storage deployment exposes the following auth-related HTTP endpoints. All paths are relative to the deployment URL; all use Connect-RPC where applicable, plain HTTP for OAuth-standard endpoints that browsers hit directly.

| Endpoint | Method | Notes |
|---|---|---|
| `/.well-known/oauth-authorization-server` | GET | RFC 8414 metadata. |
| `/authorize` | GET | OAuth Authorization endpoint (renders the login page; handles provider redirect). |
| `/oidc-callback` | GET | OIDC callback receiver from the upstream provider. |
| `/token` | POST | OAuth token endpoint (auth_code → tokens; refresh_token → tokens; device_code → tokens). |
| `/device` | GET, POST | Device Authorization Grant user-confirmation page. |
| `/oauth/register` | POST | RFC 7591 dynamic client registration. |
| `/auth/logout` | POST | Revoke current session. (OHDC RPC also available.) |
| `/auth/logout-everywhere` | POST | Revoke all of user's sessions. (OHDC RPC also available.) |

OHDC RPCs related to auth (full list in [`../components/connect.md`](../components/connect.md)):

- `Auth.WhoAmI` — returns the authenticated `user_ulid`, identity bindings, current session metadata.
- `Auth.ListIdentities` — list `oidc_identities` for the current user.
- `Auth.LinkIdentity` — initiate linking a new OIDC provider to the existing account (drives a fresh OAuth flow under the existing self-session).
- `Auth.UnlinkIdentity` — remove an OIDC binding (must keep at least one).
- `Auth.ListSessions` — show active sessions (for the "logged in on these devices" view).
- `Auth.RevokeSession` — revoke a specific session by id.
- `Auth.IssueInvite` — issue a registration invite (admin-role users only; only meaningful in `invite_only` mode).
- `Auth.ListInvites`, `Auth.RevokeInvite` — invite management.

## Multiple identities per user

A `user_ulid` can have multiple `oidc_identities` rows (e.g. "I usually log in with Google but my work account is Microsoft"). Linking is explicit:

1. User is logged in via existing identity A.
2. User goes to "Settings → Linked accounts → Link another provider."
3. App calls `Auth.LinkIdentity(provider=<new>)` which redirects to the new provider's OAuth flow under the existing self-session.
4. On callback, OHD Storage verifies the new `id_token`, ensures `(iss, sub)` doesn't already exist in `oidc_identities`, inserts a new row pointing at the same `user_ulid`.

Unlinking is symmetric, with one safety: a user must always have at least one identity bound. Unlinking the last one fails with `LAST_IDENTITY` and points the user at "delete account" if that's actually what they want.

A future "merge accounts" flow (user discovers they accidentally created two `user_ulid`s — one via Google, one via Apple — and wants to combine them) is out of scope for v1; manual operator intervention or export/import is the workaround.

## What this auth doc deliberately does NOT cover

| Topic | Where it goes |
|---|---|
| Grant token issuance, share artifacts, grantee import | [`privacy-access.md`](privacy-access.md) + a forthcoming grant-issuance UX doc |
| Device pairing for sensors / lab pushers | A forthcoming device-pairing flow doc |
| Care operator authentication (clinic SSO, operator binding to grant audit) | [`../components/care.md`](../components/care.md) + a forthcoming Care-auth doc |
| Emergency / break-glass auth | [`../components/emergency.md`](../components/emergency.md) and [`../../design/screens-emergency.md`](../../design/screens-emergency.md) |
| Encryption-key recovery, rotation, multi-device key handoff (the crypto details) | A forthcoming encryption design doc (Task #12) |
| Operator admin role model, audit of admin actions | A forthcoming operator-admin doc |

## Cross-references

- Conceptual three-auth-profile overview: [`privacy-access.md`](privacy-access.md) "The three auth profiles"
- OHDC operations and `Auth.*` RPC list: [`../components/connect.md`](../components/connect.md)
- On-disk schema (per-user `_meta.oidc_identities`, `events`, `grants`): [`storage-format.md`](storage-format.md)
- Relay (which carries auth traffic for on-device storage): [`../components/relay.md`](../components/relay.md)
- Project-level identity-vs-storage commitment: [`../02-principles.md`](../02-principles.md)
