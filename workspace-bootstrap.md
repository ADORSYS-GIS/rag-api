# Workspace Bootstrap Plan

## Purpose

This document defines the initial Rust workspace layout, crate boundaries, trait design, and startup structure for:

- `legacy-proxy`
- `rag-api.rs`
- `rag-mcp.rs`

The objective is to keep the core reusable while making the adapters thin.

## Initial Workspace Layout

```text
rag.rs/
  Cargo.toml
  rust-toolchain.toml
  .gitignore
  .editorconfig
  clippy.toml
  compose.yaml
  .env.example
  crates/
    core/
    storage-qdrant/
    openai-compat/
    ingest/
    cache-lock/
    http-api/
    mcp-api/
    legacy-compat/
    app-runtime/
  bins/
    rag-api.rs/
    rag-mcp.rs/
    legacy-proxy.rs/
```

## Crate Responsibilities

### `core`

Contains:

- canonical domain types
- auth context
- service traits
- error types
- source and chunking enums
- stable DTOs shared across adapters

Must not contain:

- HTTP framework types
- Qdrant client types
- Redis client types
- OpenAI HTTP client types

### `storage-qdrant`

Contains:

- Qdrant repository implementation
- collection management
- payload mapping
- filtered search and upsert logic

Implements traits defined in `core`.

### `openai-compat`

Contains:

- embeddings client
- optional document-understanding or vision client
- OpenAI-compatible request and response handling
- retries and timeouts

Implements provider traits defined in `core`.

### `ingest`

Contains:

- extractor selection
- extraction pipeline
- normalization
- chunking
- ingestion orchestration
- extraction-only workflows

Depends on `core` plus repository and provider traits, not concrete HTTP adapters.

### `cache-lock`

Contains:

- Redis-based lock service
- idempotency key service
- optional query embedding cache
- optional website fetch cache

Implements cache and locking traits from `core` or a narrow shared interface.

### `http-api`

Contains:

- canonical REST route handlers
- request parsing
- OpenAPI wiring
- auth extraction to canonical auth context
- response mapping

No business logic should live here.

### `mcp-api`

Contains:

- MCP transport wiring
- tool registration
- resource registration
- auth and origin validation at the transport layer
- mapping from MCP calls into shared services

No business logic should live here.

### `legacy-compat`

Contains:

- route translation logic
- legacy request and response shapes
- LibreChat-specific compatibility mapping
- compatibility adapters into the canonical REST service layer

This crate is allowed to be awkward. The rest are not.

### `app-runtime`

Contains:

- configuration loading
- dependency graph assembly
- startup validation
- shared runtime builders
- telemetry initialization

This avoids duplicating bootstrap code across the three binaries.

## Binary Responsibilities

### `bins/rag-api.rs`

Assembles:

- config
- shared services
- canonical REST server

### `bins/rag-mcp.rs`

Assembles:

- config
- shared services
- MCP server

### `bins/legacy-proxy.rs`

Assembles:

- config
- shared services or canonical client facade
- legacy compatibility routes

## Cargo Workspace Skeleton

Suggested root `Cargo.toml`:

```toml
[workspace]
members = [
  "crates/core",
  "crates/storage-qdrant",
  "crates/openai-compat",
  "crates/ingest",
  "crates/cache-lock",
  "crates/http-api",
  "crates/mcp-api",
  "crates/legacy-compat",
  "crates/app-runtime",
  "bins/rag-api.rs",
  "bins/rag-mcp.rs",
  "bins/legacy-proxy.rs",
]
resolver = "2"

[workspace.package]
edition = "2024"
license = "MIT"
version = "0.1.0"

[workspace.dependencies]
anyhow = "1"
async-trait = "0.1"
axum = "0.8"
bytes = "1"
chrono = { version = "0.4", features = ["serde"] }
clap = { version = "4", features = ["derive"] }
config = "0.15"
futures = "0.3"
http = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
uuid = { version = "1", features = ["serde", "v5", "v7"] }
```

Version choices can move later. The boundary layout matters more than exact crate versions.

## Core Trait Design

Start with small, explicit traits.

### Repository Traits

