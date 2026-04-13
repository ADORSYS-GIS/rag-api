# Testing Surface

This directory explains how to cover the migration with unit, integration, and end-to-end tests (the `unit+IT+e2e` directive mentioned earlier).

## Unit tests (`cargo test`)

- **`crates/core`**: focus on domain types, serialization, `ChunkRecord` helpers, and error conversions. These are pure-Rust and should remain fast (`cargo test -p rag-core`).
- **`crates/storage-qdrant` / `crates/cache-lock`**: mock the Qdrant client and ensure deterministic point ids, payload shaping, and lock coordination (`cargo test -p storage-qdrant` etc.).
- **`crates/openai-compat`**: use `#[cfg(test)]` mocks to exercise `EmbeddingClient` error normalization without hitting real providers.
- **`crates/http-api` and `crates/mcp-api`**: the existing `#[tokio::test]` coverage targets request validation, response shaping, and the placeholder error paths. Expand with more cases as needed.

Unit tests should run continuously in CI (`cargo test --workspace`) and remain isolated from external services.

## Integration tests (`cargo test --workspace --features=integration`)

Integration coverage sits one level above the units:

- Launch the `rag-api.rs` binary with dummy providers using the runtime wiring in `crates/app-runtime`. Use a `TestConfig` fixture (similar to the existing `SampleQueryService`) to swap in deterministic repositories and caches. Exercise the full `/v1/query` and `/v1/assets` routes via `tower::ServiceExt::ready` and `request::Builder`.
- Verify legacy request translation by instantiating `legacy-proxy` with the same seeded services, then driving `POST /query`/`POST /embed`.
- Use the configuration validation logic from `Runtime Configuration Contract (Draft v1)` to ensure the integration build fails fast on missing values.

These suites can be grouped under an `integration` feature flag and invoked with `cargo test --workspace --features=integration`.

## End-to-end (E2E) regression scenarios

E2E scenarios target the entire stack (services, docs, and configuration) once real dependencies are available.

### Documented e2e flow
1. Start a local Qdrant instance (default `QDRANT_URL=http://localhost:6334`) and ensure `QDRANT_COLLECTION` matches the configured embedding dimension.
2. Run `cargo run --bin rag-api` with `OPENAI_BASE_URL=https://api.openai.com` (or a compatible mock) and `OPENAI_API_KEY` set.
3. Using the OpenAPI definition in `docs/api/openapi.yaml`, issue:
   - `POST /v1/assets:ingest` with sample `tenant_id`, `namespace`, and `asset_id` plus `content`.
   - `POST /v1/query` against that `asset_id` and assert the returned `matches` include `chunk.text`.
   - `GET /v1/assets/{asset_id}/context` (once implemented) to confirm context retrieval.
4. Repeat the exercise via `legacy-proxy` to validate legacy-polyfill headers and JSON shapes.

Automate this workflow with shell scripts or CI jobs (`curl` or `httpie` against the OAS3 spec) once the pipes are fully wired (see `docs/api/openapi.yaml` for exact payloads). Capture results as golden files to later gate Python retirement.
