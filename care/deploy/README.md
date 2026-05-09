# OHD Care — Deploy

Reference Docker Compose + Caddy stack for standing up Care on the operator's domain.

This is the **single-host** shape — suitable for a solo practitioner, small clinic, or a development / staging environment. For larger shapes (hospital department, mobile crew tablets, clinical-trial site) the same image set ships; what changes is OIDC config, KMS backend, and network topology. See [`../README.md`](../README.md) "Deployment shapes" for the full matrix.

> Status: image references in `docker-compose.yml` (e.g. `ohd/care-mcp:0.1`) point at images you build locally from `../mcp/` until the project publishes registry-hosted images. The Caddyfile and topology are production-shaped.

## What's deployed

| Service | Role |
|---|---|
| `caddy` | Reverse proxy + automatic HTTPS (HTTP/3, HTTP/2 fallback). Serves the SPA from `../web/dist` and routes API/MCP traffic to `care-mcp`. |
| `care-mcp` | The Care MCP server. In v0.1 it also handles operator-side API endpoints (`/api/operator/*`). Splitting `care-api` out is a future refactor. |
| `postgres` | Operator-side state DB (sessions, `care_patient_grants`, `care_operator_audit`). Optional — swap to SQLite for solo deployments. |

Patient OHD storage is **not** part of this stack. Care talks to each patient's storage via OHDC (relay-mediated for phones / NAS, direct for cloud) — only operator-side state lives here.

## Quickstart

```sh
# 1. Configure
cp .env.example .env
chmod 600 .env
$EDITOR .env   # set OHD_CARE_DOMAIN, OIDC_*, POSTGRES_PASSWORD

# 2. Build the web SPA (one-shot until a published image lands)
cd ../web && pnpm install && pnpm build && cd -

# 3. Build the MCP image
docker build -t ohd/care-mcp:0.1 ../mcp

# 4. Provide the KMS passphrase for grant-token encryption-at-rest
mkdir -p secrets
echo "$(openssl rand -base64 64)" > secrets/care_kms_passphrase.txt
chmod 600 secrets/care_kms_passphrase.txt

# 5. Bring up the stack
docker compose up -d

# 6. Point DNS for OHD_CARE_DOMAIN at this host. Caddy auto-provisions TLS
#    on the first HTTPS request.
```

## Layout

```
deploy/
├── docker-compose.yml
├── Caddyfile
├── .env.example
└── secrets/
    └── care_kms_passphrase.txt   # not committed; create per-deployment
```

## Caddy routes

| Route | Backend | Notes |
|---|---|---|
| `/` (and `/index.html`, `/assets/*`) | static `../web/dist` mounted into Caddy | the SPA |
| `/mcp/care/*` | `care-mcp:8001` | Care MCP via Streamable HTTP transport (in implementation phase) |
| `/api/operator/*` | `care-mcp:8001` | operator-side endpoints (login, grant vault, audit) |
| `/authorize`, `/token`, `/oidc-callback`, `/oauth/*`, `/.well-known/*` | `care-mcp:8001` | OIDC flow into Care |
| `/health` | `care-mcp:8001` | liveness probe |
| `/metrics` | `care-mcp:8001` | Prometheus exposition; restricted to RFC1918 client IPs |

## Hardening (pre-production)

The wider OHD checklist applies (`spec/docs/design/deployment.md` "Hardening checklist"). Care-specific items:

- **Per-deployment KMS posture:** the `passphrase` backend is fine for solo / small-clinic deployments. For hospital / clinical-trial sites switch to `aws-kms` / `gcp-kms` / `vault-transit` and remove the local key file.
- **Operator session timeouts:** SPEC §2.2 defaults to 30 min access / 8 h refresh. Tighten in shared-workstation environments.
- **Restrict `/metrics`** at network layer (the Caddyfile already restricts to RFC1918, but a real production deployment runs it on a separate internal subnet).
- **Backups:** the unit of backup is the system DB (`care_system_db`) and any KMS-wrapped key material. Patient data lives in patient storage — Care does not back it up. Document restore drills.
- **Audit retention:** `care_operator_audit` has compliance retention requirements per the deployment's regulatory regime (HIPAA: 6 years; GDPR: contextual). Configure DB-level retention policies.

## Variants

### Solo practitioner — SQLite, no Postgres

Drop the `postgres` service and set:

```env
CARE_SYSTEM_DB=sqlite:///var/lib/care/system.db
```

Mount a volume into `care-mcp` for the SQLite file. Backup is a single `sqlite3 .backup`.

### Hospital department — external Postgres + cloud KMS

Replace the `postgres` service with a connection to the hospital's Postgres cluster. Set `CARE_KMS_BACKEND=vault-transit` (or AWS / GCP equivalent) and remove the `secrets/care_kms_passphrase.txt` file. OIDC points at the hospital ADFS / Entra.

### Mobile / ambulance — tablet kiosk

The MCP service is unnecessary; only the web SPA. Operator session timeouts shortened to ~10 min access. The KMS posture should match the EMS station's broader IT (often a vault-transit setup at the station's central server).

### Clinical-trial site — sponsor-supplied SSO

OIDC issuer is the sponsor's IDP; per-study deployment scaffold. Add per-study branding via Caddy's `header` directives.

## Cross-references

- Component spec: [`../../spec/docs/components/care.md`](../../spec/docs/components/care.md)
- Operator auth & vault: [`../../spec/docs/design/care-auth.md`](../../spec/docs/design/care-auth.md)
- Wider deployment guidance (Hetzner, sizing, backup, monitoring, hardening): [`../../spec/docs/design/deployment.md`](../../spec/docs/design/deployment.md)
- Implementation contract for this directory: [`../SPEC.md`](../SPEC.md)
