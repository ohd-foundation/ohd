# OHD CORD

The OHD conversational agent. A deployable web service that lets a user —
or an authorized clinician — talk to a health-data store in natural
language. OHD Cloud runs it at `cord.ohd.dev`; clinics and ambulance
services self-host it the same way they would OHD Storage.

- **Spec:** [`SPEC.md`](SPEC.md)
- **Data link** (how CORD reaches a user's storage): [`spec/data-link.md`](spec/data-link.md)

## Layout

```
cord/
  crates/cord-server/   Rust/axum backend — auth, sources, models, chats
  cord.example.toml     configuration template
  deploy/Dockerfile     container image
```

`cord-agent` (the agent loop + model providers) and `cord-web` (the SPA)
arrive in Phase 2.

## Running locally

```sh
cd cord
cargo run --bin cord-server            # dev config: 127.0.0.1:8446, login disabled
cargo run --bin cord-server -- --config cord.toml
```

Copy `cord.example.toml` to `cord.toml` and set the referenced environment
variables (session secret, data key, OIDC + model provider keys) for a
real deployment.

## Status

Phase 1 — service skeleton: OIDC login, JWT sessions, the encrypted
data-source registry, model/BYO-key management, chat scaffolding,
`/healthz`. Message streaming and the relay data plane follow in later
phases (see `SPEC.md` "Implementation roadmap").
