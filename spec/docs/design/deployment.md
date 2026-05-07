# Design: Deployment

> How OHD instances get deployed. Opinionated toward Hetzner + Docker Compose + Caddy because that's what the project runs on, but everything is portable.

## Deployment targets

OHD is designed to deploy to three main targets:

1. **A VPS** (Hetzner, Linode, DigitalOcean, your own server) — the primary and recommended path.
2. **A home server** (NAS, Raspberry Pi with SSD, spare laptop) — for users who want zero cloud dependency.
3. **A hosted Kubernetes cluster** — for large deployments (hospitals, SaaS). Not the MVP path.
4. **A user's phone** — OHD on-device. Phase 2+. Requires a subset of the codebase and a lightweight database (SQLite, likely).

For all VPS and home-server deployments, Docker Compose is the unit of deployment.

## Stack

```
┌──────────────────────────────────┐
│  Caddy                            │  Automatic HTTPS (Let's Encrypt)
│  - reverse proxy                  │  HTTP/2, HTTP/3
│  - cert management                │
└────────────┬─────────────────────┘
             │
             ▼
┌──────────────────────────────────┐
│  ohd-api (FastAPI)                │  The OHD Core service
│  - REST API                       │  Python 3.11+
│  - OIDC auth                      │
│  - Background jobs                │
└────────┬────────────────┬────────┘
         │                │
         ▼                ▼
┌─────────────────┐  ┌─────────────────┐
│  PostgreSQL 16  │  │  Redis 7        │
│  - events       │  │  - sessions     │
│  - users        │  │  - cache        │
│  - grants       │  │  - rate limits  │
│  - audit log    │  └─────────────────┘
└─────────────────┘

Optional:
┌──────────────────────────────────┐
│  ohd-web (static dashboard)       │  Served by Caddy as static files
└──────────────────────────────────┘
```

## Docker Compose

```yaml
# docker-compose.yml (reference)
version: "3.9"

services:

  caddy:
    image: caddy:2-alpine
    restart: unless-stopped
    ports:
      - "80:80"
      - "443:443"
      - "443:443/udp"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy_data:/data
      - caddy_config:/config
      - ./web/dist:/srv/web:ro  # optional web dashboard
    depends_on:
      - ohd-api

  ohd-api:
    image: ohd/api:latest  # or build: ./api
    restart: unless-stopped
    environment:
      DATABASE_URL: postgresql+asyncpg://ohd:${POSTGRES_PASSWORD}@postgres:5432/ohd
      REDIS_URL: redis://redis:6379/0
      OHD_SECRET_KEY: ${OHD_SECRET_KEY}
      OHD_BASE_URL: https://${OHD_DOMAIN}
      OIDC_GOOGLE_CLIENT_ID: ${OIDC_GOOGLE_CLIENT_ID}
      OIDC_GOOGLE_CLIENT_SECRET: ${OIDC_GOOGLE_CLIENT_SECRET}
      LOG_LEVEL: info
    depends_on:
      postgres:
        condition: service_healthy
      redis:
        condition: service_healthy
    expose:
      - "8000"

  postgres:
    image: postgres:16-alpine
    restart: unless-stopped
    environment:
      POSTGRES_DB: ohd
      POSTGRES_USER: ohd
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
    volumes:
      - postgres_data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ohd"]
      interval: 10s
      timeout: 5s
      retries: 5

  redis:
    image: redis:7-alpine
    restart: unless-stopped
    volumes:
      - redis_data:/data
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 10s
      timeout: 5s
      retries: 5

volumes:
  postgres_data:
  redis_data:
  caddy_data:
  caddy_config:
```

## Caddyfile

```
{$OHD_DOMAIN} {
    encode gzip zstd

    # API
    handle /api/* {
        reverse_proxy ohd-api:8000
    }

    # MCP endpoints (if served from same domain)
    handle /mcp/* {
        reverse_proxy ohd-api:8000
    }

    # Static web dashboard (optional)
    handle /* {
        root * /srv/web
        try_files {path} /index.html
        file_server
    }

    # Security headers
    header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains"
        X-Content-Type-Options "nosniff"
        X-Frame-Options "DENY"
        Referrer-Policy "strict-origin-when-cross-origin"
    }
}
```

