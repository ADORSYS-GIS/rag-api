# ─── builder ──────────────────────────────────────────────────────────────────
# Single build stage compiles all three binaries so the layer cache is shared.
FROM rust:1-slim-bookworm AS builder

ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests first so dependency layers are cached independently of source.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY bins/ bins/

RUN cargo build --release \
    --bin rag-api-rs \
    --bin rag-mcp-rs \
    --bin legacy-proxy-rs

# ─── base runtime ─────────────────────────────────────────────────────────────
# Shared non-root base used by all three runtime images.
FROM debian:bookworm-slim AS runtime-base

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -r raguser \
    && useradd -r -g raguser -s /sbin/nologin raguser

WORKDIR /app

# ─── rag-api runtime ──────────────────────────────────────────────────────────
FROM runtime-base AS rag-api-runtime

LABEL org.opencontainers.image.title="rag-api"
LABEL org.opencontainers.image.description="Canonical RAG REST API backed by Qdrant"
LABEL org.opencontainers.image.source="https://github.com/ADORSYS-GIS/ai-helm"

COPY --from=builder /app/target/release/rag-api-rs /usr/local/bin/rag-api

RUN chown raguser:raguser /usr/local/bin/rag-api

USER raguser

EXPOSE 8080

ENV RAG_SERVER_BIND_ADDR=0.0.0.0:8080 \
    RUST_LOG=info

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/rag-api", "--help"] || exit 1

ENTRYPOINT ["/usr/local/bin/rag-api"]

# ─── legacy-proxy runtime ─────────────────────────────────────────────────────
FROM runtime-base AS legacy-proxy-runtime

LABEL org.opencontainers.image.title="rag-legacy-proxy"
LABEL org.opencontainers.image.description="LibreChat-compatible RAG proxy backed by Qdrant"
LABEL org.opencontainers.image.source="https://github.com/ADORSYS-GIS/ai-helm"

COPY --from=builder /app/target/release/legacy-proxy-rs /usr/local/bin/legacy-proxy

RUN chown raguser:raguser /usr/local/bin/legacy-proxy

USER raguser

EXPOSE 8000

ENV LEGACY_PROXY_BIND_ADDR=0.0.0.0:8000 \
    RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/legacy-proxy"]

# ─── rag-mcp runtime ──────────────────────────────────────────────────────────
FROM runtime-base AS rag-mcp-runtime

LABEL org.opencontainers.image.title="rag-mcp"
LABEL org.opencontainers.image.description="RAG MCP server (Streamable HTTP) backed by Qdrant"
LABEL org.opencontainers.image.source="https://github.com/ADORSYS-GIS/ai-helm"

COPY --from=builder /app/target/release/rag-mcp-rs /usr/local/bin/rag-mcp

RUN chown raguser:raguser /usr/local/bin/rag-mcp

USER raguser

EXPOSE 8081

ENV RAG_MCP_BIND_ADDR=0.0.0.0:8081 \
    RUST_LOG=info

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD curl -sf http://localhost:8081/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/rag-mcp"]
