# OHD Relay

> The bridging service that lets remote OHDC consumers reach OHD Storage instances behind NAT — phones, home servers, anything without a public IP. Forwards opaque ciphertext between paired endpoints; **does not decrypt**, does not authenticate request payloads.

OHD Relay is one of the five components of [Open Health Data (OHD)](../spec/README.md). The other four — Storage, Connect, Care, Emergency — live in sibling top-level directories.

## What this crate is

A single Rust binary (`ohd-relay`) plus deployment recipes (Docker Compose + Caddy). Fronted by Caddy for outer TLS; uses HTTP/3 (QUIC) for the tunnel and consumer-attach paths. The reference implementation targets [`quinn`](https://github.com/quinn-rs/quinn) for HTTP/3.

The relay sees ciphertext only. TLS terminates at the consumer and at storage — end-to-end through the tunnel — using the storage's identity-key-signed self-signed cert pinned via the grant artifact. See [`spec/relay-protocol.md`](./spec/relay-protocol.md) for the wire spec.

## Two routing patterns

- **Pairing-mediated** (NFC / QR, ephemeral). In-person handshake; trust anchor is physical proximity.
- **Grant-mediated** (durable rendezvous URL). Storage maintains a long-lived tunnel registered against an opaque rendezvous ID; every grant the user issues against that storage embeds the same URL.

Both patterns ride the same tunnel framing and multiplex many consumer sessions onto one storage tunnel.

## Optional emergency-authority mode

A relay deployment can opt into `--features authority` to additionally hold an authority cert (Fulcio-issued, 24h TTL) and sign break-glass `EmergencyAccessRequest` payloads bound for patient phones. Most relays stay as plain packet forwarders. See [`spec/emergency-trust.md`](./spec/emergency-trust.md).

## Layout

```
relay/
├── README.md                  # this file
├── SPEC.md                    # implementation-ready spec for this crate
├── STATUS.md                  # handoff notes for the implementation agent
├── Cargo.toml                 # bin crate
├── rust-toolchain.toml        # pinned stable toolchain
├── src/
│   ├── main.rs                # clap CLI: serve | health | version
│   ├── server.rs              # HTTP server skeleton + relay-protocol RPC stubs
│   ├── state.rs               # RegistrationTable / SessionTable / PairingTable (in-memory stubs)
│   └── auth_mode.rs           # emergency-authority mode (feature-gated)
├── spec/                      # vendored snapshots of canonical design docs
│   ├── README.md
│   ├── relay-protocol.md
│   ├── emergency-trust.md
│   └── notifications.md
└── deploy/
    ├── docker-compose.yml
    ├── Caddyfile
    └── relay.example.toml
```

## Build & run

```bash
cd relay
cargo build
cargo run -- health         # → "OHD Relay v0 — health: ok"
cargo run -- version        # prints crate version
cargo run -- serve          # starts the HTTP skeleton (stub handlers)
```

With emergency-authority mode:

```bash
cargo build --features authority
cargo run --features authority -- serve --config deploy/relay.example.toml
```

## Deployment

The Docker Compose + Caddy reference stack lives in [`deploy/`](deploy/) — see [`deploy/README.md`](deploy/README.md) for the full recipe (ports, volumes, push-provider secrets, emergency-authority mode).

For native packages (.deb / .rpm / Arch), see [`../PACKAGING.md`](../PACKAGING.md).

Caddy fronts the relay on `:443` and terminates **outer** TLS (the cert that authenticates `relay.example.com` to consumers). The **inner** TLS — between consumer and storage, end-to-end through the tunnel — is invisible to Caddy and to the relay; it terminates at the storage's self-signed cert pinned via the grant artifact.

See [`spec/relay-protocol.md`](./spec/relay-protocol.md) "Storage registration" and "Frame format" for the wire contract the binary implements.

## What the relay does NOT do

- Does not decrypt OHDC traffic (TLS-through-tunnel).
- Does not store grants, audit, or tokens.
- Does not authenticate the OHDC request payload (storage's job, behind the relayed TLS).
- Does not replicate / persist health data — sessions are ephemeral, logs are operational telemetry only.

A subpoena recovers traffic patterns and the five-field per-user state — never content. This minimalism is what makes the relay's privacy property load-bearing.

## Test

```bash
cargo test
```

## Status

The relay has working HTTP/2 + HTTP/3 listeners, registration persistence, frame routing, and the `/v1/emergency/initiate` + `/v1/emergency/handoff` JSON endpoints used by the Emergency tablet. Authority-mode signing (`--features authority`, Fulcio refresh, signed `EmergencyAccessRequest`) is partially landed; consult [`STATUS.md`](./STATUS.md) for the per-piece state.

## License

Dual-licensed under Apache-2.0 OR MIT, matching the broader OHD project. See [`../spec/LICENSE`](../spec/LICENSE).
