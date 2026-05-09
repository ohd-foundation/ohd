# OHD Connect — CLI

Rust + clap. Single binary `ohd-connect`. Terminal interface to OHDC under
self-session auth, for power users, scripts, and automation.

## Status

The CLI codegens the OHDC Rust client at build time (from `../../storage/proto/ohdc/v0/ohdc.proto`) and speaks Connect-RPC (HTTP/2 h2c or HTTP/3) to a running `ohd-storage-server`. Implemented subcommands: `login`, `whoami`, `health`, `log {glucose|heart-rate|temperature|medication-taken|symptom}`, `query <kind> [--last-day|--last-week|--last-month|--from|--to]`, `version`. The end-to-end demo at [`../../care/demo/run.sh`](../../care/demo/run.sh) drives both the storage server and the CLI and asserts a put-then-query round-trip.

The full subcommand surface (`grant`, `pending`, `case`, `audit`, `emergency`, `export`, `config`) tracks the storage server's RPC surface — see [`../STATUS.md`](../STATUS.md) and [`../../storage/STATUS.md`](../../storage/STATUS.md) for the roadmap. Device-flow login wires up once the storage AS exposes `/authorize` / `/token` / `/device`.

## Why Rust

See [`../STATUS.md`](../STATUS.md) "Decisions to flag — CLI is Rust, not Python".

## Requirements

- Rust 1.88 or later (bumped from 1.83 to satisfy `connectrpc 0.4` /
  `buffa 0.5`, which emit edition-2024 syntax in their generated code).
- A running `ohd-storage-server` over plaintext h2c (`http://`) for any
  command but `version` / `login`. TLS is the deployment's job (Caddy fronts
  the storage process per `../../storage/STATUS.md` "HTTP/3 deferred").
- No system protoc — the build vendors `protoc-bin-vendored 3`.

## Build / run

```bash
cd cli

cargo build                               # debug build (codegens OHDC client from ../../storage/proto)
cargo run -- --help                       # subcommand list
cargo run -- version                      # prints CLI version
cargo build --release                     # release binary at target/release/ohd-connect
cargo install --path .                    # install ~/.cargo/bin/ohd-connect
```

### Driving a round-trip against a local storage

In one terminal:

```bash
cd ../../storage
cargo run -p ohd-storage-server -- init --db /tmp/data.db
TOKEN=$(cargo run -p ohd-storage-server -- issue-self-token --db /tmp/data.db --label demo)
cargo run -p ohd-storage-server -- serve --db /tmp/data.db --listen 127.0.0.1:18443
```

In another:

```bash
cd connect/cli
cargo run -- login  --storage http://127.0.0.1:18443 --token "$TOKEN"
cargo run -- whoami
cargo run -- health
cargo run -- log glucose 6.4
cargo run -- log glucose 120 --unit mg/dL    # auto-converts to mmol/L
cargo run -- query glucose --last-day
```

The end-to-end script `../demo/run.sh` chains the same commands plus
assertions; run with `bash ../demo/run.sh`.

## Layout

```
cli/
├── Cargo.toml
├── build.rs                # connectrpc-build codegen against ../../storage/proto/ohdc/v0/
└── src/
    ├── main.rs             # clap router + per-command handlers
    ├── client.rs           # connectrpc HTTP/2 client wrapper
    ├── credentials.rs      # ~/.config/ohd-connect/credentials.toml (mode 0600)
    ├── events.rs           # CLI-arg → ohdc.v0.EventInput builders
    ├── timeparse.rs        # --last-day/week/month, --from/--to ISO8601
    └── ulid.rs             # Crockford-base32 display helpers
```

## OHDC client

Codegenned at build time by `connectrpc-build 0.4` (mirror of
`../../storage/crates/ohd-storage-server/build.rs`). The output lands in
`$OUT_DIR/_connectrpc.rs` and is included from `main.rs` as `mod proto`,
yielding `proto::ohdc::v0::OhdcServiceClient<T>`. Wire-compatible with the
storage server's `OhdcAdapter` because both compile from the same
`.proto` schema.

This deviates from the original `shared/ohdc-clients/rust/` plan
(see `../shared/ohdc-client-stub.md`): the CLI codegens directly from
`../../storage/proto/`, eliminating the need for a separate vendored crate.
A future "publish a single `ohdc-client-rust` crate" pass could collapse the
two `build.rs` files into one shared dependency, but it isn't required for
the v1 demo.

## Login flow

The spec end-state is OAuth 2.0 Device Authorization Grant (RFC 8628), per
[`../spec/auth.md`](../spec/auth.md) "CLI clients":

```
$ ohd-connect login --storage https://ohd.cloud.example.com
Open https://ohd.cloud.example.com/device on any browser
Enter code:  BCDF-XYZW
Waiting for confirmation… (expires in 10 minutes)
✓ Logged in as user 01HF8K2P… — credentials saved to ~/.config/ohd-connect/credentials
```

The storage server doesn't yet expose `/authorize` / `/token` / `/device`
(per `../../storage/STATUS.md` "HTTP-only OAuth/discovery endpoints"). Until
those land, the CLI accepts a token issued out-of-band:

```
$ ohd-connect login --storage http://127.0.0.1:18443 --token ohds_<base32>
saved credentials to ~/.config/ohd-connect/credentials.toml (mode 0600)
storage: http://127.0.0.1:18443
```

Issue the token from the storage side:

```
$ ohd-storage-server issue-self-token --db /path/to/data.db --label cli
ohds_KGBEFR…
```

Credentials file shape (`~/.config/ohd-connect/credentials.toml`, mode 0600):

```toml
storage_url = "http://127.0.0.1:18443"
token = "ohds_…"
```

When the storage device flow lands, the CLI will gain `refresh` /
`expires_at_ms` fields per `../spec/auth.md` "CLI credentials file layout"
without breaking the v1 single-token shape (the new fields are optional
TOML keys).

## Test

```bash
cargo test
```

## Distribution

Native packages (`.deb`, `.rpm`, Arch PKGBUILD) for `ohd-connect` are wired up at the repo root — see [`../../PACKAGING.md`](../../PACKAGING.md). Future post-v1: Homebrew tap, `curl … | sh` installer, `cargo install ohd-connect` from crates.io.

No pip / Python packaging — the CLI is a self-contained Rust binary.

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
