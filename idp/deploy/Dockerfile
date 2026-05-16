# Multi-stage build for OHD Identity (the OIDC provider). Builds the
# ohd-idp binary, then assembles a slim trixie runtime image. The binary
# runs with rustls TLS (via reqwest in later phases) and needs no apt
# packages beyond CA certificates.

# --- server build ----------------------------------------------------------
FROM rust:1.89-slim AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# idp is its own self-contained cargo workspace — no path-deps outside
# its tree — so copying the directory is enough.
COPY idp /src/idp
WORKDIR /src/idp
RUN cargo build --release --bin ohd-idp

# --- runtime ---------------------------------------------------------------
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /srv
COPY --from=build /src/idp/target/release/ohd-idp /usr/local/bin/ohd-idp

ENV OHD_IDP_CONFIG=/etc/ohd-idp/idp.toml
RUN mkdir -p /var/lib/ohd-idp /etc/ohd-idp

EXPOSE 8447
CMD ["/usr/local/bin/ohd-idp"]
