# Deploying CORD (cord.ohd.dev)

CORD runs as the `ohd-cord` service in `deploy/host/docker-compose.yml`,
behind Caddy at `cord.ohd.dev`. The service sits in the `cord` compose
profile, so a plain `docker compose up` does not start it — it needs config
and secrets first.

## 1. DNS

Add an A record: `cord.ohd.dev` → the Hetzner host's IPv4. Caddy issues the
TLS certificate on first start (HTTP-01 challenge), so the record must
exist beforehand.

## 2. Config

Copy `cord/deploy/cord.toml.example` to `deploy/host/cord.toml` (the compose
service mounts `./cord.toml`). For a non-OHD deployment, change the
`[[auth.provider]]` issuer / client id to the real identity provider.

## 3. Secrets

Set these in `deploy/host/.env` (never commit it):

| Variable | What |
|---|---|
| `OHD_CORD_SESSION_SECRET` | random 32+ char string — HS256 session signing |
| `OHD_CORD_DATA_KEY` | base64 of 32 random bytes — seals share tokens + BYO keys at rest. Generate with `openssl rand -base64 32` |
| `OHD_CORD_OIDC_OHD_SECRET` | the OIDC client secret for `client_id = cord-web` |
| `OHD_CORD_ANTHROPIC_KEY` | the server-side Anthropic API key |

The OIDC provider must allow the redirect URI
`https://cord.ohd.dev/v1/auth/callback`.

## 4. Bring it up

```sh
cd /opt/ohd
docker compose -f deploy/host/docker-compose.yml --profile cord up --build -d
```

## 5. Verify

- `curl https://cord.ohd.dev/healthz` → `{"status":"ok",...}`
- Open `https://cord.ohd.dev/` → the SPA login screen lists the OIDC
  provider.
- Sign in, connect a data source, start a chat.

## Note — the data key is not recoverable

`OHD_CORD_DATA_KEY` cannot be recovered. If it is lost or changed, every
stored share token and BYO model key becomes undecryptable and every user
must reconnect their data sources. Back it up.
