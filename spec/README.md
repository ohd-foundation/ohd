# Open Health Data (OHD)

> A decentralized, user-owned protocol for personal health data.

**OHD** is an open-source protocol and reference implementation for storing, collecting, and sharing personal health data. It fills the gaps left by existing infrastructure (Google Health Connect, Apple HealthKit, hospital EHRs) by putting **you** in full control of **your** data.

## The Five Components

| Component | Role |
|---|---|
| **OHD Storage** | The data layer. Owns the format, persistence, permissions, audit, grants, cases, and sync. Runs on the phone, on our SaaS, on a third-party SaaS, or self-hosted. Exposes a single external protocol — OHDC. |
| **OHD Connect** | The personal-side reference application — Android, iOS, web, CLI, MCP. Speaks OHDC under self-session auth. The user's tool for logging, viewing personal data, managing grants, reviewing pending submissions, configuring emergency settings, exporting. |
| **OHD Care** | The clinical-side reference application — a real, lightweight, EHR-shaped consumer of OHDC. Multi-patient via grant tokens. Case-aware. Designed for OHD-data-centric clinical workflows; not competing with enterprise EHRs. |
| **OHD Emergency** | The emergency-services reference application — paramedic tablet, dispatch console, emergency MCP. Speaks OHDC via break-glass-issued grants under cert-signed emergency-authority requests. Mobile-first, time-critical, case-bound. |
| **OHD Relay** | The bridge that lets remote OHDC consumers reach storage instances behind NAT (phones, home servers without public IP). Forwards opaque packets after either an in-person pairing handshake (NFC/QR) or grant-mediated registration. Has an **emergency-authority mode** for cert-signed break-glass. Doesn't decrypt. |

The single protocol — **OHDC** — has three auth profiles: self-session (the user themselves), grant token (third-party with structured scope, optional approval queue), device token (write-only, attributed by device). Mixed-scope grants are natural in this model: a doctor with read access to vitals plus write-with-approval on lab results is one grant, not three protocols stitched together.

## Core Principles

1. **Your data belongs to you.** Not to us, not to your doctor, not to your hospital.
2. **Portable by design.** Your data moves with you between providers, losslessly.
3. **Privacy by default.** No identity stored. You decide who sees what.
4. **Auditable.** Every access is logged. You always know who saw what, when.
5. **Open.** Anyone can run an OHD Storage instance. Anyone can build an OHDC consumer. Anyone can run an OHD Relay.
6. **Comprehensive.** Biometrics, meals, medications, symptoms, doctor notes, hospital records — all in one protocol.

## Document Index

### Start here
- [`docs/00-vision.md`](docs/00-vision.md) — What we're building and why
- [`docs/01-architecture.md`](docs/01-architecture.md) — The four-component system explained
- [`docs/02-principles.md`](docs/02-principles.md) — Core principles and licensing philosophy
- [`docs/deployment-modes.md`](docs/deployment-modes.md) — On-device, self-hosted, custom provider, OHD Cloud — when to pick each

### Components
- [`docs/components/storage.md`](docs/components/storage.md) — OHD Storage (the core product)
- [`docs/components/connect.md`](docs/components/connect.md) — OHDC protocol + OHD Connect personal app
- [`docs/components/care.md`](docs/components/care.md) — OHD Care (reference clinical app)
- [`docs/components/emergency.md`](docs/components/emergency.md) — OHD Emergency (reference emergency-services app)
- [`docs/components/relay.md`](docs/components/relay.md) — OHD Relay (bridging service + emergency authority)
- [`docs/glossary.md`](docs/glossary.md) — every term defined once

### Design files
- [`design/screens-emergency.md`](design/screens-emergency.md) — designer-handoff doc for the emergency / break-glass screens

