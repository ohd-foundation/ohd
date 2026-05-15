FROM rust:1.89-slim AS build
WORKDIR /src

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev clang lld \
    perl make \
    && rm -rf /var/lib/apt/lists/*

# We compile from inside the workspace so the path-dep crates
# (ohd-storage-core, ohd-mcp-core) resolve. Build context is the repo root.
COPY storage /src/storage
COPY connect/mcp-rs /src/connect/mcp-rs
WORKDIR /src/connect/mcp-rs
RUN cargo build --release --bin ohd-mcp-rs

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /srv
COPY --from=build /src/connect/mcp-rs/target/release/ohd-mcp-rs /usr/local/bin/ohd-mcp-rs

ENV OHD_MCP_BIND=0.0.0.0:8445
ENV OHD_MCP_STORAGE=/var/lib/ohd-mcp/ohd-mcp.ohd
RUN mkdir -p /var/lib/ohd-mcp

EXPOSE 8445
CMD ["/usr/local/bin/ohd-mcp-rs", "http"]
