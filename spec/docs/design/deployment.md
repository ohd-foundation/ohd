# Design: Deployment

> How operators stand up OHD Storage on real infrastructure. Opinionated toward Hetzner + Docker Compose + Caddy (that's what the project runs on); everything is portable.

> **History:** earlier drafts of this file described a Python / FastAPI / Postgres / Redis stack. That was the v0 prototype and is **superseded**. The contracted v1 architecture is the **Rust core** of `ohd-storage`, fronted by Caddy, with **per-user `.ohd` SQLite/SQLCipher files** on disk and a small **system DB** for cross-user state. This file describes the v1 deployment.

## Deployment targets

| Target | Use case | Compose-deployable? |
|---|---|---|
| **Linux VPS** (Hetzner, Linode, DO, OVH, ...) | OHD Cloud, custom-provider, self-hosted-by-power-user | Yes — primary path |
| **Home server / NAS / Raspberry Pi** | Self-hosted-at-home (often co-deployed with OHD Relay on a public VPS) | Yes |
| **Kubernetes cluster** | Large multi-tenant deployments (national health services, big SaaS) | Yes — Helm chart shape; not MVP |
| **User's phone (on-device)** | Connect mobile app links the Rust core in-process via `uniffi`; no Compose | No (linked, not deployed) |

The rest of this doc covers the server-side targets. On-device is a build/distribution concern, not a deployment one — see `components/connect.md` and `components/storage.md`.

## Stack

```
                     ┌────────────────────────────────┐
                     │  Caddy 2.6+                    │  Automatic HTTPS, HTTP/3
                     │  - reverse proxy               │  TLS 1.3
                     │  - cert mgmt (Let's Encrypt)   │
                     └──────────────┬─────────────────┘
                                    │
                  ┌─────────────────┴─────────────────┐
                  │                                   │
                  ▼                                   ▼
         ┌──────────────────┐              ┌────────────────────┐
         │  ohd-storage     │              │  ohd-relay         │  (optional;
         │  (Rust binary)   │              │  (Rust binary)     │   colocated or
         │                  │              │                    │   separate)
         │  - OHDC server   │              │  - rendezvous      │
         │    (Connect-RPC) │              │  - tunnel forward  │
         │  - per-user      │              │  - push-wake       │
         │    .ohd files    │              │                    │
         │  - system DB     │              └────────────────────┘
         │  - background    │
         │    compactor     │
         └─────┬────────────┘
               │
               ▼
   ┌─────────────────────────────────┐
   │  Disk: per-user files           │
   │                                 │
   │  /var/lib/ohd/                  │
   │  ├── system.db                  │  SQLite (or Postgres on big deployments)
   │  └── users/<hash[:2]>/          │
   │      └── <user_ulid>/           │
   │          ├── data.ohd           │  per-user SQLite + SQLCipher
   │          ├── data.ohd-wal       │  WAL, SQLite-managed
   │          ├── data.ohd-shm       │
   │          └── blobs/             │  attachment payloads,
   │              └── <sha256>/...   │  encrypted with per-user key
   └─────────────────────────────────┘
```

The `ohd-storage` binary is the same one used everywhere — it's a Linux executable on servers and an `.aar`/`.xcframework` linked into Connect mobile. The on-disk format is byte-identical across platforms (a file written on Android opens unchanged on a Linux server).

## What goes where

| State | Location | Encryption | Backup unit |
|---|---|---|---|
| Per-user events, channels, samples, audit, grants, pending events | `/var/lib/ohd/users/<hash[:2]>/<user_ulid>/data.ohd` | SQLCipher 4 (per-user key) | Per-user file (cp / rsync / restic) |
| Per-user attachment blobs | `…/blobs/<sha256>` (SHA-256-addressed) | libsodium `crypto_secretstream` (same per-user key) | Sidecar dir |
| OIDC `(provider, subject) → user_ulid` mapping | System DB: `oidc_identities` table | Database-level encryption (Postgres TDE, or SQLCipher on the system file) | System DB |
| Sessions (`ohds_…` / `ohdr_…` hashes, expiries, revocations) | System DB: `sessions` table | DB-level | System DB |
| Pending invites for `invite_only` registration | System DB: `pending_invites` | DB-level | System DB |
| Pending pairing codes (deferred — see future-implementations/device-pairing.md) | System DB: `pending_pairings` | DB-level | System DB |
| Storage-relay registrations | System DB: `storage_relay_registrations` | DB-level | System DB |
| Push-token registry (per-device FCM/APNs tokens for notifications) | System DB: `push_tokens` | DB-level | System DB |

The boundary rule from [`storage-format.md`](storage-format.md) holds: rows that only make sense given the user's data live per-user (SQLCipher-encrypted with the user's key); rows that must outlive user-file deletion live system-level. Schema details for the system DB tables live in [`auth.md`](auth.md), [`care-auth.md`](care-auth.md), and [`../components/relay.md`](../components/relay.md).

