# OHD Connect

> The personal-side reference application of OHD. Android, iOS, web, CLI, and
> MCP. Speaks OHDC under self-session auth.

This directory holds all five form factors of OHD Connect plus the shared
material (spec snapshots, codegen drop zone, status). The component spec is
[`SPEC.md`](SPEC.md); the canonical project-level spec is in
[`../spec/`](../spec/).

## What Connect is

The user's tool for **everything they do with their own OHD data**:

- **Logging** — Health Connect / HealthKit bridge (Android/iOS), barcode food
  via OpenFoodFacts, manual measurements, medications, voice / free-text
  symptoms.
- **Personal dashboard** — recent activity, charts, timelines, saved views,
  cross-channel correlation.
- **Grant management** — issue grants to doctors, family, researchers; inspect
  what each grantee has queried; revoke instantly.
- **Pending review** — approve / reject grant-submitted writes that landed in
  the approval queue.
- **Audit inspection** — see exactly what every grant has done.
- **Cases** — list of ongoing cases (e.g. an active EMS event), force-close,
  retrospective grant issuance.
- **Emergency settings** — break-glass feature toggle, BLE beacon, approval
  timeout + default action, history window, sensitivity-class toggles, trusted
  authority roots, bystander-proxy role.
- **Export / portability** — full lossless OHD export, doctor-PDF, migration
  between deployment modes.

All five surfaces consume the **same OHDC schema** (codegen'd from
`../storage/proto/ohdc/v0/*.proto` per language) and run under the same
self-session auth profile.

## Form factors (one subdir each)

| Form factor | Subdir | Stack | OHDC client |
|---|---|---|---|
| Android | [`android/`](android/) | Kotlin + Jetpack Compose; links Rust core via uniffi for on-device deployments; OkHttp + Connect-Protocol JSON for remote primary | hand-rolled per [`android/BUILD.md`](android/BUILD.md) |
| iOS | [`ios/`](ios/) | Swift + SwiftUI; same Rust core via uniffi; URLSession HTTP/2 for remote primary | TBD (SwiftPM scaffold) |
| Web | [`web/`](web/) | Vite + React + TypeScript; remote primary only | `@connectrpc/connect-web`, codegen via `pnpm gen` |
| CLI | [`cli/`](cli/) | Rust + clap; ships as `ohd-connect` binary | `connectrpc` + `buffa`, codegen at build.rs time |
| MCP | [`mcp/`](mcp/) | Python + FastMCP; ships as `ohd-connect-mcp` | `ohd-shared` (`OhdcTransport`) |

The CLI is **Rust** for consistency with the OHD Storage core. The MCP server moved to **Python + FastMCP** per [`spec/mcp-servers.md`](spec/mcp-servers.md); it shares helpers with `care/mcp` and `emergency/mcp` via the [`packages/python/ohd-shared`](../packages/python/ohd-shared) workspace package.

## Shared material

| Path | Purpose |
|---|---|
| [`SPEC.md`](SPEC.md) | Implementation-ready Connect spec — distilled from `../spec/docs/components/connect.md` and pinned spec snapshots. |
| [`STATUS.md`](STATUS.md) | Handoff notes for the implementation phase: what's scaffolded, what's blocked, what's TBD per form factor. |
| [`spec/`](spec/) | Verbatim copies of spec files Connect needs (`auth.md`, `notifications.md`, `mcp-servers.md`, `health-connect.md`, `openfoodfacts.md`, `barcode-scanning.md`, `screens-emergency.md`). |
| [`shared/`](shared/) | Cross-form-factor material — most importantly the codegen drop zone for the OHDC clients. See [`shared/ohdc-client-stub.md`](shared/ohdc-client-stub.md). |

## Build / run per form factor

Each subdir holds the focused recipe.

### Android — `android/`

Two-stage build (Rust core via uniffi + Gradle assemble). Full recipe in [`android/BUILD.md`](android/BUILD.md):

```bash
cd android
./gradlew :app:assembleDebug
./gradlew :app:installDebug
```

Requires Android Studio Hedgehog+, JDK 17, NDK r26+, `cargo-ndk`. Min SDK 29.

### iOS — `ios/`

```bash
cd ios
swift build
open Package.swift
```

Requires Xcode 16+ and iOS 17+ deployment target. SwiftPM scaffold; an Xcode project lands alongside the OHDC Swift client.

### Web — `web/`

```bash
cd web
pnpm install            # or `pnpm install` from repo root (workspace)
pnpm gen                # codegen TS OHDC client from ../../storage/proto/
pnpm dev                # :5174
pnpm build              # production → dist/
pnpm test               # vitest
```

Requires Node 20+ and pnpm 9+. The web SPAs share helpers via `@ohd/shared-web` ([`../packages/web/ohd-shared-web`](../packages/web/ohd-shared-web)) — installs come from the repo-root pnpm workspace.

### CLI — `cli/`

```bash
cd cli
cargo build                                # codegens OHDC client at build time
cargo install --path .                     # installs `ohd-connect`
cargo test
```

Requires Rust 1.88+.

### MCP — `mcp/`

```bash
cd mcp
uv sync                  # creates .venv, installs deps + ohd-shared
uv run python -m ohd_connect_mcp
uv run pytest
```

Requires Python 3.11+ and `uv`. The MCP server depends on `ohd-shared` via a path-scoped uv source.

## OHDC client codegen

Storage owns the `.proto` schemas at [`../storage/proto/ohdc/v0/*.proto`](../storage/proto/ohdc/v0/). Each form factor consumes them differently:

- `cli/` — `connectrpc-build` runs at `cargo build` time.
- `web/` — `pnpm gen` runs the TS codegen via `buf.gen.yaml`.
- `mcp/` — re-uses the proto stubs bundled in `ohd-shared`.
- `android/`, `ios/` — hand-rolled clients today (see [`android/BUILD.md`](android/BUILD.md) "OHDC client" for the rationale).

The legacy [`shared/ohdc-client-stub.md`](shared/ohdc-client-stub.md) describes a per-language vendored-client plan that was superseded by codegen-at-build-time inside each form factor. See it for historical context only.

## Deploy

Connect is a personal app — there's no project-wide Connect deployment. Per form factor:

- CLI / MCP: distributed as binaries / wheels — see [`../PACKAGING.md`](../PACKAGING.md).
- Web: typically deployed alongside a storage instance; serve `dist/` from any static host (or fold into the storage operator's Caddy).
- Android / iOS: store distribution.

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
