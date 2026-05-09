# Multi-stage build for ohd-relay.
#
# Build context is the **project root** (`/`) so the build can reach the
# shared `ohd-h3-helpers` crate at `storage/crates/ohd-h3-helpers/` (the
# relay path-deps it for QUIC + cert helpers).
#
# Build:
#   docker build -t ohd-relay:dev -f relay/deploy/Dockerfile .
# Or via the host compose:
#   docker compose -f deploy/host/docker-compose.yml up --build -d
#
# Build stage
FROM rust:1.88-slim-bookworm AS build

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential \
      pkg-config \
      libssl-dev \
      perl \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Bring in the relay sources + the path-dep'd shared crate. Project-root
# build context lets us COPY across the storage/relay boundary.
COPY relay/Cargo.toml relay/Cargo.lock relay/rust-toolchain.toml /src/relay/
COPY relay/src /src/relay/src
COPY storage/crates/ohd-h3-helpers /src/storage/crates/ohd-h3-helpers

WORKDIR /src/relay

# Plain forwarding relay by default. Set BUILD_FEATURES=authority to build
# the emergency-authority variant.
ARG BUILD_FEATURES=""
RUN if [ -n "$BUILD_FEATURES" ]; then \
        cargo build --release --features "$BUILD_FEATURES"; \
    else \
        cargo build --release; \
    fi

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --create-home --uid 10001 ohdrelay \
 && mkdir -p /var/lib/ohd-relay /etc/ohd-relay \
 && chown -R ohdrelay:ohdrelay /var/lib/ohd-relay /etc/ohd-relay

COPY --from=build /src/relay/target/release/ohd-relay /usr/local/bin/ohd-relay

USER ohdrelay
WORKDIR /var/lib/ohd-relay

EXPOSE 8443/tcp
EXPOSE 9001/udp

ENTRYPOINT ["/usr/local/bin/ohd-relay"]
CMD ["serve", "--bind", "0.0.0.0:8443", "--db", "/var/lib/ohd-relay/relay.db", "--quic-tunnel-listen", "0.0.0.0:9001"]
