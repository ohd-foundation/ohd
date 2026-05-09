# OHD Connect — Status / Handoff

> Snapshot of what's scaffolded, what runs, what's blocked, what's TBD per
> form factor. The goal of the scaffolding phase was to land a structurally
> correct skeleton so the implementation phase can fan out across platforms
> without re-litigating shape choices.

## OHDC wire/API version renamed to v0 (2026-05-09)

Connect now codegens and calls the storage-owned OHDC pre-stable API as
`ohdc.v0`, including Rust client module paths, MCP generated stubs, and web
Buf output references.

## Date
2026-05-09

## CLI v1 — first end-to-end demo (2026-05) — landed

The CLI no longer prints `--help` only. It now codegens the OHDC Rust client
(`connectrpc-build 0.4` against `../storage/proto/ohdc/v0/ohdc.proto`),
speaks **real Connect-RPC over HTTP/2**, and round-trips events through a
running `ohd-storage-server` under self-session auth. End-to-end demo at
[`demo/run.sh`](demo/run.sh) drives both binaries.

### What lands

- [`cli/build.rs`](cli/build.rs) — codegens `OhdcServiceClient` from the
  same `.proto` the storage server compiles. Uses `protoc-bin-vendored 3`,
  no system protoc required.
- [`cli/src/main.rs`](cli/src/main.rs) — clap router with subcommands
  `login`, `whoami`, `health`, `log {glucose|heart-rate|temperature|
  medication-taken|symptom}`, `query <kind> [--last-day|--last-week|
  --last-month|--from|--to]`, `version`.
- [`cli/src/credentials.rs`](cli/src/credentials.rs) —
  `~/.config/ohd-connect/credentials.toml` (mode 0600); `--storage` /
  `--token` flags override the file.
- [`cli/src/client.rs`](cli/src/client.rs) — Connect-RPC client over
  plaintext h2c (TLS is deployment-side; mirrors `../storage/STATUS.md`
  "HTTP/3 deferred" rationale).
- [`cli/src/events.rs`](cli/src/events.rs) — per-kind CLI-arg → channel-key
  mapping. Glucose accepts `mg/dL` and converts to canonical `mmol/L`;
  temperature accepts `F` and converts to canonical `C`.
- [`cli/src/timeparse.rs`](cli/src/timeparse.rs) — `--last-*` / ISO8601 →
  Unix-ms with `chrono`.
