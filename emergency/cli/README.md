# `ohd-emergency` — Operator CLI

> Cargo bin crate. Operator-side CLI for OHD Emergency deployments.

## What it does

| Subcommand | Backing |
|---|---|
| `login` | Writes `~/.config/ohd-emergency/config.toml` (mode 0600). |
| `cert info` | Reads the configured authority cert PEM, prints subject / issuer / validity / SHA-256 fingerprint. |
| `cert refresh` | TBD until Fulcio integration lands; prints informative pointer to `../spec/emergency-trust.md`. |
| `cert rotate` | TBD until Fulcio integration + key-rotation policy land. |
| `roster list` | Lists all responders in the operator-side roster TOML. |
| `roster add --label NAME --role ROLE` | Adds a responder (role: `responder` or `dispatcher`). |
| `roster remove --label NAME` | Removes a responder. |
| `roster status` | Shows on-duty / total counts. |
| `audit list [--from ISO --to ISO --responder LABEL]` | Calls `OhdcService.AuditQuery`. Storage's handler is `Unimplemented` today; the CLI surfaces the error cleanly. When wired this prints real audit rows. |
| `audit export --output FILE.csv` | Same as `audit list` but writes RFC 4180 CSV. |
| `case-export --case-ulid X --output FILE.json` | Calls `OhdcService.GetCase` + `OhdcService.QueryEvents` + (best-effort) `OhdcService.AuditQuery`. Writes a portable JSON archive. |

## Configuration

`~/.config/ohd-emergency/config.toml` (mode 0600):

```toml
storage_url    = "http://localhost:8443"            # OHDC endpoint
token          = "ohds_..."                         # operator's bearer
station_label  = "EMS Prague Region"                # optional, free-form
authority_cert = "/etc/ohd-emergency/ca.pem"        # optional, used by `cert info`
roster_path    = "/etc/ohd-emergency/roster.toml"   # optional, overrides default
```

The roster TOML lives at `$XDG_DATA_HOME/ohd-emergency/roster.toml` by
default (or `roster_path` from `config.toml`). It's append-only:

```toml
[[responder]]
label    = "Dr.Test"
role     = "responder"
added_at = "2026-05-08T12:34:56Z"
on_duty  = true
```

Global flags:

- `--storage URL` / `--token TOKEN` — per-invocation override (handy for
  one-off calls, scripts, or testing against a throwaway server).
- `--insecure-skip-verify` — skip TLS verification when speaking
  `https+h3://` to a server with a self-signed cert. Dev / test only.

## OHDC transports

Mirrors `connect/cli`:

- `http://host:port` — HTTP/2 over plaintext h2c (TLS termination is
  Caddy's job per `../../storage/STATUS.md` "Wire-format swap").
- `https+h3://host:port` — HTTP/3 over QUIC, in-binary client. Requires
  the storage server to be running with `--http3-listen` (see
  `../../storage/STATUS.md`).

## Smoke tests

```bash
# Build
cd cli
cargo build

# Help tree
cargo run -- --help
cargo run -- cert --help
cargo run -- roster --help
cargo run -- audit --help

# cert info against a sample PEM
cat > /tmp/test-authority.pem <<'EOF'
-----BEGIN CERTIFICATE-----
... real PEM here ...
-----END CERTIFICATE-----
EOF
cargo run -- login \
  --storage http://localhost:8443 \
  --token ohds_FAKE \
  --authority-cert /tmp/test-authority.pem \
  --station-label "EMS Test"
cargo run -- cert info

# Roster round-trip (operator-side TOML; no server needed)
cargo run -- roster add --label "Dr.Test" --role responder
cargo run -- roster add --label "Disp.Alice" --role dispatcher
cargo run -- roster list
cargo run -- roster status
cargo run -- roster remove --label "Dr.Test"

# Unit tests
cargo test
```

## case-export archive schema

A single JSON file. v1 schema id: `ohd-emergency.case-export.v1`.

```jsonc
{
  "schema":         "ohd-emergency.case-export.v1",
  "exported_at_ms": 1778284800000,
  "exported_by":    "ohd-emergency 0.0.1",
  "exporter_label": "EMS Prague",
  "storage_url":    "http://localhost:8443",
  "case_ulid":      "01JT...",                  // 26-char Crockford-base32
  "case":   { /* pb::Case as proto3 JSON */ },
  "events": [ /* pb::Event as proto3 JSON */ ],
  "audit":  [ /* pb::AuditEntry as proto3 JSON */ ],
  "audit_status": "ok" | "rpc_unimplemented" | "rpc_error"
}
```

`audit_status` records why the audit array might be empty:

- `ok` — `AuditQuery` succeeded (will be the default once storage's
  AuditQuery handler lands).
- `rpc_unimplemented` — `OhdcService.AuditQuery` returned Unimplemented
  (current storage state per `../../storage/STATUS.md`).
- `rpc_error` — some other server / transport error; check the CLI's
  stderr.

Field naming: the proto messages embedded under `case` / `events` /
`audit` use the buffa-emitted proto3 JSON form (`lowerCamelCase`). The
archive's own envelope fields use `snake_case` (it's the human-facing
shape).

The archive is written atomically (temp file + `rename`). Large cases
remain a single file; we may switch to a tar of `header.json` +
`events.ndjson` + `audit.ndjson` if archive sizes outgrow the JSON
pretty-printer (followups list in `../STATUS.md`).

## Layout

```
cli/
├── Cargo.toml      # clap 4 + connectrpc 0.4 + buffa 0.5 + http/3 stack
├── build.rs        # codegen against ../../storage/proto/ohdc/v0/ohdc.proto
├── README.md       # this file
└── src/
    ├── main.rs        # CLI entry, runtime, login
    ├── config.rs      # ~/.config/ohd-emergency/config.toml
    ├── client.rs      # OHDC client (HTTP/2 h2c + HTTP/3)
    ├── ulid.rs        # Crockford-base32 ULID round-trip
    ├── cert.rs        # cert info / refresh / rotate
    ├── roster.rs      # operator-side roster TOML
    ├── audit.rs       # audit list / export (CSV)
    └── case_export.rs # case-export archive builder
```

## What's stubbed / blocked

- `cert refresh` / `cert rotate` — blocked on the relay's Fulcio
  integration; spec lives in `../spec/emergency-trust.md`.
- `audit list` / `audit export` — wires unchanged once
  `OhdcService.AuditQuery` lands (storage's handler returns
  `Unimplemented` today; see `../../storage/STATUS.md` "8. AuditQuery
  server-streaming handler"). The CLI surfaces the Unimplemented error.
- `case-export` — `OhdcService.GetCase` is also `Unimplemented` in the
  current storage server, so this command will return the same wire
  error. Once `GetCase` + `QueryEvents` are end-to-end, archives are
  produced.
- Roster network mode — `roster *` reads / writes a local TOML for v0.
  When `relay/` exposes a roster API this module gains a network mode
  with the IdP as the source of truth.

## Distribution

Native packages (.deb / .rpm / Arch) for `ohd-emergency` are wired up at the repo root — see [`../../PACKAGING.md`](../../PACKAGING.md).

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