### Design
- [`docs/design/data-model.md`](docs/design/data-model.md) — Conceptual event vocabulary
- [`docs/design/storage-format.md`](docs/design/storage-format.md) — On-disk format, schema, sample blocks, channel registry, grants, sync
- [`docs/design/ohdc-protocol.md`](docs/design/ohdc-protocol.md) — OHDC v1 wire spec: services, messages, error model, pagination, idempotency, filter language, streaming, `.proto` source
- [`docs/design/auth.md`](docs/design/auth.md) — OIDC self-session, OAuth flows, token formats, account-join modes, on-device sub-modes
- [`docs/design/care-auth.md`](docs/design/care-auth.md) — OHD Care operator auth, per-patient grant-token vault, two-sided audit, patient-curated case workflow
- [`docs/design/encryption.md`](docs/design/encryption.md) — Per-user file encryption, key hierarchy, recovery (BIP39), multi-device pairing, key rotation, export encryption
- [`docs/design/emergency-trust.md`](docs/design/emergency-trust.md) — Authority cert chain (Fulcio + X.509 + Rekor), short-lived (24h) org certs, signed emergency-access request, patient-phone verification
- [`docs/design/notifications.md`](docs/design/notifications.md) — Push delivery (FCM/APNs/email), no-PHI payload contract, quiet hours, system-DB tables
- [`docs/design/relay-protocol.md`](docs/design/relay-protocol.md) — Relay tunnel wire spec: TLS-through-tunnel cert pin, frame format, session multiplexing, registration & lifecycle
- [`docs/design/sync-protocol.md`](docs/design/sync-protocol.md) — Cache↔primary sync wire spec: SyncService RPCs, frame stream, watermarks, attachment payload sync, grant lifecycle out-of-band
- [`docs/design/conformance.md`](docs/design/conformance.md) — OHDC v1 conformance corpus: structure, categories, runner, what claiming "v1" means
- [`docs/design/privacy-access.md`](docs/design/privacy-access.md) — Identity, permissions, audit trails
- [`docs/design/deployment.md`](docs/design/deployment.md) — Docker Compose, Hetzner, Caddy

### Research (external systems we integrate with)
- [`docs/research/health-connect.md`](docs/research/health-connect.md) — Android Health Connect integration
- [`docs/research/openfoodfacts.md`](docs/research/openfoodfacts.md) — OpenFoodFacts API integration
- [`docs/research/barcode-scanning.md`](docs/research/barcode-scanning.md) — Barcode scanning on Android/iOS
- [`docs/research/mcp-servers.md`](docs/research/mcp-servers.md) — MCP server design & tool definitions

### Future implementations (deferred, post-v1)

Design-space sketches kept to verify the v1 architecture doesn't block future work. Not contracted v1 deliverables.

- [`docs/future-implementations/device-pairing.md`](docs/future-implementations/device-pairing.md) — Sensor / lab / vendor / bridge integration models (companion-app IPC, vendor-OAuth, in-app, direct-device, BLE)

## Tech Stack

- **Storage engine**: Rust core, SQLite + SQLCipher under the hood. Same code on Linux servers and on mobile via `uniffi` bindings (Kotlin for Android, Swift for iOS) and `PyO3` (Python for tooling).
- **Wire protocol**: OHDC, the project's only external API. **Connect-RPC over HTTP/3** (HTTP/2 fallback), schemas defined in **Protobuf**. Codegen via Buf CLI produces typed clients in Rust, Kotlin, Swift, TypeScript, and Python. Caddy fronts deployments with automatic HTTPS. Binary Protobuf encoding by default; JSON encoding available per-request for debugging. Wire-compatible with gRPC for integrators that already speak it.
- **Mobile**: native Android (Kotlin) and iOS (Swift), each linking the Rust core directly so the same storage engine runs on-device.
- **OHD Relay**: Rust binary, also fronted by Caddy. Forwards Connect-RPC traffic opaquely (TLS terminates at storage and consumer).
- **AI integration**: two MCP servers — Connect MCP (write + read tools for the user's personal LLM) and Care MCP (multi-patient, by-user contextualized read + write-with-approval tools for clinical LLM use).
- **Identity**: OIDC delegation (no PII stored in the OHD protocol itself).

## License

**Dual-licensed** under your choice of:

- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- **MIT License** ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

See [LICENSE](LICENSE) for details on the dual-license arrangement, the `Apache-2.0 OR MIT` Rust convention this follows, and trademark notes. The non-binding "spirit of the project" asks (data ownership, portability, contribution norms) live in [SPIRIT.md](SPIRIT.md) and [`docs/02-principles.md`](docs/02-principles.md).
