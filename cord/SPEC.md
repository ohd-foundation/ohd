# OHD CORD — Implementation Spec

> Implementation-ready spec for **CORD**, the OHD conversational agent. CORD
> lets a user (or an authorized clinician) talk to a health-data store in
> natural language. It is a **deployable web service**, peer to OHD Storage,
> OHD Relay, and OHD SaaS. This document is the contract between the design
> phase and the implementation phase; the data-link mechanism — how CORD
> reaches a user's storage — is specced separately in
> [`spec/data-link.md`](spec/data-link.md).

## What CORD is

Today "CORD" exists only as an on-device agent inside OHD Connect for Android
(`CordRunner.kt` / `CordTools.kt`): a local Anthropic tool-use loop driving the
`ohd-mcp-core` tool catalog against on-device storage via uniffi. It works, but
it is phone-only, single-user, and tied to one model provider with a
bring-your-own key.

CORD-the-service generalizes that into a hosted product:

- A **web app** at `cord.ohd.dev` (OHD Cloud's deployment) — and self-hostable
  by any clinic, ambulance service, or employer exactly like OHD Storage.
- A user signs in (OIDC), connects one or more **data sources** (their own
  phone storage, a self-hosted instance, a cloud instance), and chats.
- The agent runs **server-side**: model inference, the tool-use loop, and the
  MCP session to the data source all live in the CORD backend.

CORD is not an OHDC server and never holds a user's storage keys. It holds
**share credentials** — scoped grant tokens the user explicitly issued to it —
and reaches storage the same way any other grantee does: through OHD Relay.

> Naming note: the glossary records "CORD" as an *earlier* draft name for the
> read-only protocol/app, since renamed to **OHD Care**. That rename is
> superseded — "CORD" is now the proper noun for the conversational agent
> component (the Android code already uses it). `glossary.md` is updated to
> match.

## Scope

In scope:

1. **CORD web service** — `cord-server` (Rust/axum backend) + `cord-web`
   (browser SPA). Multi-user, multi-tenant-per-deployment.
2. **Authentication** — OIDC sign-in; per-deployment provider config. Sessions.
3. **Data sources** — connect, list, manage, disconnect storage links. Each is
   a share credential (grant token + relay rendezvous + cert pin) stored
   encrypted at rest. See [`spec/data-link.md`](spec/data-link.md).
4. **Agent** — the tool-use loop, reusing the `ohd-mcp-core` tool catalog. The
   data plane is an MCP session tunneled to the data source through OHD Relay.
5. **Model providers** — pluggable (Anthropic, Gemini, OpenAI). Deployment-wide
   system keys; per-deployment policy on whether users may add their own.
6. **Chat** — conversations, streaming responses, history persistence.
7. **Deployment** — Docker image, compose entry, Caddy route, config schema.

Out of scope:

- The OHDC server and grant minting — those live in OHD Storage; CORD only
  *consumes* grants the user issued to it.
- The relay itself — CORD is a relay *consumer*; OHD Relay is a separate
  component.
- The Connect-app **Shares** UI (the issuing side of the data link) — that is
  Connect work, specced in [`spec/data-link.md`](spec/data-link.md) and
  built in the Connect Android app.
- On-device CORD — the existing Android in-app agent stays; it is the
  zero-backend mode and is unaffected. CORD-the-service is additive.

## Deployment model

CORD is **a protocol participant, not a single product instance**. Like OHD
Storage, anyone can run it. The four realistic operators mirror
[`../spec/docs/deployment-modes.md`](../spec/docs/deployment-modes.md):

| Operator | Example | Auth | Model keys |
|---|---|---|---|
| **OHD Cloud** | `cord.ohd.dev` | OHD SaaS OIDC | OHD-managed system keys; users may add their own |
| **Clinic / ambulance** | `cord.clinic.example` | the clinic's IdP | clinic's system keys; BYO usually **disabled** for HIPAA/data-residency |
| **Employer / insurer** | custom | their IdP | their keys |
| **Self-host (one person)** | a VPS | a single OIDC provider, or OHD SaaS | the operator's key |

Everything that differs between operators is **config**, not code (see
"Configuration"). The same image runs `cord.ohd.dev` and a hospital's
on-prem box.

## Architecture

```
                 cord.ohd.dev  (Caddy: TLS, HTTP/3)
                        │
            ┌───────────┴────────────┐
            │       cord-server      │   Rust / axum
            │  ┌──────────────────┐  │
   browser ─┼─▶│ web API (SSE)    │  │
   (cord-web)  │ OIDC / sessions  │  │
            │  │ data-source reg. │──┼──▶ encrypted source store (SQLite)
            │  │ agent loop       │  │
            │  │ model providers  │──┼──▶ Anthropic / Gemini / OpenAI
            │  │ MCP relay client │──┼──▶ OHD Relay ──▶ user's storage
            │  └──────────────────┘  │      (phone / self-host / cloud)
            └────────────────────────┘
```

Two crates + one frontend, in a new top-level `cord/` directory (peer to
`relay/`, `saas/`):

- **`cord/` workspace** — `Cargo.toml`, `rust-toolchain.toml`, `deploy/`.
- **`cord-server`** — the backend binary. axum HTTP, SQLite via `rusqlite`,
  `jsonwebtoken` for sessions, OIDC verification reused from the relay's
  `auth/oidc.rs` pattern.
- **`cord-agent`** — a library crate: the tool-use loop, model-provider
  abstraction, and the MCP client. Depends on `ohd-mcp-core` for the tool
  catalog. Kept separate from `cord-server` so it is unit-testable headless
  and reusable (e.g. a future `cord` CLI).
- **`cord-web`** — Vite + React SPA (consistent with the Connect web form
  factor). Served as static assets by `cord-server` or by Caddy.

### Why Rust

`cord-agent` reuses `ohd-mcp-core` (the tool catalog + dispatch) and the OHDC
client / relay-tunnel code directly, with no FFI shim. One tool catalog, one
relay-frame implementation, shared with Storage, the MCP server, and Connect.

## Configuration

A single `cord.toml` (env-overridable, `OHD_CORD_*`). Loaded at startup;
mirrors `relay/src/config.rs`.

```toml
[server]
listen = "0.0.0.0:8446"
public_url = "https://cord.ohd.dev"
data_dir = "/var/lib/ohd-cord"

[auth]
# One or more OIDC providers. OHD Cloud points at OHD SaaS; a clinic points
# at its own IdP. Discovery via the issuer's /.well-known.
providers = [
  { id = "ohd", issuer = "https://api.ohd.dev", client_id = "...", client_secret_env = "OHD_CORD_OIDC_OHD_SECRET" },
]
session_ttl_hours = 720          # 30 days
session_jwt_secret_env = "OHD_CORD_SESSION_SECRET"

[models]
# System-wide provider keys. Every user of this deployment shares these.
default_provider = "anthropic"
[[models.provider]]
id = "anthropic"
kind = "anthropic"
api_key_env = "OHD_CORD_ANTHROPIC_KEY"
models = ["claude-opus-4-7", "claude-sonnet-4-6"]
[[models.provider]]
id = "gemini"
kind = "gemini"
api_key_env = "OHD_CORD_GEMINI_KEY"
models = ["gemini-2.5-pro"]

[models.byo]
# Whether users may register their own provider key. OHD Cloud: true.
# HIPAA-bound clinic: false (enforce inference stays on the operator's
# contracted/BAA-covered provider).
allow_user_keys = true

[relay]
# Default relay for data sources that don't carry their own. A share link
# can override this per-source.
default_relay = "https://relay.ohd.dev"
allow_custom_relay = true
```

`allow_user_keys = false` is the HIPAA-compliance lever: it forces all
inference through the operator's BAA-covered provider account.

## Backend surface (`cord-server`)

All under `/v1`. JSON, except chat streaming which is SSE.

### Auth & session

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/v1/auth/providers` | none | List configured OIDC providers (for the login screen). |
| GET | `/v1/auth/start?provider=<id>` | none | Begin OAuth 2.0 Authorization Code + PKCE; 302 to the IdP. |
| GET | `/v1/auth/callback` | none | OIDC redirect target; verifies `id_token`, mints a CORD session. |
| GET | `/v1/me` | session | Current user, plan/policy flags (`allow_user_keys`, …). |
| POST | `/v1/auth/logout` | session | Revoke the session. |

A CORD session is an HS256 JWT (`sub = cord_user_ulid`), same shape as
`ohd-saas`. The `cord_user_ulid` is derived from the OIDC `(issuer, subject)`
pair — stable across logins, never the storage `user_ulid`.

### Data sources

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/v1/sources/connect` | session | Body: a share link (`ohd://share/...`). CORD parses, verifies reachability, stores the credential encrypted. See data-link spec. |
| GET | `/v1/sources` | session | List the user's connected sources: label, status, last-reachable, scope summary. |
| GET | `/v1/sources/:id` | session | One source: full scope (what the share permits), relay, health. |
| POST | `/v1/sources/:id/refresh` | session | Re-probe reachability / re-handshake. |
| DELETE | `/v1/sources/:id` | session | Forget the source; wipe the stored credential. |

### Models

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/v1/models` | session | System providers/models + (if `allow_user_keys`) the user's own. |
| POST | `/v1/models/byo` | session | Register a user-supplied provider key (rejected if `allow_user_keys=false`). Stored encrypted. |
| DELETE | `/v1/models/byo/:id` | session | Remove a user key. |

### Chat

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/v1/chats` | session | List conversations. |
| POST | `/v1/chats` | session | New conversation `{ source_id, model }`. |
| GET | `/v1/chats/:id` | session | Full message history. |
| POST | `/v1/chats/:id/messages` | session | Send a user message; **response is `text/event-stream`** — streams assistant text deltas, tool-use status, and a final `done`. |
| DELETE | `/v1/chats/:id` | session | Delete a conversation. |

| GET | `/healthz` | none | Liveness + version. |

## Agent (`cord-agent`)

The server-side analogue of `CordRunner.kt`:

1. **Tool catalog** — fetched once per data source via the MCP session's
   `tools/list`. Same catalog `ohd-mcp-core` emits to Android CORD. No drift.
2. **Tool-use loop** — bounded (`MAX_TOOL_ROUNDS`, as in `CordRunner`). For
   each model round: send conversation + tools, stream text out, execute any
   tool calls via MCP `tools/call`, append results, loop.
3. **Model providers** — a `ModelProvider` trait with `anthropic`, `gemini`,
   `openai` impls. Resolves to a system key, or the user's BYO key when
   policy allows and the user picked their provider.
4. **Streaming** — provider streaming (SSE for Anthropic) is surfaced straight
   through to the browser's `text/event-stream`. (The Android client ships
   non-streaming today; `cord-agent` is streaming-first — see
   `deferred.md` "CORD chat polish".)