## System DB

For **small deployments** (single-host self-hosted, family server, small clinic): the system DB is a single SQLite file at `/var/lib/ohd/system.db`. Easy backup; single-process; no extra ops.

For **medium-to-large deployments** (OHD Cloud, hospitals, large insurers): the system DB is **Postgres 16+**, accessed by the `ohd-storage` binary via standard pooling. The schema is identical (modulo dialect differences); the storage binary detects from a connection-string flag. Postgres is preferred when:

- More than ~10k active users (SQLite write contention starts mattering for the system DB even though per-user files are isolated).
- Multi-instance deployments (multiple `ohd-storage` processes need a shared system DB).
- The operator already runs Postgres for other things and wants one stack.

**Redis is not used.** Sessions live in the system DB; OAuth state lives in the system DB; rate-limiting is per-user-file or in-process. The simpler-stack property is preserved.

## Concurrency & process model

Each `.ohd` file follows SQLite's WAL constraint — **one writer at a time**. The `ohd-storage` binary runs a small per-file actor pool: one writer task per active user, many reader tasks. Cross-process access to the same `.ohd` file is **not supported**.

For multi-instance deployments (horizontal scaling), routing must pin all writes for a given `user_ulid` to the same instance. Options:
- **Sticky load balancer** with consistent-hashing on `user_ulid` (extracted from the auth token at the LB layer).
- **Per-shard sharding** — operator partitions the user-id space across instances; users on shard A always go to instance A.
- **Inter-process write queue** — instances forward writes for non-owned users to the owning instance via internal RPC. Operationally heavier; suitable when the LB layer can't do consistent hashing.

For Phase 1 / single-instance deployments, none of this matters — one process owns everything.

## Docker Compose (reference)

```yaml
# docker-compose.yml — reference single-host deployment
services:

  caddy:
    image: caddy:2-alpine
    restart: unless-stopped
    ports:
      - "80:80"
      - "443:443"
      - "443:443/udp"     # HTTP/3
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy_data:/data
      - caddy_config:/config
    depends_on:
      - ohd-storage

  ohd-storage:
    image: ohd/storage:latest    # built from openhealth-data/ohd-storage
    restart: unless-stopped
    environment:
      OHD_BIND: 0.0.0.0:8000
      OHD_DATA_DIR: /var/lib/ohd
      OHD_SYSTEM_DB: sqlite:///var/lib/ohd/system.db
      # or, on bigger deployments:
      # OHD_SYSTEM_DB: postgres://ohd:${POSTGRES_PASSWORD}@postgres:5432/ohd_system
      OHD_BASE_URL: https://${OHD_DOMAIN}
      OHD_REGISTRATION_MODE: invite_only         # 'open' | 'invite_only' | 'closed'
      OHD_AUTH_PROVIDERS: ohd_account,google,apple,custom
      OIDC_GOOGLE_CLIENT_ID: ${OIDC_GOOGLE_CLIENT_ID}
      OIDC_GOOGLE_CLIENT_SECRET: ${OIDC_GOOGLE_CLIENT_SECRET}
      # ... apple, microsoft, github, custom-OIDC discovery URL similarly
      RUST_LOG: info,ohd_storage=info
    volumes:
      - ohd_data:/var/lib/ohd
    expose:
      - "8000"

  # Optional: ohd-relay if this host also acts as a relay.
  # Otherwise omit and point grants at a remote relay URL.
  ohd-relay:
    image: ohd/relay:latest
    restart: unless-stopped
    environment:
      OHD_RELAY_BIND: 0.0.0.0:8001
      OHD_RELAY_BASE_URL: https://${OHD_RELAY_DOMAIN}
    expose:
      - "8001"

volumes:
  ohd_data:
  caddy_data:
  caddy_config:
```

### Caddyfile