- [`cli/src/ulid.rs`](cli/src/ulid.rs) — Crockford-base32 display helpers
  (mirror of the storage core's `ulid::to_crockford`).
- [`demo/run.sh`](demo/run.sh) — boots the storage server on a temp DB at
  `127.0.0.1:18443`, mints a self-session token, drives the CLI through
  login → whoami → health → log glucose → query glucose, asserts the
  round-trip, tears down. Uses an isolated `XDG_CONFIG_HOME` so it doesn't
  clobber a developer's real credentials. Run with `bash demo/run.sh`.

### Smoke output

The full flow against a real storage server, captured during this pass:

```
$ ohd-connect whoami
storage:    http://127.0.0.1:18443
user_ulid:  1SD243RKCX6SC8WTB6W47YQ6HK
token_kind: self_session

$ ohd-connect health
status:           ok
server_version:   0.0.0
protocol_version: ohdc.v0
server_time_ms:   1778245130224

$ ohd-connect log glucose 6.4
committed 01KR3TPP280V8TPT99TRZYG8ZX at 1778245130312 ms

$ ohd-connect log glucose 120 --unit mg/dL
committed 01KR3TPP4T67FM5AAM6ZGVGC7Y at 1778245130394 ms

$ ohd-connect query glucose --last-day
ULID                        TIMESTAMP (UTC)            TYPE                      CHANNELS
01KR3TPP4T67FM5AAM6ZGVGC7Y  2026-05-08T12:58:50Z       std.blood_glucose         value=6.65993273467938
01KR3TPP280V8TPT99TRZYG8ZX  2026-05-08T12:58:50Z       std.blood_glucose         value=6.4
(2 events)
```

The 120 mg/dL → 6.66 mmol/L conversion lands canonically; the
`std.glucose` alias resolves to `std.blood_glucose` server-side via the
registry's `type_aliases` table.

### HTTP/3 client (2026-05-08, landed)

Two transports now ship in `cli/src/client.rs`:

- **HTTP/2 over h2c** — original v1 path; uses
  `connectrpc::client::Http2Connection`. Selected by `http://` URLs.
- **HTTP/3 over QUIC** — new `H3RawClient` talks directly to a server's
  QUIC port using `quinn 0.11` + `h3 0.0.8` + `h3-quinn 0.0.10` +
  `rustls 0.23`. Selected by `https+h3://host:port` URLs. Bypasses
  connectrpc on the client side (connectrpc 0.4 ships only
  `Http2Connection`); encodes / decodes the Connect-Protocol unary +
  streaming wire format directly using the buffa codegen messages.

`OhdcClient` is now a thin facade over either transport, exposing the
five v1 methods (`health`, `who_am_i`, `put_events`, `query_events`,
`get_event_by_ulid`) with owned-message returns regardless of which path
is in use. CLI subcommands route through these methods unchanged.

A new global flag `--insecure-skip-verify` is supported for dev /
self-signed certs (matches the storage server's
`http3::dev_self_signed_cert`). Production HTTP/3 mode (system trust
roots) is not yet wired — the client will refuse to connect over
`https+h3://` without `--insecure-skip-verify` and surface the missing
trust-store integration as the actionable next step. Pickup: pull in
`rustls-native-certs` and wire it into the verifier branch.

`query_events` over HTTP/3 collects the entire server-streaming response
eagerly and emits it as a single `futures::stream::iter`. Sufficient for
v1 (small result sets); when streaming-during-receive becomes a workload,
swap to a lazy `async_stream::stream!` that pulls `recv_data` chunks per
poll. The HTTP/2 path is unchanged.

Smoke test in `cli/tests/h3_client_smoke.rs` boots a minimal in-process
h3 server stub that returns a canned `HealthResponse` and validates the
client transport (encode → send → recv → decode). End-to-end against the
real storage HTTP/3 listener is exercised by the storage workspace's
`tests/end_to_end_http3.rs`.

### Decisions to flag (CLI v1)

1. **MSRV bump 1.83 → 1.88** (in `cli/Cargo.toml`) to satisfy `connectrpc
   0.4` / `buffa 0.5`. Same bump as `../storage/`.
2. **Plaintext h2c on HTTP/2 path; HTTP/3 over TLS**. The CLI rejects
   plain `https://` URLs and points the user at either `http://host:port`
   (h2c) or `https+h3://host:port` (HTTP/3 / QUIC). Adding HTTPS over
   HTTP/2 is mechanical (swap `Http2Connection::connect_plaintext` for
   the rustls variant) once the storage server stops being fronted by
   Caddy in dev.
3. **No shared `ohdc-client-rust` crate yet**. Both the storage server and
   the CLI codegen the same `.proto` directly. The "publish one
   `ohdc-client-rust` crate consumed by all Rust callers" idea from
   `shared/ohdc-client-stub.md` is still the right end-state — it just
   isn't required for the v1 demo, and forking the codegen to two trees
   would have meant cross-crate path mangling for build.rs.
4. **Token in TOML, not yet in the OS keyring**. v1 stores the bearer in
   `~/.config/ohd-connect/credentials.toml` chmod 0600. The `keyring` crate
   integration (Secret Service / Keychain / Credential Manager) is the
   v1.x deliverable; the file path stays as a cross-platform fallback.
5. **No device-flow login yet** (storage hasn't shipped `/authorize`,
   `/token`, `/device`, `/oauth/register`). `ohd-connect login` accepts a
   token issued out-of-band by `ohd-storage-server issue-self-token`. When
   storage's HTTP-only OAuth surface lands, `login` grows the device-flow
   poll loop without changing the on-disk credentials format.
6. **Channel rendering is value-only for the v1 demo**. The query table
   prints `channel=value`; enum ordinals print `[#N]` rather than the
   resolved enum label, because `Registry.ResolveChannel` is one of the
   storage RPCs that still returns `Unimplemented`. When that lands the
   CLI grows a single helper that turns the ordinal into a label.

### What's stubbed / TBD (CLI v1.x)

Tracking with `../STATUS.md` "What's blocked / TBD per form factor — CLI":

- `grant`, `pending`, `case`, `audit`, `emergency`, `export`, `config` —
  the storage RPCs return `Unimplemented` today (`../storage/STATUS.md`).
- ~~Device-flow login (`oauth2` crate + the storage HTTP OAuth surface).~~
  Landed 2026-05-09: `ohd-connect oidc-login --issuer URL --client-id ID`
  runs the full OAuth 2.0 Device Authorization Grant (RFC 8628) via the
  `oauth2` crate v5. Discovery (RFC 8414) hits
  `/.well-known/oauth-authorization-server` with fallback to
  `/openid-configuration`. The flow is end-to-end against any compliant
  OIDC issuer; gating now is the storage HTTP OAuth surface itself
  (`/authorize`, `/token`, `/device` not yet exposed by storage v0).
- ~~System-keyring credentials backend.~~ Landed 2026-05-09: `--kms-backend
  auto|keyring|passphrase|none`. Default `auto` tries the OS keyring
  (Linux Secret Service / macOS Keychain / Windows Credential Manager)
  via the `keyring` crate; falls back to passphrase-derived AES-GCM
  (Argon2id KDF) on headless machines. Legacy plaintext-TOML
  credentials still load (back-compat).
- `ohd-connect logout` clears tokens locally; server-side revocation via
  storage's `/auth/logout` lands when storage exposes the OAuth surface.
- Token refresh (`oauth2`'s refresh-token grant is wired in
  `oidc::DeviceFlowClient::refresh`; not yet used automatically before
  expiry — every `whoami` / `query` / `log` call still treats the
  access token as opaque). Wire `refresh_if_needed` into `client.rs`'s
  request path as the next step.
- The richer subcommand surface in [`SPEC.md`](SPEC.md) "CLI command
  surface" (e.g. `log measurement <channel> <value>` for ad-hoc registered
  types, `log free <event_type> --data <json>` for namespaced custom
  types, `query summarize / correlate / latest`).
