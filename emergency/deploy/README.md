# `deploy/` — Reference Deployment for OHD Emergency

> Docker Compose + Caddyfile reference deployment for a small EMS station running OHD Emergency end-to-end.

## What this deploys

Per [`../SPEC.md`](../SPEC.md) "Reference deployment shape" and [`../README.md`](../README.md):

```
internet → caddy ─┬→ relay  (emergency-authority mode, OHDC + signed-request issuer)
                  ├→ dispatch-web  (built ../dispatch/ SPA, static)
                  └→ (postgres-records, internal only)
```

Plus paramedic tablets connecting from the field over HTTPS to `relay.<operator-domain>`.

## Files

| File | Purpose |
|---|---|
| [`docker-compose.yml`](docker-compose.yml) | Service topology: `relay`, `dispatch-web`, `postgres-records`, `caddy`. |
| [`Caddyfile`](Caddyfile) | Outer Caddy: TLS termination + HTTP/3 + reverse proxy on `relay.<domain>` and `dispatch.<domain>`. |
| [`Caddyfile.dispatch`](Caddyfile.dispatch) | Inner Caddy in the dispatch container: serves the SPA from `../dispatch/dist`. |
| [`.env.example`](.env.example) | Env-var template. Copy to `.env` and fill in. |

## Quick start

```bash
cd deploy
cp .env.example .env
# edit .env to set OPERATOR_DOMAIN, OPERATOR_ADMIN_EMAIL, OPERATOR_LABEL,
# AUTHORITY_OIDC_ISSUER, FULCIO_URL, OPERATOR_IDP_ISSUER, POSTGRES_PASSWORD

docker compose config       # validate
docker compose up -d        # start
docker compose ps           # check status
```

## Pre-requisites

- The relay image (`../relay/`) must be built and pushed (or built locally and the image tag swapped to a dev tag).
- The dispatch SPA must be built (`cd ../dispatch && npm install && npm run build`) so `../dispatch/dist/` exists for the bind-mount in `dispatch-web`.
- The operator must have a domain pointed at the host running compose (for ACME).
- The operator must be onboarded with the OHD project's emergency-authority OIDC provider (i.e. their org pubkey is registered with the Fulcio they're going to refresh against).

## Production hardening (TBD; checklist)

Per [`../SPEC.md`](../SPEC.md) "Security" and [`../../spec/docs/components/emergency.md`](../../spec/docs/components/emergency.md) "Security":

- [ ] Postgres `data` volume on a LUKS-encrypted disk.
- [ ] Authority cert key material on an HSM / TPM-backed mount, NOT a plain Docker volume.
- [ ] Roster-sync hook from operator IdP → relay (departed paramedics removed promptly).
- [ ] Backup for `postgres-records-data` (operator's records are subject to regulatory retention).
- [ ] Log shipping for `caddy` access logs and the relay's audit log.
- [ ] Network policy: dispatch console reachable from the operator's office network only (or VPN-gated).

## Implementation TBDs

(See [`../STATUS.md`](../STATUS.md) "What's NOT done — `deploy/`".)

- Pin a real relay image tag once `../relay/` ships.
- Postgres init schema for operator records (placeholder schema in [`../SPEC.md`](../SPEC.md) "Operator-side records").
- Backup tooling.
- LUKS / volume-encryption guidance with concrete commands.
- HSM mount example.

## License

Dual-licensed `Apache-2.0 OR MIT` — see [`../../spec/LICENSE`](../../spec/LICENSE).
