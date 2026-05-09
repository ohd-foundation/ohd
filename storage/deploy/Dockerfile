# Multi-stage build for ohd-storage-server.
#
# Build stage: pulls the workspace, compiles the server binary in release mode.
# Runtime stage: minimal Debian slim with the binary + ca-certificates.
#
# rusqlite uses `bundled-sqlcipher` (per workspace Cargo.toml), which builds
# OpenSSL + SQLCipher from source — slower first build, but no system libs
# needed at runtime.
#
# Build:
#   # From the storage/ dir, when the storage workspace is self-contained:
#   docker build -t ohd-storage:dev -f deploy/Dockerfile ./..
#   # Or via the host compose (project-root context):
#   docker compose -f deploy/host/docker-compose.yml up --build
#
# Build context is the **project root** so we can `COPY storage/...`.
FROM rust:1.88-slim-bookworm AS build

# protoc bundled by `protoc-bin-vendored` build dep — no apt protobuf-compiler
# needed. build-essential + pkg-config + libssl-dev are required for the
# bundled-sqlcipher OpenSSL build.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential \
      pkg-config \
      libssl-dev \
      perl \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Copy the storage workspace. Order chosen for Docker layer cache:
# manifests change less often than sources.
COPY storage/Cargo.toml storage/Cargo.lock storage/rust-toolchain.toml \
     storage/buf.yaml storage/buf.gen.yaml ./
COPY storage/proto ./proto
COPY storage/migrations ./migrations
COPY storage/crates ./crates

RUN cargo build --release -p ohd-storage-server

# --- Runtime stage ---
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates wget \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --create-home --uid 10001 ohd \
 && mkdir -p /var/lib/ohd-storage /etc/ohd-storage \
 && chown -R ohd:ohd /var/lib/ohd-storage /etc/ohd-storage

COPY --from=build /src/target/release/ohd-storage-server /usr/local/bin/ohd-storage-server

USER ohd
WORKDIR /var/lib/ohd-storage

# HTTP/2 (TCP) — primary OHDC transport today.
EXPOSE 8443/tcp
# HTTP/3 (QUIC over UDP) — exposed in advance of the HTTP/3 listener landing.
EXPOSE 8443/udp

VOLUME ["/var/lib/ohd-storage"]

ENTRYPOINT ["/usr/local/bin/ohd-storage-server"]
CMD ["serve", "--db", "/var/lib/ohd-storage/storage.db", "--listen", "0.0.0.0:8443"]