- TLS / HTTPS support (mechanical swap once storage isn't always behind
  Caddy in dev).
- Conformance-corpus driver mode (`cli/SPEC.md` testing surface).

## What's done

### Top-level
- [x] [`README.md`](README.md) — component overview, dir layout, build/run per
      form factor, codegen drop zone.
- [x] [`SPEC.md`](SPEC.md) — implementation-ready spec covering all five form
      factors; OHDC client surface; OIDC self-session flows per form factor;
      grant management, pending review, audit, cases, emergency settings;
      Connect MCP tool list; CLI command surface; notification handling.
- [x] [`spec/`](spec/) — verbatim copies of canonical spec files Connect needs
      (`auth.md`, `notifications.md`, `mcp-servers.md`, `health-connect.md`,
      `openfoodfacts.md`, `barcode-scanning.md`, `screens-emergency.md`) +
      [`spec/README.md`](spec/README.md) index.

### Form factors
- [x] [`android/`](android/) — Gradle-Kotlin-DSL skeleton + first-pass
      Compose UI exercising the uniffi pipeline. Setup → Main flow with a
      four-tab bottom-bar shell (Log / Dashboard / Grants / Settings),
      Material3 dark theme defaulting per `ux-design.md`,
      `data/StorageRepository.kt` wrapping the uniffi handle, and
      `data/Auth.kt` holding the self-session bearer + first-run flag.
      The uniffi calls are real (✅ wired 2026-05-09 against the full
      `ohd-storage-bindings` surface — `OhdStorage.create(...)`,
      `putEvent(...)`, `queryEvents(...)`, `listGrants(...)`,
      `createGrant(...)`, `revokeGrant(...)`, `listPending(...)`,
      `approvePending(...)`, `rejectPending(...)`, `listCases(...)`,
      `getCase(...)`, `forceCloseCase(...)`,
      `issueRetrospectiveGrant(...)`, `auditQuery(...)`,
      `getEmergencyConfig(...)`, `setEmergencyConfig(...)`,
      `exportAll(...)`); the **only** runtime prerequisite is the
      [`BUILD.md`](android/BUILD.md) Stage 1 (cargo-ndk → `.so`) and
      Stage 2 (uniffi-bindgen → Kotlin) flow.
      `app/build.gradle.kts` adds JNA 5.14 (uniffi runtime), the
      `buildRustCore` documentation task, and the per-ABI NDK filter.
      `AndroidManifest.xml` declares `INTERNET`, `POST_NOTIFICATIONS`,
      `USE_BIOMETRIC`, `WAKE_LOCK`. **Not built locally** — the NDK
      isn't in the scaffolding env; building requires the BUILD.md
      recipe.
- [x] [`ios/`](ios/) — `Package.swift` for SwiftPM iOS target, stub
      `ContentView.swift`.