## Environment

A `.env` file, never committed:

```
OHD_DOMAIN=ohd.mydomain.org
OHD_SECRET_KEY=<generated, 64 random bytes>
POSTGRES_PASSWORD=<generated, long>
OIDC_GOOGLE_CLIENT_ID=<from Google Cloud Console>
OIDC_GOOGLE_CLIENT_SECRET=<from Google Cloud Console>
```

## Hetzner provisioning (automated)

The founder's workflow for Hetzner deployments:

1. **Create VM via Hetzner API** (scripted), providing SSH key.
2. **Wait for cloud-init** to complete and SSH to become available.
3. **Ship Docker Compose and configs via `scp`/`rsync`.**
4. **SSH in and run `docker compose up -d`.**
5. **Point DNS** (Cloudflare API? Manual?) to the new server IP.
6. **Caddy auto-provisions TLS** on first HTTPS request.

A script to automate this is part of the deployment tooling. Rough skeleton:

```python
# deploy.py (sketch)
import subprocess
from hcloud import Client

client = Client(token=os.environ["HCLOUD_TOKEN"])

server = client.servers.create(
    name="ohd-prod",
    server_type=client.server_types.get_by_name("cx22"),
    image=client.images.get_by_name("ubuntu-24.04"),
    location=client.locations.get_by_name("hel1"),
    ssh_keys=[client.ssh_keys.get_by_name("my-key")],
)

# ... wait for SSH, rsync docker-compose.yml, configs
# ... ssh and run docker compose up
```

**Server size.** For personal use or tens of users: `cx22` (2 vCPU, 4 GB, 40 GB NVMe, ~€4.50/month) is plenty. For thousands of users, scale up or out.

## On-device deployment (phone)

This is a Phase 2+ target but worth keeping in mind architecturally:

- The OHD Core becomes a lightweight Python app bundled with the Android app (via Chaquopy) or rewritten as a Kotlin service.
- Postgres is replaced by SQLite (or Postgres-compatible SQLite.wasm).
- Redis is replaced by an in-memory cache.
- Caddy is not needed (the app queries OHD locally).

Externally accessible phone OHD (so a doctor can query from the internet) is a thorny problem: dynamic IPs, NAT traversal, background battery use. One solution: the phone connects out to a relay server, and Cord apps talk to the relay. Not MVP.

## Backup

Minimum viable backup (MVP):

- Nightly `pg_dump` to a local file.
- Sync to object storage (Hetzner Storage Box, Backblaze B2, etc.) with age/restic.
- Retention: 7 daily, 4 weekly, 12 monthly.

Document the restore procedure, and test it.

For user-facing backup (user wants to back up their own data): the `/export` endpoint gives them a signed JSON file. They can store that wherever they want. This is the "escape hatch" if the provider goes down or the user loses trust.

## Monitoring

MVP:

- Docker logs via `docker compose logs` or shipped to a collector.
- Healthcheck endpoint on `ohd-api`: `GET /health` returns 200 if DB and Redis are reachable.
- External uptime monitoring (Uptime Robot free tier, or the founder's infrastructure) hitting `/health`.

Phase 2:

- Prometheus metrics endpoint (`/metrics`).
- Grafana dashboards.
- Alerting (AlertManager, or Hetzner's built-in monitoring).

## Hardening checklist (pre-production)

- [ ] Only Caddy's 80/443 exposed; everything else on internal Docker network.
- [ ] SSH hardened: key auth only, no root login, fail2ban.
- [ ] Postgres not exposed to the internet.
- [ ] `.env` file has 0600 perms, not world-readable.
- [ ] OHD secret key is 64 random bytes, not reused.
- [ ] Regular OS updates (unattended-upgrades).
- [ ] Automated backups running and tested.
- [ ] Rate limiting configured at both Caddy and app level.
- [ ] Audit log queries work from the CLI for incident response.

## Multiple deployments, one codebase

The same Docker image serves:

- Personal self-hosted deployments (one user, small VPS).
- The OHD project's non-profit SaaS (many users, bigger infra).
- Hospital deployments (private, behind hospital network).

The differences are configuration and operational scale, not code. Multi-tenancy is built into the core.
