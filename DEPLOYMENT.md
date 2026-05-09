# OHD — Deployment

> Operator-facing deployment guide. Includes the live `ohd.dev` reference deployment.

## Live reference deployment (ohd.dev)

The OHD project runs a public reference deployment serving:

| URL | Service |
|---|---|
| `https://ohd.dev` | Static landing page (`landing/`) |
| `https://www.ohd.dev` | Same as apex |
| `https://storage.ohd.dev` | `ohd-storage-server` (OHDC over Connect-RPC, HTTP/2 via Caddy) |
| `https://relay.ohd.dev` | `ohd-relay` REST endpoints (HTTP/2 via Caddy) |
| `udp://relay.ohd.dev:9001` | `ohd-relay` raw QUIC tunnel (ALPN `ohd-tnl1`, direct to host) |
| `https://openhealthdata.org` | 301 → `https://ohd.dev` |
| `https://www.openhealthdata.org` | 301 → `https://ohd.dev` |

### Infrastructure

- **Host**: Hetzner Cloud `cax11` (ARM64, 2 vCPU / 4 GB / 40 GB), Falkenstein (`fsn1-dc14`)
- **Server name**: `ohd-demo-fsn1` (ID `128710106`)
- **IPv4**: `178.105.71.238`
- **IPv6**: `2a01:4f8:c014:d23b::1`
- **OS**: Ubuntu 24.04 LTS (aarch64)
- **Docker**: 29.4.1 (system-installed)
- **DNS**: Cloudflare zones `ohd.dev` and `openhealthdata.org`, both account `Jakub@leska.me's Account`. Records DNS-only (not proxied) so Caddy can do HTTP-01 challenge for Let's Encrypt.

### Stack

```
        ┌──────────────────────────────────────────────┐
        │  Caddy 2 (TLS, HTTP/3, reverse proxy)        │
        │  :80, :443/tcp, :443/udp                     │
        └──┬───────────┬───────────┬───────────────────┘
           │           │           │
   /srv/ohd-landing  storage:8443  relay:8443
   (static files)    (Connect-RPC) (REST)

           Plus, bypassing Caddy:
              relay:9001/udp  (raw QUIC tunnel ALPN ohd-tnl1)
```

Compose at `deploy/host/docker-compose.yml`; Caddyfile at `deploy/host/Caddyfile`.

### How it was set up

1. **Hetzner server**. Existed prior to this deployment as `ohd-demo-fsn1` (cax11 ARM, see above). Created via the project's Hetzner Cloud account. Both project SSH keys (`jakub@leska.me`, `desktop`) are authorized.

2. **Cloudflare DNS** (zone IDs in `deploy/host/dns-records.json` for re-application). Records added via the Cloudflare API:
   - `ohd.dev`, `www.ohd.dev`, `storage.ohd.dev`, `relay.ohd.dev` → A `178.105.71.238`
   - `www.openhealthdata.org` → A `178.105.71.238` (apex was already set)
   - All `proxied: false` so Caddy can request Let's Encrypt certs.

3. **Repo sync**. Project rsynced to `/opt/ohd/` on the server. Re-sync from a workstation:
   ```bash
   cd /path/to/local/ohd
   rsync -az --exclude='target/' --exclude='node_modules/' --exclude='.venv/' \
         --exclude='__pycache__/' --exclude='dist/' --exclude='build/' \
         --exclude='*.db' --exclude='*.db-wal' --exclude='*.db-shm' \
         --exclude='.git/' \
         ./ root@178.105.71.238:/opt/ohd/
   ```
   Cargo.lock files MUST be present (the Dockerfiles `COPY` them); don't add `Cargo.lock` to the rsync excludes.

4. **Bring up the stack**:
   ```bash
   ssh root@178.105.71.238
   cd /opt/ohd
   docker compose -f deploy/host/docker-compose.yml up --build -d
   ```
   First build is ~10–15 min on ARM (cargo compile of storage core + server + relay).

5. **Initialize storage** (one-time after first start):
   ```bash
   docker compose -f /opt/ohd/deploy/host/docker-compose.yml exec ohd-storage \
     ohd-storage-server issue-self-token --db /var/lib/ohd-storage/storage.db
   # → ohds_…  (write down — there's no recovery yet without BIP39 mnemonic)
   ```

   The DB is auto-created on first server start — schema migrations run during `Storage::open`. The token printed by `issue-self-token` is a self-session bearer for storage — **never commit it anywhere public**. The live deployment's token is held out-of-band by the maintainer; rotate via `issue-self-token` again whenever needed (the old row stays in `_tokens` until you `revoke-self-token`).

6. **Verify**:
   ```bash
   curl -sI https://ohd.dev/                                  # 200 OK, landing page
   curl https://storage.ohd.dev/ohdc.v0.OhdcService/Health \  # 200 OK + status: ok
        -H 'Content-Type: application/json' --data '{}'
   curl https://relay.ohd.dev/v1/auth/info                    # 200 OK + JSON
   ```

### Caddy auto-TLS

Caddy auto-provisions Let's Encrypt certs on first HTTPS request to each hostname. State in the `caddy_data` Docker volume — survives container restart. To force renewal: `docker compose exec caddy caddy reload --config /etc/caddy/Caddyfile`.

### Updates / redeploy