- [x] [`web/`](web/) — **First-pass SPA landed (2026-05-09).** Vite +
      React 18 + TS + react-router-dom + `@connectrpc/connect-web` against
      the storage component's wire schema. Six routed tabs (Log /
      Dashboard / Grants / Pending writes / Pending reads / Settings)
      with sub-pages for Storage / Emergency / Cases / Delegates /
      Export / Appearance. Self-session auth via URL `?token=ohds_…` or
      paste-token form. 14 OHDC RPCs wired (Health, WhoAmI, PutEvents,
      QueryEvents, GetEventByUlid, ListGrants, CreateGrant, RevokeGrant,
      ListPending, ApprovePending, RejectPending, ListCases, CloseCase,
      CreateCase). Dark theme default per `ux-design.md`; mobile-first
      with bottom-bar nav, sidebar on desktop. Sparklines for dashboard
      charts. 21 vitest tests pass (`pnpm test`). `pnpm build`
      produces a ~404 KB / 121 KB gzipped bundle. Dev server on :5174
      (avoiding care/web's :5173). See [`web/STATUS.md`](web/STATUS.md)
      for full handoff (UX choices, what's stubbed, follow-up RPC
      wiring once storage ships AuditQuery / Aggregate / ReadSamples /
      Export).
      - **F. `require_approval_per_query` UI** — ✅ landed 2026-05-09 as
        `/pending-queries` route + `PendingQueriesPage` (mirrors the
        write-approval shape with multi-select bulk approve / reject).
        Storage core has the helpers (`list/approve/reject_pending_query`,
        migration `005_pending_queries.sql`); the proto RPCs aren't yet
        wire-exposed in `storage/proto/ohdc/v0/ohdc.proto`. Connect-web
        falls back to an in-memory mock with an explicit "mock" banner;
        the client probe in `client.ts` flips automatically when the
        proto sweep adds the RPCs.
      - **G. Light theme toggle** — ✅ landed 2026-05-09 as
        Settings → Appearance: three-way picker (System / Dark / Light)
        persisted to `localStorage["ohd-connect-theme"]`. Bootstrap-time
        apply in `main.tsx` (no flash of wrong theme).
- [x] [`cli/`](cli/) — Cargo bin crate `ohd-connect`, clap-driven, prints
      `--help` and `version` subcommand. **Smoke test passes.**
- [x] [`mcp/`](mcp/) — **Python + FastMCP** server with all 27 tools from
      `SPEC.md` "Connect MCP — tool list" registered against a stubbed
      OHDC client. `pyproject.toml` (uv / hatchling), `src/ohd_connect_mcp/`
      package, `tests/` with FastMCP `Client` smoke tests. Replaces the
      original TypeScript scaffold per the Pinned implementation decisions
      in the repo root `README.md`. See [`mcp/STATUS.md`](mcp/STATUS.md)
      for the OHDC wire-up integration point.

### Shared
- [x] [`shared/ohdc-client-stub.md`](shared/ohdc-client-stub.md) — placeholder
      describing the codegen drop layout.

## Smoke tests

| Form factor | Command | Status |
|---|---|---|
| Android | `cd android && ./gradlew :app:assembleDebug` | **Not run** — needs Stage 1 (cargo-ndk → `.so`) + Stage 2 (uniffi-bindgen → Kotlin) first. NDK isn't in the scaffolding env. Documented in [`android/BUILD.md`](android/BUILD.md). |
| iOS | `cd ios && swift build` | **Not run** — needs Xcode toolchain. Documented in `ios/README.md`. |
| Web | `cd web && npm install && npm run build` | **Not run** — `npm install` skipped per scaffolding constraint. Documented in `web/README.md`. |
| CLI | `cd cli && cargo build && cargo run -- --help` | **PASS** — see Verification below. v1 also rounds a real event through `ohd-storage-server` over Connect-RPC; demo at `demo/run.sh`. |
| MCP | `cd mcp && uv sync && uv run pytest` | **PASS** — 5 tests, all 27 tools registered against the stubbed OHDC client. Server boots via `uv run python -m ohd_connect_mcp`. |

### CLI smoke test verification

Verified 2026-05-08 by the scaffolding agent on Rust 1.94.0 / Arch Linux:

```
$ cd connect/cli && cargo build
   Compiling ohd-connect v0.0.0 (.../connect/cli)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.22s

$ cargo run -- --help
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.02s
     Running `target/debug/ohd-connect --help`
OHD Connect — terminal interface to OHDC under self-session auth

Usage: ohd-connect <COMMAND>

Commands:
  version  Print CLI version
  help     Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

$ cargo run -- version
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.01s
     Running `target/debug/ohd-connect version`
ohd-connect 0.0.0
ohdc protocol: v1 (planned)
```

## Decisions to flag

### CLI is Rust, not Python
The earlier `ux-design.md` brief mentioned "pip package" / Homebrew + curl
distribution. We chose **Rust** for the v1 CLI:

- Reuses `ohdc-client-rust` codegen drop directly with no shim crate.
- Optionally links `libohdstorage` (the OHD Storage Rust core) for in-process
  on-device REPL — single static binary, no Python interpreter needed on
  target.
- Single release pipeline shared with the storage server.
- Distributable as a static binary via Homebrew formula, curl-to-bash, GitHub
  Releases — no per-platform Python packaging.

This is stated in [`README.md`](README.md) and [`SPEC.md`](SPEC.md) "Why Rust
for the CLI".

### MCP server is Python + FastMCP (was TypeScript)
The original scaffold used `@modelcontextprotocol/sdk` on Node + TypeScript.
That decision was **reversed** at the repo level — the Pinned implementation
decisions in the root [`README.md`](../README.md) now mandate Python +
FastMCP for all three MCP servers (Connect, Care, Emergency), per
[`spec/docs/research/mcp-servers.md`](../spec/docs/research/mcp-servers.md).

The reasoning:

- Standalone FastMCP (`fastmcp` 3.x) is the actively-maintained framework;
  the official MCP SDK's bundled FastMCP is the older 1.0 stub.
- Pydantic-validated tool schemas and the in-process `fastmcp.Client` test
  harness give us cleaner LLM-facing surfaces and trivial unit testing.
- The research doc's `FastMCP.from_fastapi` auto-generation pattern doesn't
  apply (storage is Rust + Connect-RPC, not Python + FastAPI), but
  hand-written intent-shaped tools were always the better LLM ergonomics
  story anyway. The catalog from the research doc is the v1 starting set.
- The OHDC client itself is a stub for v0; the wire-up agent generates
  Python Connect-RPC stubs from `../storage/proto/ohdc/v0/*.proto` and
  drops them in. See [`mcp/STATUS.md`](mcp/STATUS.md) "OHDC client —
  stubbed" for the integration point.

A future TypeScript MCP wrapper (sharing the web `ohdc-client-ts` drop) is
a possible add-on for users who already run a Node toolchain, but is
explicitly **not** the v1 path.

### Web has no on-device deployment support
Browsers can't host an OHD Storage instance reliably (service workers expire,
no persistent FS, OPFS quotas are tight). The web client only supports
**remote primary** deployments. On-device users open Connect mobile or the
desktop CLI.

