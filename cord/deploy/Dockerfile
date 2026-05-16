# Multi-stage build for the CORD web service. Builds the cord-web SPA and
# the cord-server binary, then assembles a trixie-slim runtime image that
# serves both (cord-server serves the SPA when `web_dir` is configured).

# --- SPA build -------------------------------------------------------------
FROM node:22-slim AS web
WORKDIR /web
# `npm ci` wipes and reinstalls node_modules from the lockfile, so a stale
# host node_modules in the copied context does not matter.
COPY cord/web/ ./
RUN npm ci && npm run build

# --- server build ----------------------------------------------------------
FROM rust:1.89-slim AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# cord is its own cargo workspace. cord-agent path-deps the shared
# storage crates (ohd-relay-client, ohd-h3-helpers) and the cord-relay-e2e
# workspace member path-deps the relay crate, so the build context needs
# all three trees — `..//..` from cord/crates/* resolves to /src/.
COPY cord /src/cord
COPY storage /src/storage
COPY relay /src/relay
WORKDIR /src/cord
RUN cargo build --release --bin cord-server

# --- runtime ---------------------------------------------------------------
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /srv
COPY --from=build /src/cord/target/release/cord-server /usr/local/bin/cord-server
COPY --from=web /web/dist /srv/cord-web

ENV OHD_CORD_CONFIG=/etc/ohd-cord/cord.toml
RUN mkdir -p /var/lib/ohd-cord /etc/ohd-cord

EXPOSE 8446
CMD ["/usr/local/bin/cord-server"]
