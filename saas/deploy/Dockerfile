# Multi-stage build: chiseled debian for the final image so the binary
# runs with libsqlite (bundled) and rustls TLS without dragging an apt
# update along.

FROM rust:1.89-slim AS build
WORKDIR /src

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy the whole repo subset we need. Cargo workspace lives at the
# repo root but ohd-saas is standalone (its own Cargo.toml), so a
# directory copy is enough.
COPY saas /src/saas
WORKDIR /src/saas
RUN cargo build --release --bin ohd-saas

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /srv
COPY --from=build /src/saas/target/release/ohd-saas /usr/local/bin/ohd-saas

ENV OHD_SAAS_BIND=0.0.0.0:8444
ENV OHD_SAAS_DB=/var/lib/ohd-saas/ohd-saas.db
RUN mkdir -p /var/lib/ohd-saas

EXPOSE 8444
CMD ["/usr/local/bin/ohd-saas"]
