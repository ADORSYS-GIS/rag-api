# rag.rs

Rust workspace for:

- `legacy-proxy` (LibreChat-compatible edge adapter)
- `rag-api.rs` (canonical REST service)
- `rag-mcp.rs` (canonical MCP service)

## Quick Start

1. Start dependencies:

```bash
docker compose -f compose.yaml up -d
```

2. Run checks:

```bash
cargo check --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

3. Run services:

```bash
cargo run -p rag-api-rs
cargo run -p rag-mcp-rs
cargo run -p legacy-proxy-rs
```
