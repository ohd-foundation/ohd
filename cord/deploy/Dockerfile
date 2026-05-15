# Multi-stage build for cord-server. trixie-slim runtime so the binary's
# GLIBC matches the build image (same fix applied to ohd-mcp-rs).

FROM rust:1.89-slim AS build
WORKDIR /src

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# cord is its own cargo workspace, standalone from the repo-root workspace.
COPY cord /src/cord
WORKDIR /src/cord
RUN cargo build --release --bin cord-server

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /srv
COPY --from=build /src/cord/target/release/cord-server /usr/local/bin/cord-server

ENV OHD_CORD_CONFIG=/etc/ohd-cord/cord.toml
RUN mkdir -p /var/lib/ohd-cord /etc/ohd-cord

EXPOSE 8446
CMD ["/usr/local/bin/cord-server"]