```rust
#[async_trait]
pub trait ChunkRepository {
    async fn upsert_chunks(&self, scope: &Scope, chunks: Vec<ChunkRecord>) -> Result<UpsertSummary, CoreError>;
    async fn delete_asset(&self, scope: &Scope, asset_id: &AssetId) -> Result<DeleteSummary, CoreError>;
    async fn get_asset_chunks(&self, scope: &Scope, asset_id: &AssetId) -> Result<Vec<ChunkRecord>, CoreError>;
    async fn list_assets(&self, scope: &Scope, filter: AssetFilter) -> Result<Vec<AssetSummary>, CoreError>;
    async fn search(&self, scope: &Scope, request: SearchRequest) -> Result<Vec<ScoredChunk>, CoreError>;
}
```

### Provider Traits

```rust
#[async_trait]
pub trait EmbeddingClient {
    async fn embed_texts(&self, model: &str, inputs: &[String]) -> Result<Vec<EmbeddingVector>, CoreError>;
    async fn embed_query(&self, model: &str, input: &str) -> Result<EmbeddingVector, CoreError>;
}

#[async_trait]
pub trait DocumentUnderstandingClient {
    async fn describe_image(&self, request: ImageUnderstandingRequest) -> Result<ImageUnderstandingResult, CoreError>;
}
```

### Locking and Cache Traits

```rust
#[async_trait]
pub trait AssetLockManager {
    async fn acquire_asset_lock(&self, key: AssetLockKey) -> Result<AssetLockGuard, CoreError>;
}

#[async_trait]
pub trait QueryCache {
    async fn get_query_embedding(&self, key: QueryEmbeddingCacheKey) -> Result<Option<EmbeddingVector>, CoreError>;
    async fn put_query_embedding(&self, key: QueryEmbeddingCacheKey, value: EmbeddingVector, ttl_secs: u64) -> Result<(), CoreError>;
}
```

### Ingestion Traits

```rust
#[async_trait]
pub trait IngestService {
    async fn ingest(&self, ctx: RequestContext, req: IngestRequest) -> Result<IngestResponse, CoreError>;
}

#[async_trait]
pub trait ExtractService {
    async fn extract(&self, ctx: RequestContext, req: ExtractRequest) -> Result<ExtractResponse, CoreError>;
}

#[async_trait]
pub trait QueryService {
    async fn query(&self, ctx: RequestContext, req: QueryRequest) -> Result<QueryResponse, CoreError>;
    async fn query_batch(&self, ctx: RequestContext, req: BatchQueryRequest) -> Result<BatchQueryResponse, CoreError>;
}
```

## Canonical Core Types

Minimum types to define first:

- `TenantId`
- `Namespace`
- `AssetId`
- `ActorId`
- `RequestContext`
- `Scope`
- `SourceType`
- `AssetSource`
- `IngestRequest`
- `ExtractRequest`
- `QueryRequest`
- `BatchQueryRequest`
- `ChunkRecord`
- `ScoredChunk`
- `AssetSummary`
- `CoreError`

Suggested `SourceType`:

```rust
pub enum SourceType {
    Upload,
    LocalFile,
    Website,
    Pdf,
    Image,
    Code,
    Text,
    IdeBuffer,
}
```

## Request Context Model

All adapters should convert transport auth into this shape before entering services.

```rust
pub struct RequestContext {
    pub tenant_id: TenantId,
    pub actor_id: Option<ActorId>,
    pub roles: Vec<String>,
    pub allowed_namespaces: Vec<Namespace>,
    pub request_id: String,
}
```

This avoids baking JWT semantics into the domain.

## Qdrant Payload Mapping

Suggested persisted chunk shape:

```rust
pub struct ChunkRecord {
    pub tenant_id: TenantId,
    pub namespace: Namespace,
    pub asset_id: AssetId,
    pub actor_id: Option<ActorId>,
    pub source_type: SourceType,
    pub source_uri: Option<String>,
    pub digest: String,
    pub chunk_index: u32,
    pub page: Option<u32>,
    pub path: Option<String>,
    pub language: Option<String>,
    pub mime_type: Option<String>,
    pub title: Option<String>,
    pub text: String,
    pub tags: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
```

Point id should be derived deterministically from scope and chunk identity.

## Extraction Pipeline Modules

Suggested `ingest` crate module layout:

```text
crates/ingest/src/
  lib.rs
  orchestrator.rs
  normalize.rs
  detect.rs
  source/
    mod.rs
    upload.rs
    local_file.rs
    website.rs
    ide_buffer.rs
  extractors/
    mod.rs
    plain_text.rs
    pdf.rs
    image.rs
    website.rs
    code.rs
  chunkers/
    mod.rs
    prose.rs
    pdf.rs
    website.rs
    code.rs
```

