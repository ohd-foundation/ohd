# OHD Storage

> The data layer of the [Open Health Data](../spec/) protocol. Owns
> persistence, the on-disk format, the channel registry, grants, audit, sync,
> and encryption-at-rest. Exposes its functionality only through the **OHDC**
> protocol.

OHD Storage is one of the five OHD components (Storage, Connect, Care,
Emergency, Relay). It's the source of truth for a person's health data — a
focused, single-purpose typed store, **not** a generic database.

## Where to start

- [`spec/README.md`](spec/README.md) — index of the design docs storage
  owns (file format, OHDC wire spec, sync, encryption, privacy, conformance).
- [`SPEC.md`](SPEC.md) — implementation-ready summary of what storage has to
  build, distilled from the design docs.
- [`STATUS.md`](STATUS.md) — current scaffolding state and a hand-off list
  for the implementation phase.
- [`../spec/`](../spec/) — the global, cross-component spec. The canonical
  source for everything in `spec/` here.

## Component summary

| Property | Value |
|---|---|
| Language | Rust (single core) |
| Engine | SQLite + SQLCipher 4 (WAL) |
| Wire protocol | **OHDC** — Connect-RPC over HTTP/3 with Protobuf schemas |
| Auth profiles | self-session, grant token, device token |
| Distribution | Linux server binary, Android `.aar` (uniffi), iOS `.xcframework` (uniffi), Python wheel (PyO3) |
| File layout | One file per user (`data.db`) + a per-user `blobs/` sidecar dir |
| Concurrency | Single writer + many readers per file; cross-process sharing not supported |
| Sync model | Bidirectional event-log replay between cache + primary, ULID-based dedup, per-peer rowid watermarks |

## Directory layout

```
storage/
├── README.md                         # this file
├── SPEC.md                           # implementation-ready spec
├── STATUS.md                         # what's scaffolded / stubbed / next
├── Cargo.toml                        # workspace manifest
├── rust-toolchain.toml               # Rust toolchain pin (rustup; ignored without)
├── buf.yaml                          # buf module + lint config
├── buf.gen.yaml                      # buf codegen targets (Rust + future K/S/T/Py)
├── .gitignore
├── crates/
│   ├── ohd-storage-core/             # library: format, registry, grants, audit, sync, ...
│   ├── ohd-storage-server/           # binary: HTTP/3 OHDC server (Linux)
│   └── ohd-storage-bindings/         # uniffi (Kotlin/Swift) + PyO3 (Python) bindings
├── proto/
│   └── ohdc/v0/                      # OHDC v0 .proto files
│       ├── ohdc.proto                # OhdcService — main consumer surface
│       ├── auth.proto                # AuthService — sessions, identities, device tokens
│       ├── sync.proto                # SyncService — cache ↔ primary
│       └── relay.proto               # RelayService — storage ↔ relay
├── migrations/                       # SQL migrations land here
└── spec/                             # design docs storage owns (copies of ../spec/docs/design/*)
```

## Build and run

```bash
# Build everything
cargo build

# Run the server's health check
cargo run -p ohd-storage-server -- health

# Init a database, issue a self-session token, then serve:
cargo run -p ohd-storage-server -- init --db /tmp/data.db
cargo run -p ohd-storage-server -- issue-self-token --db /tmp/data.db
cargo run -p ohd-storage-server -- serve --db /tmp/data.db --listen 127.0.0.1:8443

# Optional HTTP/3 listener (in-binary; see STATUS.md):
cargo run -p ohd-storage-server -- serve --db /tmp/data.db \
    --listen 127.0.0.1:8443 --http3-listen 127.0.0.1:8443

# Tests
cargo test --workspace
```

The full implementation surface is wired end-to-end (registry, auth resolution,
write-with-approval, OHDC server). See [`STATUS.md`](STATUS.md) for what's
landed, what's stubbed, and the conformance corpus state.

## Deploy

- Docker: [`deploy/README.md`](deploy/README.md) — single-service compose with init / token-issuance flow.
- Native packages (.deb / .rpm / Arch): [`../PACKAGING.md`](../PACKAGING.md). The systemd unit at `../packaging/systemd/ohd-storage.service` runs the binary as the dedicated `ohd-storage` system user.

## How storage fits the rest of OHD

```
┌─────────────────────────────────────────────────────────────────┐
│                        OHD STORAGE  (this dir)                  │
│   on-disk format · permissions · audit · grants · sync          │
│   ┌──── OHDC over HTTP/3 ──────────────────────────────────┐    │
│   │  self-session  │  grant token  │  device token         │    │
│   └────────▲──────────────▲──────────────▲─────────────────┘    │
└────────────┼──────────────┼──────────────┼─────────────────────┘
             │              │              │
       OHD Connect      OHD Care       Sensors / Labs / EHRs
       (../connect)    (../care)        (third-party device tokens)
                       OHD Emergency
                       (../emergency)

       Plus OHD Relay (../relay) bridging remote consumers ↔
       storage instances behind NAT.
```

Cross-component contracts:

- **OHDC `.proto` files** in `proto/ohdc/v0/` are imported by Connect, Care,
  and Emergency for their typed clients. They're the only thing those
  components depend on from storage. (The Rust core in `crates/` is linked
  into Connect's mobile builds via `ohd-storage-bindings`, but that's an
  implementation detail of "on-device deployment", not an external contract.)
- **OHD Relay** (`../relay`) consumes the `RelayService` definition in
  `proto/ohdc/v0/relay.proto` and forwards opaque traffic; relay sees
  ciphertext only.

## Status & contributing

- [`STATUS.md`](STATUS.md) tracks landed work, stubs, and the implementation hand-off.
- Schema changes start in [`proto/ohdc/v0/`](proto/ohdc/v0/) and ripple out through every consumer's codegen pipeline. Note them in `STATUS.md`.

## License

Dual-licensed under your choice of:

- **Apache License, Version 2.0** ([`../spec/LICENSE-APACHE`](../spec/LICENSE-APACHE))
- **MIT License** ([`../spec/LICENSE-MIT`](../spec/LICENSE-MIT))

See [`../spec/LICENSE`](../spec/LICENSE) for the dual-license arrangement.