```caddyfile
{$OHD_DOMAIN} {
    encode zstd gzip

    # OHDC Connect-RPC service — every OHDC operation lives under this.
    handle /ohdc.v1.OhdcService/* {
        reverse_proxy ohd-storage:8000
    }

    # OAuth / OIDC HTTP endpoints used by browser-based and CLI clients.
    handle /authorize    { reverse_proxy ohd-storage:8000 }
    handle /token        { reverse_proxy ohd-storage:8000 }
    handle /oidc-callback { reverse_proxy ohd-storage:8000 }
    handle /device       { reverse_proxy ohd-storage:8000 }
    handle /oauth/*      { reverse_proxy ohd-storage:8000 }
    handle /.well-known/* { reverse_proxy ohd-storage:8000 }

    # Health, metrics, optional admin static pages
    handle /health       { reverse_proxy ohd-storage:8000 }
    handle /metrics      { reverse_proxy ohd-storage:8000 }   # restrict at the network layer

    header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains"
        X-Content-Type-Options "nosniff"
        X-Frame-Options "DENY"
        Referrer-Policy "strict-origin-when-cross-origin"
    }
}

# Separate Caddy site for the relay if colocated:
{$OHD_RELAY_DOMAIN} {
    reverse_proxy ohd-relay:8001
    encode zstd gzip
}
```

## Environment file

A `.env`, never committed:

```
OHD_DOMAIN=ohd.your-domain.org
OHD_RELAY_DOMAIN=relay.your-domain.org

OIDC_GOOGLE_CLIENT_ID=...
OIDC_GOOGLE_CLIENT_SECRET=...
OIDC_APPLE_CLIENT_ID=...
OIDC_APPLE_TEAM_ID=...
OIDC_APPLE_KEY_ID=...
OIDC_APPLE_PRIVATE_KEY_PATH=/run/secrets/apple_p8

POSTGRES_PASSWORD=<long random>   # only if using Postgres for system DB
```

For real deployments, prefer a secrets manager (Vault, AWS/GCP/Hetzner secrets, sealed-secrets in K8s) over plain `.env` — but `.env` with `0600` perms is acceptable for solo / small-clinic deployments.

## Hetzner provisioning (founder's path)

1. Create VM via Hetzner API (scripted), passing SSH key.
2. Cloud-init sets up base packages + Docker.
3. `scp` Compose, Caddyfile, `.env`, optional Postgres init scripts.
4. SSH and `docker compose up -d`.
5. Point DNS (`OHD_DOMAIN` and optionally `OHD_RELAY_DOMAIN`) to the new IP.
6. Caddy auto-provisions TLS on the first HTTPS request.

Sketch (uses the same `hcloud` client the original `deploy.py` used):

```python
from hcloud import Client
import subprocess, os, time

client = Client(token=os.environ["HCLOUD_TOKEN"])

server = client.servers.create(
    name="ohd-prod",
    server_type=client.server_types.get_by_name("cx22"),   # 2 vCPU, 4 GB, 40 GB NVMe
    image=client.images.get_by_name("ubuntu-24.04"),
    location=client.locations.get_by_name("hel1"),
    ssh_keys=[client.ssh_keys.get_by_name("my-key")],
)

# wait for SSH, rsync compose stack, run `docker compose up -d`...
```

**Server sizing:**

| Scale | Hetzner type | RAM / disk |
|---|---|---|
| Personal (one user, on-device + cache server) | `cx22` | 4 GB / 40 GB |
| Small clinic / family (10–100 users) | `cx32` | 8 GB / 80 GB |
| Mid SaaS (1k–10k users) | `cax21` (ARM, cheaper) or `cpx41` | 16+ GB / 160+ GB; consider attached volume for `ohd_data` |
| Large multi-tenant (10k+ users) | Multiple instances + Postgres for system DB + S3-compatible object storage for blobs |

A cx22 holds one user's lifetime (40 GB NVMe ≈ 70+ years of dense CGM + HR data) easily. SaaS sizing is dominated by user-count more than per-user data volume.

## Backups

The unit of backup is **the per-user file + its blobs sidecar**, plus the system DB.

### Strategy