**Option A — rsync source + remote build (slow first time, fast incrementals):**
```bash
cd /path/to/local/ohd
rsync -az --exclude='target/' --exclude='node_modules/' --exclude='.venv/' \
      --exclude='__pycache__/' --exclude='dist/' --exclude='build/' \
      --exclude='*.db' --exclude='*.db-wal' --exclude='*.db-shm' \
      --exclude='.git/' \
      ./ root@178.105.71.238:/opt/ohd/
ssh root@178.105.71.238 'cd /opt/ohd && \
  docker compose -f deploy/host/docker-compose.yml up --build -d'
```

**Option B — local build + image transfer (faster diffs):** when you have an ARM build environment locally (or `docker buildx --platform linux/arm64` with QEMU):
```bash
docker build --platform linux/arm64 -t ohd-storage:dev -f storage/deploy/Dockerfile .
docker build --platform linux/arm64 -t ohd-relay:dev   -f relay/deploy/Dockerfile   .
docker save ohd-storage:dev ohd-relay:dev | gzip | \
  ssh root@178.105.71.238 'gzip -d | docker load'
ssh root@178.105.71.238 'cd /opt/ohd && \
  docker compose -f deploy/host/docker-compose.yml up -d'
```

Storage data persists in the `ohd_storage_data` Docker volume across redeploys; relay state in `ohd_relay_data`; Caddy auto-TLS state in `host_caddy_data`.

### Backups

```bash
# Storage SQLite snapshot:
ssh root@178.105.71.238 'docker run --rm -v ohd_storage_data:/d -v /tmp:/o \
  alpine tar czf /o/ohd-storage-$(date +%F).tar.gz -C /d .'
scp root@178.105.71.238:/tmp/ohd-storage-*.tar.gz ./backups/
```

`relay_data` (relay's own SQLite — registrations, OAuth state) similarly under volume `ohd_relay_data`.

### Costs

Single cax11 in fsn1: **~€3.79/mo** at the Hetzner Cloud public pricing (May 2026). DNS via Cloudflare free tier. Domains (`ohd.dev`, `openhealthdata.org`) per the project's existing registrar billing. Caddy + Let's Encrypt: free. No CDN — landing page is small enough not to need one for now.

### Tear-down

```bash
# Stop the stack, drop volumes:
ssh root@178.105.71.238 'cd /opt/ohd && \
  docker compose -f deploy/host/docker-compose.yml down -v'

# Delete the server (irreversible):
# via Hetzner Cloud MCP: hetzner_delete_server --id 128710106
# or via console at console.hetzner.cloud

# Delete DNS records via Cloudflare API or dashboard.
```

### Known gaps in the live deployment

- **APK download** at `/downloads/ohd-connect-latest.apk` is a placeholder — Caddy currently 302-redirects to GitHub Releases. The actual APK build (NDK + cargo-ndk + uniffi-bindgen) hasn't been wired into a CI pipeline yet.
- **HTTP/3 in-binary** is disabled at the storage container level (it's reachable, but Caddy is the HTTP/3 terminator from clients; storage talks h2c upstream). Switching the storage container to listen on `--http3-listen` would require host UDP port-mapping; see `storage/STATUS.md` "HTTP/3 (in-binary) — landed".
- **Email / monitoring**: no alerting yet. Caddy logs to stdout (Docker journal); `docker compose logs -f` is the only observability.
- **Backups are manual**. A cron-based snapshot to off-host storage (Hetzner Storage Box / S3) is the next step.

---

## Other deployment shapes

| Goal | Where to look |
|---|---|
| Storage daemon (Docker, single-host) | [`storage/deploy/README.md`](storage/deploy/README.md) |
| Relay daemon (Docker + Caddy) | [`relay/deploy/README.md`](relay/deploy/README.md) |
| Care SPA + MCP (clinic deployment) | [`care/deploy/README.md`](care/deploy/README.md) |
| Emergency dispatch + relay (EMS) | [`emergency/deploy/README.md`](emergency/deploy/README.md) |
| End-to-end write-with-approval demo | [`care/demo/README.md`](care/demo/README.md) |
| Native packages (.deb / .rpm / Arch) | [`PACKAGING.md`](PACKAGING.md) |

## Topologies

| Shape | Components | Where to start |
|---|---|---|
| **Personal / on-device** | Connect (Android/iOS) links the storage Rust core via uniffi. No server. | [`connect/android/BUILD.md`](connect/android/BUILD.md) |
| **Personal + remote primary** | Connect + single `ohd-storage` + optional `ohd-relay`. | [`storage/deploy/README.md`](storage/deploy/README.md) + [`relay/deploy/README.md`](relay/deploy/README.md) |
| **Clinical (Care)** | Care SPA + Care MCP + Postgres + Caddy. Talks to patient storages over OHDC. | [`care/deploy/README.md`](care/deploy/README.md) |
| **Emergency / EMS station** | Relay (authority mode) + Dispatch + Postgres-records + Caddy. | [`emergency/deploy/README.md`](emergency/deploy/README.md) |
| **Bare-metal** | Native `.deb` / `.rpm` / Arch packages | [`PACKAGING.md`](PACKAGING.md) |

## Cross-cutting

- Canonical deploy design: [`spec/docs/design/deployment.md`](spec/docs/design/deployment.md)
- Encryption & key handling at deployment time: [`storage/spec/encryption.md`](storage/spec/encryption.md)
- Per-OIDC issuer gating on relay registration: [`relay/STATUS.md`](relay/STATUS.md) "Per-OIDC registration gating"

## License

Dual-licensed `Apache-2.0 OR MIT` — see [`spec/LICENSE`](spec/LICENSE).
