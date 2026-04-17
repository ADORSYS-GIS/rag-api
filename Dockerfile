# Multi-stage build for Rust application
FROM rustlang/rust:nightly-slim AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY bins/ bins/

RUN cargo build --release \
    --bin rag-api-rs \
    --bin rag-mcp-rs \
    --bin legacy-proxy-rs

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/rag-api-rs /app/rag-api
COPY --from=builder /app/target/release/rag-mcp-rs /app/rag-mcp
COPY --from=builder /app/target/release/legacy-proxy-rs /app/legacy-proxy

RUN useradd -r -s /bin/false raguser && chown raguser:raguser /app/rag-api /app/rag-mcp /app/legacy-proxy
USER raguser

EXPOSE 8000 8080 8081

ENV RAG_SERVER_BIND_ADDR=0.0.0.0:8080

CMD ["/app/rag-api"]