## What's blocked / TBD per form factor

### Android
- **uniffi binding** — ✅ landed at
  `../storage/crates/ohd-storage-bindings/` and ✅ fully wired into
  Connect Android via `data/StorageRepository.kt` (2026-05-09). All 18+
  call sites — events, grants, pending, cases, audit, emergency config,
  export — now invoke the uniffi Kotlin façade directly (no more mock
  stubs). The `.so` files + Kotlin façade are gitignored (regenerated
  locally per [`android/BUILD.md`](android/BUILD.md) since the
  scaffolding env doesn't include the NDK). On a developer machine with
  the NDK installed:
    1. `cd storage/crates/ohd-storage-bindings && cargo ndk -t arm64-v8a
       -t armeabi-v7a -t x86_64 -o ../../../connect/android/app/src/main/jniLibs
       build --release` — drops per-ABI cdylibs into `app/src/main/jniLibs/`.
    2. `cd storage && cargo run --features cli --bin uniffi-bindgen --
       generate --library target/release/libohd_storage_bindings.so
       --language kotlin --out-dir ../connect/android/app/src/main/java/uniffi`
       — emits `uniffi.ohd_storage` Kotlin façade.
    3. `cd connect/android && ./gradlew :app:assembleDebug`.

  Steps 1+2 are a runtime prerequisite (NDK + `cargo ndk` + `uniffi-bindgen`
  on the developer's PATH); they're not a code gap. The Compose layer
  never imports `uniffi.ohd_storage.*` directly — `StorageRepository` is
  the only entry point.
- **OHDC Kotlin client (remote deployment)** — needs
  `shared/ohdc-clients/kotlin/` from the storage codegen pipeline. The
  on-device deployment doesn't need it; the remote-primary deployment
  does. Currently absent.
- **Real key derivation** — the v0 first-run flow uses a deterministic
  stub key (`"00".repeat(32)`) so the SQLCipher PRAGMA key is well-formed.
  Pickup: BIP39 → seed → HKDF → `K_file` per `spec/encryption.md`,
  unlocked via BiometricPrompt.
- **EncryptedSharedPreferences** — ✅ **wired (mirroring tablet pattern)**
  (2026-05-09). `data/Auth.kt` is now backed by `EncryptedSharedPreferences`
  (`androidx.security:security-crypto:1.1.0-alpha06`) with Keystore-bound
  AES-256-GCM. Falls back to plain SharedPreferences with a logged
  warning if Keystore is unavailable. The contract on `Auth` is
  unchanged — existing call sites need no edits.

- **Self-session OIDC integration** — ✅ **OIDC wired (mirroring tablet
  pattern)** (2026-05-09).
  - `data/OidcManager.kt` — wraps AppAuth-Android (`net.openid:appauth:0.11.1`).
    Same shape as `emergency/tablet`'s OidcManager but signs the user in
    against *their own* OHD Storage instance (which acts as the OAuth AS
    per `spec/docs/design/auth.md`).
  - `data/Auth.signInWithOidc(...)` — persists access + refresh + AppAuth
    state JSON to EncryptedSharedPreferences.
  - `ui/screens/SetupScreen.kt` — replaces the OIDC TODO with a real
    "Connect to a remote storage" form: Storage URL + client_id +
    redirect URI; `OidcManager.startAuthFlow(...)` → Custom Tab →
    `OidcManager.handleAuthResult(...)` → routes to the Main surface.
    On-device path unchanged.
  - Storage URL / client_id / redirect URI come from `BuildConfig`
    defaults set via Gradle `manifestPlaceholders` + `buildConfigField`,
    overridable per build with `-Pohd.connect.oidc.*` flags.
  - Documented in [`android/BUILD.md`](android/BUILD.md) "Self-session
    OIDC".
- **Health Connect bridge service** — needs the device-token issuance flow
  defined in storage; per `spec/health-connect.md`, this is a separate
  Worker that holds an `ohdd_…` token. v0 is empty.
- **APNs / FCM registration** — needs storage's `Notify.RegisterDevice`
  RPC to be live before Connect can register tokens.
- **Emergency dialog above lock screen** — needs platform plumbing
  (`Notification.Builder.setFullScreenIntent`, `FLAG_SHOW_WHEN_LOCKED`,
  ringtone channel registration). Designer-handoff doc in
  `spec/screens-emergency.md`.
- **Grants / Pending tabs** — ✅ **landed (2026-05-09)**.
  - `ui/screens/GrantsScreen.kt` — real impl with active grants list,
    template-driven create flow, share-token bottom sheet, revoke. Backs
    `Grants.{ListGrants, CreateGrant, RevokeGrant}`.
  - `ui/screens/PendingScreen.kt` — real impl with bulk select,
    approve / approve-and-trust / reject. Backs `Pending.{ListPending,
    ApprovePending, RejectPending}`.
- **Cases / Audit / Emergency / Export sub-screens** — ✅ **landed
  (2026-05-09)**, accessible from the Settings tab via in-screen
  navigation tiles (no `androidx.navigation.compose` needed for v0):
  - `ui/screens/CasesScreen.kt` — open + closed sections; tap a case
    expands to audit + handoff chain; "Force close" on active;
    "Issue retrospective grant" sheet on closed (with template picker).
    Auto-granted-via-timeout badge in distinct colour. Backs
    `Cases.{ListCases, GetCase, ForceCloseCase, IssueRetrospectiveGrant}`.
  - `ui/screens/AuditScreen.kt` — `LazyColumn` with sticky day headers;
    op-kind multi-select chips (read / write / grant_mgmt); time-range
    chips (24h / 7d / 30d / all); per-row actor + op + query summary +
    rows_returned + rows_filtered; auto-granted entries get distinct
    background. Backs `Audit.AuditQuery`.
  - `ui/screens/EmergencySettingsScreen.kt` — all 8 sections from
    `connect/spec/screens-emergency.md`: feature toggle, BLE beacon,
    approval timeout slider + default-on-timeout radio, lock-screen
    behaviour, history window 0/3/12/24h, per-channel toggles,
    sensitivity-class toggles, location share, trusted authorities
    (with add/remove), bystander proxy, reset-to-defaults, disable.
    Mirrors `connect/web/src/pages/settings/EmergencySettingsPage.tsx`.
    Persistence today goes through `StorageRepository.{get,set}EmergencyConfig`
    backed by `EncryptedSharedPreferences`; flips to remote
    persistence when `Settings.SetEmergencyConfig` RPC ships.
  - `ui/screens/ExportScreen.kt` — full lossless export button (writes
    a stub `.ohd` placeholder via `StorageRepository.exportAll()`);
    doctor PDF button (renders one-page A4 placeholder via Android's
    `PdfDocument` API client-side, swaps to server-side rendering when
    `Export.GenerateDoctorPdf` ships); migration assistant (TBD,
    documented). Recent-export history list at the bottom.
- **uniffi binding gap for cases/audit/export RPCs** — ✅ closed
  (2026-05-09). The bindings crate exposes the full surface
  (`list_grants`, `create_grant`, `revoke_grant`, `update_grant`,
  `list_pending`, `approve_pending`, `reject_pending`, `list_cases`,
  `get_case`, `force_close_case`, `issue_retrospective_grant`,
  `audit_query`, `get_emergency_config`, `set_emergency_config`,
  `register_signer`, `list_signers`, `revoke_signer`, `export_all` —
  alongside the original `put_event` / `query_events` /
  `issue_self_session_token`). `data/StorageRepository.kt` flipped every
  call site over: each method now calls into the real `uniffi.ohd_storage`
  Kotlin façade. NDK + cargo-ndk + uniffi-bindgen are a runtime
  prerequisite (see [`android/BUILD.md`](android/BUILD.md) Stages 1+2),
  not a code gap. Mock data is gone.

#### Demo path (after BUILD.md flow has run)

What a developer with the NDK installed sees on first launch:
1. App opens to **Setup** screen — minimal layout per `ux-design.md`:
   "OHD Connect" title in light Outfit, subtitle "Your health data, on
   your terms.", red primary button "Use on-device storage", outlined
   secondary button "Connect to a remote storage".
2. Tap "Use on-device storage" → `OhdStorage.create(filesDir/data.db, key)`
   runs the SQLCipher PRAGMA key + migrations + `_meta.user_ulid` stamp;
   `issue_self_session_token()` mints `ohds_…`; the first-run flag is set.
3. App routes to the **Main** surface: bottom-bar with 4 tabs.
   - **Log** — four cards (Glucose / Heart rate / Body temperature /
     Medication taken), tap → ModalBottomSheet with value + notes input
     → `put_event()` round-trips → result chip appears at the bottom.
   - **Dashboard** — pulls `query_events(limit=50)` and renders a flat
     list of monospace channel-key=value rows with relative timestamps.
   - **Grants** — placeholder copy ("Grants management coming in v0.x").
   - **Settings** — storage path, user ULID, truncated bearer, format /
     protocol versions, app build, deployment-mode placeholder.
4. Subsequent launches skip Setup, reopen the existing storage with the
   stub key, and land on the Log tab.

### iOS
- **OHDC Swift client** — needs `shared/ohdc-clients/swift/`.
- **uniffi binding** — same blocker as Android.
- **HealthKit bridge** — corresponds to Android's Health Connect bridge.
  Needs HealthKit entitlement + permission plumbing; spec doc TBD (the
  Connect-side `health-connect.md` is Android-specific; HealthKit is parallel
  work).
- **APNs critical-alert capability** — Apple entitlement requires App Store
  review approval; tracked as a release blocker, not a code blocker.
- **The emergency dialog** — `UNNotificationContent` with
  `interruption-level: critical`, registered critical-alert sound, locked-
  screen full-screen-intent equivalent.
- **`Package.swift` is a CLI target stub** — turning it into a proper iOS app
  needs an Xcode project + asset catalog + Info.plist. SwiftPM alone can't
  produce an iOS app bundle. The implementation phase will likely add
  `ios/OhdConnect.xcodeproj/` (or migrate to Tuist / XcodeGen) and keep
  `Package.swift` as the SwiftPM entry for shared modules + tests.

### Web
- **OHDC TS client** — needs `shared/ohdc-clients/typescript/`.
- **OAuth code flow + PKCE** — needs `@openid/client` or `oauth4webapi`
  pinned; the v0 has no auth.
- **Token storage** — IndexedDB module + origin-isolation guards; not
  sketched yet.
- **Web Push registration** — needs VAPID keys from operator config + a
  service worker; v0 is single-page React.
- **`BarcodeDetector` API** — Chrome / Edge support is good; Safari support
  via `Vision` is iOS-only; Firefox lacks it. Document fallback (manual
  barcode entry) at implementation time.
- **`require_approval_per_query` page (F)** — ✅ landed 2026-05-09. UI
  against in-memory mock until storage exposes
  `OhdcService.{List,Approve,Reject}PendingQuery` in the proto;
  `pendingQueriesIsMock()` probe handles the swap.
- **Light theme toggle (G)** — ✅ landed 2026-05-09. Settings →
  Appearance.
- **Family / delegate access UI** — ✅ **landed (2026-05-09)**.
  - `pages/settings/DelegatesSettingsPage.tsx` — new sub-page under
    Settings → Delegates. Issue-delegate modal: label + paste-token OIDC
    identity blob (v0 limitation, documented inline) + per-channel
    read/write scope toggles + sensitivity-class deny toggles
    (defaults deny mental_health/substance_use/sexual_health/reproductive)
    + expiry chooser (1 month / 3 months / 1 year / custom days).
  - Active delegates rendered with distinct visual treatment: yellow
    "delegate" badge, left border accent, separated from regular grants
    list. Revoke calls `OhdcService.RevokeGrant`; existing wiring.
  - Issue calls `OhdcService.CreateGrant` with `granteeKind="delegate"`
    as the v0 stand-in. Storage's dedicated `IssueDelegateGrant` proto
    extension (with the `delegate_for_user_ulid` field) lands in v0.x
    per `storage/STATUS.md`; swapping is mechanical at that point.
  - 16 vitest tests pass (was 14, +2 for delegates: empty-state mount +
    populated-state with badge assertion).

### CLI
- **OHDC Rust client** — ✅ landed. The CLI codegens directly from
  `../storage/proto/ohdc/v0/` via `connectrpc-build` in `cli/build.rs`. The
  separate `shared/ohdc-clients/rust/` crate is no longer required for the
  CLI; remains a future consolidation target for cross-form-factor reuse.
- **Five wired RPCs** — ✅ `Health`, `WhoAmI`, `PutEvents`, `QueryEvents`,
  `GetEventByUlid` round-trip cleanly; matched against the same five RPCs
  the storage server has implemented (`../storage/STATUS.md` "OHDC
  server").
- **Device flow login** — ⏳ still blocked on the OHDC OAuth surface. v1
  workaround: `ohd-storage-server issue-self-token` issues the token
  out-of-band; `ohd-connect login --token <ohds_…>` writes it to TOML.
- **Credential storage** — ✅ `~/.config/ohd-connect/credentials.toml` mode
  0600. Keyring integration deferred.
- **Subcommand tree** — partial. `version`, `login`, `whoami`, `health`,
  `log {glucose|heart-rate|temperature|medication-taken|symptom}`, `query
  <kind> [--last-*|--from|--to]` work. `grant`, `pending`, `case`, `audit`,
  `emergency`, `export`, `config` await the corresponding storage RPCs.

### MCP
- **Stack** — Python + FastMCP 3.x; pyproject + uv; FastMCP `Client` test
  harness for in-process smoke tests. See [`mcp/STATUS.md`](mcp/STATUS.md).
- **OHDC Python client** — stubbed. Wire-up agent generates Python
  Connect-RPC stubs from `../storage/proto/ohdc/v0/*.proto` and replaces
  the bodies in `mcp/src/ohd_connect_mcp/ohdc_client.py`.
- **Tool catalog** — all 27 tools from [`SPEC.md`](SPEC.md) "Connect MCP —
  tool list" are registered with real pydantic-validated input schemas and
  real docstrings; the OHDC call layer is the only thing stubbed.
- **OAuth proxy** — FastMCP's `OAuthProxy` against the OHD Storage AS
  metadata, for the remote Streamable HTTP transport. Not wired yet.
- **Time parsing** — `_resolve_ts` accepts ISO 8601 only for now. Add
  `dateparser` for natural-phrase inputs ("yesterday", "30m ago") per
  `spec/docs/research/mcp-servers.md` "Handling time input".
- **Transport** — both stdio and Streamable HTTP supported via
  `OHD_MCP_TRANSPORT={stdio,http}`; OAuth proxy required for HTTP.

### Shared
- **OHDC client codegen** — the entire `shared/ohdc-clients/` tree is empty.
  This is **the** cross-cutting blocker. The storage component owns the
  `.proto` schemas at `../storage/proto/ohdc/v0/` and the Buf pipeline that
  generates the per-language drops. Connect implementation cannot proceed
  past hello-world on any form factor until the first codegen lands.

## Open design items inherited from the canonical spec

Per `../spec/docs/components/connect.md` "Open design items":

- Family / delegate access (one user acting on behalf of another) — `kind='delegate'` grants; UX TBD.
- Source signing scheme (optional integration-level signing for high-trust
  writes) — UX surface TBD.
- Approval-policy templates beyond the v1 set — community contributions
  expected.

## Recommended order for implementation phase

1. Storage component delivers OHDC `.proto` v1 + Buf pipeline + first codegen
   drop into `shared/ohdc-clients/`.
2. CLI implementation — straightest line: Rust + clap + `ohdc-client-rust` +
   device-flow login. Validates the auth flow + the OHDC client surface end-
   to-end without UI complexity. Doubles as conformance harness driver.
3. Web — second easiest: same TS client, browser-native fetch transport,
   familiar OAuth flow. Validates the Connect-Web HTTP/3 transport.
4. Android — Compose + uniffi + Cronet HTTP/3 + Health Connect bridge.
   Heaviest single platform; depends on uniffi from storage.
5. iOS — parallel to Android once uniffi Swift bindings ship.
6. MCP — last, because every other form factor smoke-tests the same client
   surface; the MCP just wraps it for LLM consumption.

The emergency dialog (above-lock full-screen) is independent of the OHDC
client and can be prototyped in parallel.