| Component | Tool | Cadence | Retention |
|---|---|---|---|
| `users/<hash>/<ulid>/data.ohd` and `…/blobs/` | `rsync` to a backup destination, or `restic` for deduplication and encryption | Hourly incremental, daily full snapshot | 7 daily, 4 weekly, 12 monthly |
| `system.db` (or Postgres dump) | `sqlite3 .backup` for SQLite, `pg_dump` for Postgres | Hourly | Same |
| Encryption keys (KMS / passphrase escrow) | Whatever the operator's KMS is | Continuous (KMS handles it) | Per KMS policy |

Backup destinations: S3-compatible object storage (Hetzner Storage Box, B2, R2, etc.), an off-site server via `rsync over ssh`, or both.

### Per-user surgery

A single user's data can be backed up, restored, exported, or deleted independently — the file boundary is the privacy and operational boundary. This makes:

- **GDPR / right-to-be-forgotten**: `rm -rf users/<hash>/<ulid>/` + scrub the system DB row.
- **Per-user export**: zip the file + blobs, hand to the user.
- **Per-user migration to another instance**: same.
- **Selective restore**: re-place one user's directory from backup without touching others.

trivial. Avoid the all-or-nothing pain of monolithic Postgres backups.

### Restore drills

Document the restore procedure. Test it. **Untested backups are not backups.** A reasonable cadence is quarterly: pick a random user's files from yesterday's backup, restore to a staging instance, verify the user can authenticate and read their data.

### User-side escape hatch

Independent of operator backups, every user can hit `Auth.Export` (OHDC RPC) and download their full portable export anytime. This is the durable promise: even if the operator goes away, users have their data.

## Monitoring

Minimum viable, shipping with `ohd-storage`:

- **`GET /health`** — returns `200 OK` if the storage binary is up and the system DB is reachable. For Caddy / Docker healthchecks and external uptime monitors.
- **`GET /metrics`** — Prometheus exposition. Standard counters: requests by RPC method, latency histograms, audit-row insertions, sample-block writes, per-user file open count, KMS errors, sync errors. Restrict at the network layer (don't expose publicly).
- **Structured logs** — JSON to stdout. `tracing` crate; controlled by `RUST_LOG`. Pipe to whatever the operator collects (Loki, OpenSearch, CloudWatch, etc.).

For mid-to-large deployments add Grafana dashboards (the project ships a starter dashboard JSON), AlertManager rules (per-user-file lock contention spikes, KMS unavailable, push-token rejection rate, sync watermark drift), and external uptime monitoring against `/health`.

## Hardening checklist (pre-production)

- [ ] Only Caddy's 80 / 443 / 443-UDP exposed; everything else on the internal Docker network.
- [ ] SSH hardened: key auth only, no root login, fail2ban or equivalent.
- [ ] System DB not exposed to the internet (loopback or Docker network only).
- [ ] `.env` and any key files have `0600` perms.
- [ ] Secrets in a secrets manager, not committed.
- [ ] OIDC client secrets rotatable without downtime (multiple values supported in config).
- [ ] Regular OS updates (`unattended-upgrades` on Debian/Ubuntu).
- [ ] Automated backups running and tested.
- [ ] Rate limiting configured at both Caddy and storage levels.
- [ ] Audit log query path works from CLI for incident response.
- [ ] KMS / HSM provisioned and tested for the system DB and (where applicable) per-user file-key wrapping.
- [ ] HTTP/3 enabled; HTTP/2 fallback verified.
- [ ] Caddy auto-renew working; cert expiry monitored.
- [ ] Disaster-recovery runbook written (key compromise, host loss, region loss).

## Multi-deployment, one codebase

Same Docker image (`ohd/storage:latest`) serves:

- Personal self-hosted (one user, one host).
- OHD Cloud (many users, multiple instances + Postgres + object-store blobs).
- Custom-provider deployments (clinic, insurer, employer wellness, research consortium) with that operator's OIDC providers, registration mode, KMS.
- Hospital-internal deployments behind clinical networks.

The differences are configuration and operational scale, not code. Multi-tenancy is built in at the per-user-file level from the start.

## Cross-references

- On-disk format and schema: [`storage-format.md`](storage-format.md)
- Auth, sessions, system-DB tables: [`auth.md`](auth.md)
- Care operator auth and grant vault: [`care-auth.md`](care-auth.md)
- Relay deployment: [`../components/relay.md`](../components/relay.md)
- Storage component spec: [`../components/storage.md`](../components/storage.md)
- User-facing tradeoffs of the four deployment modes: [`../deployment-modes.md`](../deployment-modes.md)
