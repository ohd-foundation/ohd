# OHD Relay — Deployment

Reference Docker Compose + Caddy stack for `ohd-relay`. Caddy fronts the relay on `:443` (HTTP/2 + HTTP/3) and terminates outer TLS; the relay process listens on `:8443` inside the network. End-to-end TLS between consumer and storage rides through the tunnel and is invisible to both Caddy and the relay (the relay sees ciphertext only).

## What's in this dir

| File | Purpose |
|---|---|
| [`docker-compose.yml`](docker-compose.yml) | Two-service stack: `caddy` + `relay`. |
| [`Caddyfile`](Caddyfile) | Caddy reverse-proxy config, automatic HTTPS via Let's Encrypt. |
| [`Dockerfile`](Dockerfile) | Builds `ohd-relay:0.1.0` from the parent crate. |
| [`relay.example.toml`](relay.example.toml) | Annotated config template. Copy to `relay.toml` and edit. |

## Quick start

```bash
# From this directory:
cp relay.example.toml relay.toml
$EDITOR relay.toml   # set public_host, push providers, optional authority-mode

docker compose up --build -d
docker compose logs -f relay caddy
```

After Caddy picks up the cert (first HTTPS request), the relay is reachable at
`https://<public_host>/`. Verify the inner relay is healthy:

```bash
docker compose exec relay /usr/local/bin/ohd-relay health
```

## Pre-requisites

- A domain pointing at the host (Caddy provisions TLS on first HTTPS request via ACME).
- `relay.toml` with `public_host` set to your domain.
- For emergency-authority mode: build with `--features authority` and provision the OIDC issuer + Fulcio config in `relay.toml`. See [`../spec/emergency-trust.md`](../spec/emergency-trust.md).

## Ports

| Port | Protocol | Purpose |
|---|---|---|
| 80/tcp | HTTP | ACME challenge + redirect to HTTPS |
| 443/tcp | HTTPS / HTTP/2 | Outer TLS terminator (Caddy) |
| 443/udp | HTTP/3 (QUIC) | Same, over QUIC |

The relay process listens internally on `:8443`. The host ports are owned by Caddy.

## Volumes

| Volume | Mount | Purpose |
|---|---|---|
| `relay_data` | `/var/lib/ohd-relay` | Registration SQLite + tunnel state. |
| `caddy_data` | `/data` | Caddy's ACME state + issued certs. Don't lose this — it has your private keys. |
| `caddy_config` | `/config` | Caddy runtime config. |

## Push provider secrets

The relay touches push only on the silent tunnel-wake path. Mount FCM service-account JSON / APNs P8 into the container as needed and reference the paths from `relay.toml`. Never put credentials in `relay.toml` directly. See [`relay.example.toml`](relay.example.toml) for the wiring template.

## Native packages

For bare-metal / VM installs, see [`../../PACKAGING.md`](../../PACKAGING.md). The systemd unit at `../../packaging/systemd/ohd-relay.service` runs the binary as the dedicated `ohd-relay` system user; the data path is the same `/var/lib/ohd-relay` as this Docker compose uses.

## License

Dual-licensed `Apache-2.0 OR MIT` — see [`../../spec/LICENSE`](../../spec/LICENSE).