The system prompt is the `CordRunner` prompt, generalized: the agent is told
which data source it is bound to and that access is **scoped by the share**
(it may legitimately see "permission denied" for out-of-scope event types and
must not treat that as missing data).

## The data link (summary — full spec separately)

The crucial, currently-unbuilt piece. Detailed in
[`spec/data-link.md`](spec/data-link.md). In brief:

- In OHD Connect the user manages **Shares** (a first-class tab). A share is a
  grant plus an optional remote-access binding. Emergency break-glass is one
  pre-configured share.
- Activating a share for remote access registers a **per-share rendezvous**
  with a relay (OHD's or a custom one) and yields a **share link**:
  `ohd://share/<rendezvous_id>?token=<ohdg_…>&pin=<spki>&relay=<host>` plus an
  `https://` mirror, a QR, and an NFC payload.
- The user hands that link to CORD (or to a clinician's device). CORD's
  `POST /v1/sources/connect` parses it, opens the relay tunnel, completes the
  pinned inner-TLS handshake, and stores the credential encrypted.
- Thereafter CORD runs an **MCP session over the relay tunnel**. The phone
  runs a share-scoped MCP responder built on `ohd-mcp-core`; every tool call
  is filtered by the share's grant scope before results leave the device.

## Security

- **No storage keys.** CORD holds grant tokens, not `K_file`. Worst case
  compromise = the scope the user granted, revocable from Connect.
- **Credentials encrypted at rest.** Share tokens and BYO model keys are
  sealed with a deployment key (`OHD_CORD_DATA_KEY`); the SQLite file alone is
  inert.
- **Cert pinning.** CORD verifies the storage's self-signed cert SPKI against
  the `pin` from the share link; fails closed on mismatch
  (`relay-protocol.md` "TLS-through-tunnel").
- **Scope is enforced at the source, not at CORD.** Even a buggy/hostile CORD
  cannot exceed the share — the phone-side MCP responder applies the grant
  filter. CORD enforcement is defense-in-depth, not the boundary.
- **Audit.** Every MCP session shows up in the user's OHD audit log under the
  share's grant, like any other grantee.
- **BYO-key lockout** (`allow_user_keys=false`) keeps PHI-bearing prompts on
  the operator's contracted model provider.

## Deployment

- **Image** — `cord/deploy/Dockerfile`, multi-stage, `debian:trixie-slim`
  runtime (matches the GLIBC fix already applied to `ohd-mcp-rs`).
- **Compose** — a `ohd-cord` service in `deploy/host/docker-compose.yml`:
  port `8446`, volume `cord_data:/var/lib/ohd-cord`, env for the secrets.
- **Caddy** — a `cord.ohd.dev` block in `deploy/host/Caddyfile`, reverse-proxy
  to `ohd-cord:8446`, HSTS, gzip/zstd.
- **DNS** — `cord.ohd.dev` A record → the Hetzner host.

## Implementation roadmap

Each phase ships independently; a later phase failing never rolls back an
earlier one.

**Phase 1 — service skeleton. [done]** `cord/` workspace, `cord-server` axum binary,
`cord.toml` config, OIDC login + sessions, the data-source registry
(encrypted SQLite), `/healthz`, Docker + compose + Caddy. Data sources accept
a **direct storage URL** (cloud/self-host, CA-cert, no relay) so the service
is end-to-end testable before the relay path lands.

**Phase 2 — agent + chat. [done]** `cord-agent` crate: tool-use loop, `ModelProvider`
trait (Anthropic first), MCP client, BYO-key policy. Chat API with SSE
streaming. `cord-web` SPA: login, source list, chat. Usable against a direct
storage URL or `mcp.ohd.dev`.

**Phase 3 — Connect Shares tab.** Promote sharing in OHD Connect from the
buried "Profile & Access" screen to a first-class **Shares** tab: per-share
row + quick enable/disable toggle + detail screen; emergency modeled as a
pre-configured share. Share-link generation (`ohd://share/...`, QR, NFC).
Connect work; see `spec/data-link.md`.

**Phase 4 — relay data plane (the crucial part). [done]** Split into
independently shippable sub-tasks, each carrying its own tests:

- **4a — `ShareScope` in `ohd-mcp-core`.** Grant-scope enforcement on tool
  dispatch: intersect query filters, redact out-of-scope channels, hide
  write tools for read-only grants. Self-contained.
- **4b — reusable relay-client crate.** Extract
  `ohd-storage-server/src/relay_client.rs` so the Android binding and CORD
  share one implementation. Self-contained.
- **4c — cert-pinning / inner-TLS-through-tunnel wire.** Close the open item
  in `relay-protocol.md`: storage identity cert, SPKI pin in the artifact,
  fail-closed verifier, relay DATA-frame forwarding.
- **4d — phone-side share-scoped MCP responder.** Connect maintains the
  per-share tunnel, terminates inner-TLS, serves scoped MCP. Wires
  "Activate remote access". Needs 4a + 4b + 4c.
- **4e — CORD relay MCP client + `ohdr://` link.** CORD dials the rendezvous
  through a pinned session; the share-link open → connect flow. Needs 4b + 4c.
- **4f — end-to-end integration + tests.** The full CORD → relay → phone
  path; scope + cert-pin verification. Needs 4d + 4e.

**Deploy `cord.ohd.dev`.** Serve `cord-web` from `cord-server`, production
config + DNS + secrets, bring up the `ohd-cord` compose profile. Independent
of Phase 4 — CORD is usable in direct-source mode from Phase 2.

**Phase 5 — hardening.** Custom-relay config, push-wake of a sleeping phone,
per-share metering / rate limits, chat-history polish, token-level streaming.

## Cross-references

- Data link (shares, relay path, phone-side responder): [`spec/data-link.md`](spec/data-link.md)
- Relay wire protocol: [`../relay/spec/relay-protocol.md`](../relay/spec/relay-protocol.md)
- OHDC protocol: [`../storage/spec/ohdc-protocol.md`](../storage/spec/ohdc-protocol.md)
- Grants & access: [`../storage/spec/privacy-access.md`](../storage/spec/privacy-access.md)
- Connect (the share-issuing app): [`../connect/SPEC.md`](../connect/SPEC.md)
- SaaS account service: [`../saas/SPEC.md`](../saas/SPEC.md)
- Deployment modes: [`../spec/docs/deployment-modes.md`](../spec/docs/deployment-modes.md)
- Tool catalog crate: `storage/crates/ohd-mcp-core`
