# OHD Storage — deployment

Single-service Docker deployment for `ohd-storage-server`.

## Quick start

```bash
# From the storage/ directory:
cp deploy/.env.example deploy/.env

# One-time DB init (creates /var/lib/ohd-storage/storage.db inside the volume):
docker compose -f deploy/docker-compose.yml run --rm storage init \
  --db /var/lib/ohd-storage/storage.db

# Issue a self-session token. Write this down — there is no recovery.
docker compose -f deploy/docker-compose.yml run --rm storage issue-self-token \
  --db /var/lib/ohd-storage/storage.db
# → ohds_…

# Bring up the server.
docker compose -f deploy/docker-compose.yml up -d

# Verify it's serving.
curl http://localhost:8443/ohdc.v0.OhdcService/Health \
  -H 'Content-Type: application/json' \
  --data '{}'
```

## Building manually

```bash
docker build -t ohd-storage:dev -f deploy/Dockerfile .
```

The build pulls SQLCipher + OpenSSL via `bundled-sqlcipher`, so the runtime image needs no system libraries beyond `ca-certificates`. First build is slow (~5–10 min); subsequent builds are cached.

## Ports

| Port | Protocol | Purpose |
|---|---|---|
| 8443/tcp | HTTP/2 | OHDC RPC (default) |
| 8443/udp | HTTP/3 (QUIC) | OHDC RPC over QUIC — opt-in via `--http3-listen ADDR:PORT`. See `../STATUS.md` "HTTP/3 (in-binary)". |

## Volumes

| Volume | Mount point | Purpose |
|---|---|---|
| `ohd_storage_data` | `/var/lib/ohd-storage` | SQLite database file + WAL + sidecar attachment blobs |

The volume is named so the database survives `docker compose down`. To wipe and start fresh: `docker volume rm ohd_storage_data`.

## TLS / public deployment

This compose file exposes plain HTTP/2 (cleartext h2c) on 8443. For public deployments, front with Caddy (auto-TLS via Let's Encrypt) or another terminator. Example Caddyfile snippet:

```
storage.example.com {
    reverse_proxy ohd-storage:8443 {
        transport http {
            versions h2c
        }
    }
}
```

## Subcommands

The image entrypoint is `ohd-storage-server`. Useful subcommands:

```bash
# Run any subcommand by overriding the default `serve` command:
docker compose run --rm storage <subcommand> [flags]

# Examples:
docker compose run --rm storage init --db /var/lib/ohd-storage/storage.db
docker compose run --rm storage issue-self-token --db /var/lib/ohd-storage/storage.db
docker compose run --rm storage issue-grant-token --db /var/lib/ohd-storage/storage.db \
  --read std.blood_glucose,std.heart_rate_resting --write std.clinical_note \
  --approval-mode always --label "Dr. Smith" --expires-days 30
docker compose run --rm storage pending-list --db /var/lib/ohd-storage/storage.db
docker compose run --rm storage pending-approve --db /var/lib/ohd-storage/storage.db --ulid 01KR...
```

## Native packages — see `../../packaging/`

`.deb`, `.rpm`, and Arch `PKGBUILD` are now wired up at the OHD root —
see [`../../PACKAGING.md`](../../PACKAGING.md). Build artifacts:

```bash
# From the storage workspace root (one level up from this dir):
cargo build --release -p ohd-storage-server
cargo deb           --no-build -p ohd-storage-server
cargo generate-rpm                  -p ohd-storage-server
# Arch:
cd ../packaging/arch/ohd-storage && makepkg -si
```

The systemd unit at `../packaging/systemd/ohd-storage.service` runs the
binary as the dedicated `ohd-storage` system user with the same
`/var/lib/ohd-storage/storage.db` layout this Docker compose uses, so a
DB created here can be moved to a native install (and vice versa).

## License

Dual-licensed `Apache-2.0 OR MIT` — see [`../../spec/LICENSE`](../../spec/LICENSE).