## Canonical REST Route Layout

Suggested `http-api` modules:

```text
crates/http-api/src/
  lib.rs
  router.rs
  auth.rs
  errors.rs
  routes/
    health.rs
    ingest.rs
    extract.rs
    query.rs
    assets.rs
```

Suggested canonical route contract:

- `POST /v1/assets:ingest`
- `POST /v1/assets:extract`
- `POST /v1/query`
- `POST /v1/query:batch`
- `GET /v1/assets`
- `GET /v1/assets/{asset_id}/chunks`
- `GET /v1/assets/{asset_id}/context`
- `DELETE /v1/assets`
- `GET /healthz`
- `GET /readyz`

## MCP Route Layout

Suggested `mcp-api` modules:

```text
crates/mcp-api/src/
  lib.rs
  server.rs
  auth.rs
  tools.rs
  resources.rs
  prompts.rs
```

Initial tool set:

- `ingest_asset`
- `extract_asset_text`
- `query_asset`
- `query_assets`
- `delete_assets`
- `list_assets`

Initial resources:

- `rag://assets/{asset_id}/context`
- `rag://assets/{asset_id}/chunks`
- `rag://assets/{asset_id}/metadata`

## Legacy Compatibility Modules

Suggested `legacy-compat` modules:

```text
crates/legacy-compat/src/
  lib.rs
  router.rs
  auth.rs
  models.rs
  translate/
    ingest.rs
    query.rs
    assets.rs
    responses.rs
```

This crate should own all mappings from the old Python API into canonical requests.

## Runtime Assembly

Suggested `app-runtime` responsibilities:

- load config from env and files
- build tracing
- build Qdrant client
- build Redis client
- build OpenAI-compatible client
- build repositories and services
- expose shared app state builders for the three binaries

Suggested top-level runtime constructor:

```rust
pub struct AppContainer {
    pub ingest_service: Arc<dyn IngestService>,
    pub extract_service: Arc<dyn ExtractService>,
    pub query_service: Arc<dyn QueryService>,
    pub chunk_repository: Arc<dyn ChunkRepository>,
}
```

## Configuration Layout

Suggested configuration sections:

- `server`
- `auth`
- `qdrant`
- `redis`
- `openai_compat`
- `ingest`
- `chunking`
- `features`
- `telemetry`

Example environment variable groups:

- `RAG_SERVER__BIND_ADDR`
- `RAG_QDRANT__URL`
- `RAG_QDRANT__API_KEY`
- `RAG_REDIS__URL`
- `RAG_OPENAI_COMPAT__BASE_URL`
- `RAG_OPENAI_COMPAT__API_KEY`
- `RAG_OPENAI_COMPAT__EMBEDDING_MODEL`
- `RAG_FEATURES__ENABLE_VISION`

## Local Development Bootstrap

Initial `compose.yaml` should include:

- Qdrant
- Redis
- optional mock OpenAI-compatible service for tests

Suggested developer commands:

- `cargo check --workspace`
- `cargo test --workspace`
- `cargo run -p rag-api-rs`
- `cargo run -p rag-mcp-rs`
- `cargo run -p legacy-proxy-rs`

## First Implementation Slice

Bootstrap the smallest useful slice in this order:

1. workspace skeleton
2. core types and traits
3. Qdrant repository with text-only chunks
4. embedding client
5. text-only ingest and query service
6. canonical REST ingest and query endpoints
7. MCP query and context resources
8. legacy `/query`, `/embed`, `/text`, and `/documents/{id}/context` mapping

This gives an end-to-end vertical slice before adding PDFs, websites, images, and code-aware chunking.

## Testing Strategy

At bootstrap, create these test layers:

- unit tests for core contracts and translators
- integration tests for Qdrant repository
- integration tests for REST handlers
- smoke tests for MCP transport
- golden tests for legacy compatibility responses

## Non-Negotiable Boundaries

Keep these rules from day one:

- no business logic in HTTP or MCP adapters
- no direct Qdrant usage from `legacy-compat`
- no JWT-specific types in `core`
- no provider-specific code outside `openai-compat`
- no modality-specific parsing inside route handlers
