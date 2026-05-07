# Open Health Data (OHD)

> A decentralized, user-owned protocol for personal health data.

**OHD** is an open-source protocol and reference implementation for storing, collecting, and sharing personal health data. It fills the gaps left by existing infrastructure (Google Health Connect, Apple HealthKit, hospital EHRs) by putting **you** in full control of **your** data.

## The Four Components

| Component | Role |
|---|---|
| **OHD Storage** | The data layer. Owns the format, persistence, permissions, audit, grants, and sync. Runs on the phone, on our SaaS, on a third-party SaaS, or self-hosted. Exposes a single external protocol — OHDC. |
| **OHD Connect** | The personal-side reference application — Android, iOS, web, CLI, MCP. Speaks OHDC under self-session auth. The user's tool for logging, viewing personal data, managing grants, reviewing pending submissions, exporting. |
| **OHD Care** | The clinical-side reference application — a real, lightweight, EHR-shaped consumer of OHDC. Multi-patient via grant tokens. Designed for OHD-data-centric clinical workflows; not competing with enterprise EHRs. |
| **OHD Relay** | The bridge that lets remote OHDC consumers reach storage instances behind NAT (phones, home servers without public IP). Forwards opaque packets after either an in-person pairing handshake (NFC/QR) or grant-mediated registration. Doesn't decrypt. |

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
- [`docs/components/relay.md`](docs/components/relay.md) — OHD Relay (bridging service)
- [`docs/glossary.md`](docs/glossary.md) — every term defined once

### Design
- [`docs/design/data-model.md`](docs/design/data-model.md) — Conceptual event vocabulary
- [`docs/design/storage-format.md`](docs/design/storage-format.md) — On-disk format, schema, sample blocks, channel registry, grants, sync
- [`docs/design/privacy-access.md`](docs/design/privacy-access.md) — Identity, permissions, audit trails
- [`docs/design/deployment.md`](docs/design/deployment.md) — Docker Compose, Hetzner, Caddy

### Research (external systems we integrate with)
- [`docs/research/health-connect.md`](docs/research/health-connect.md) — Android Health Connect integration
- [`docs/research/openfoodfacts.md`](docs/research/openfoodfacts.md) — OpenFoodFacts API integration
- [`docs/research/barcode-scanning.md`](docs/research/barcode-scanning.md) — Barcode scanning on Android/iOS
- [`docs/research/mcp-servers.md`](docs/research/mcp-servers.md) — MCP server design & tool definitions

## Tech Stack

- **Storage engine**: Rust core, SQLite + SQLCipher under the hood. Same code on Linux servers and on mobile via `uniffi` bindings (Kotlin for Android, Swift for iOS) and `PyO3` (Python for tooling).
- **Network transport**: HTTP/3 over QUIC (HTTP/2 fallback), fronted by Caddy with automatic HTTPS.
- **Mobile**: native Android (Kotlin) and iOS (Swift), each linking the Rust core directly so the same storage engine runs on-device.
- **OHD Relay**: Rust binary, also fronted by Caddy.
- **AI integration**: two MCP servers — Connect MCP (write + read tools for the user's personal LLM) and Care MCP (multi-patient, by-user contextualized read + write-with-approval tools for clinical LLM use).
- **Identity**: OIDC delegation (no PII stored in the OHD protocol itself).

## License

TBD — something open-source with strong provisions for user data ownership and portability. See [`docs/02-principles.md`](docs/02-principles.md) for the licensing philosophy.
