# Migration Plan

## Goal

Build a reusable Rust-based RAG platform split into three deployable components:

- `legacy-proxy`: compatibility layer for LibreChat and other legacy clients
- `rag-api.rs`: canonical REST API for ingestion and retrieval
- `rag-mcp.rs`: canonical remote MCP server for agentic clients

The new system will use:

- Qdrant as the only persistent database
- Redis only for transient caching and locking
- OpenAI-compatible providers only

The key architectural rule is:

- `legacy-proxy` may preserve legacy quirks
- `rag-api.rs` and `rag-mcp.rs` must stay clean and reusable

## Current Surface Being Replaced

From the current Python service, the real public surface is file-centric and small:

- `GET /health`
- `GET /ids`
- `GET /documents?ids=...`
- `DELETE /documents`
- `POST /query`
- `POST /query_multiple`
- `GET /documents/{id}/context`
- `POST /embed`
- `POST /embed-upload`
- `POST /local/embed`
- `POST /text`

The Postgres inspection routes are debug-only and should not be carried forward.

## Topology

```text
LibreChat / legacy clients
        |
        v
   legacy-proxy
        |
        v
     rag-api.rs  <---->  Redis
        |
        v
      Qdrant
        ^
        |
     rag-mcp.rs
```

`rag-api.rs` and `rag-mcp.rs` should share the same core crate and service layer. `rag-mcp.rs` should not call the REST API over HTTP unless isolation is explicitly desired.

## Responsibilities

### `legacy-proxy`

Purpose:

- preserve old route names and old request and response shapes
- translate LibreChat-specific auth and payload semantics into the canonical model
- isolate backward-compatibility logic from the reusable core

Rules:

- no extraction logic
- no chunking logic
- no vector logic
- no direct Qdrant access
- no provider-specific business rules beyond compatibility translation

### `rag-api.rs`

Purpose:

- expose a clean canonical REST API for ingestion and retrieval
- support multiple source types beyond LibreChat file uploads
- provide the operational interface for services that want HTTP integration

### `rag-mcp.rs`

Purpose:

- expose the same core capabilities for modern agent frameworks and IDEs
- use Streamable HTTP MCP transport
- provide tools and resources rather than legacy file-upload route semantics

## Iteration Status (Updated March 11, 2026)

### Completed now

- Rust workspace scaffold for all crates and binaries
- canonical `POST /v1/query` route scaffold with validation and error mapping
- legacy `POST /query` translation route with legacy tuple response shape
- initial runtime service wiring for query flows
- Qdrant repository implementation for:
  - collection and payload index bootstrap
  - upsert
  - filtered search
  - asset chunk fetch and list
  - asset delete
- workspace `cargo check` and `cargo test` passing

### Not completed yet

- runtime wiring from API handlers to real Qdrant + OpenAI embedding client
- canonical ingest and extract endpoints
- legacy embed and text endpoints
- Redis lock and cache integration
- MCP tool/resource implementation
- parity and golden tests against the Python service

## Next Iterations

### Iteration 2: Real Query Pipeline Wiring

Scope:

- wire `rag-api.rs` query flow to real `ChunkRepository` + `EmbeddingClient`
- remove sample query behavior from runtime
- add configurable model and dimension validation

Deliverables:

- production query service implementation
- dependency graph wiring in runtime container
- startup checks for Qdrant collection compatibility

Acceptance criteria:

- `POST /v1/query` executes real embed + vector search path
- query filters enforce `tenant_id` and `namespace`
- query errors map cleanly to API statuses
- integration test proves end-to-end query against local Qdrant

### Iteration 3: Canonical Ingestion and Extract

Scope:

- implement `POST /v1/assets:ingest`
- implement `POST /v1/assets:extract`
- support text and upload sources first

Deliverables:

- ingestion pipeline (extract -> normalize -> chunk -> embed -> upsert)
- extract-only path without vector writes
- deterministic chunk ids and digest behavior

Acceptance criteria:

- ingest writes searchable chunks in Qdrant
- extract returns text without storage side effects
- basic upload and plain-text requests work end-to-end

### Iteration 4: Legacy Proxy Expansion

Scope:

- implement legacy endpoints beyond `/query`
- preserve Python response shapes

Deliverables:

- `/embed`
- `/embed-upload`
- `/text`
- `/documents/{id}/context`
- `/ids`, `/documents`, and `DELETE /documents`

Acceptance criteria:

- route behavior matches legacy contract for status code and response structure
- translation logic stays isolated inside `legacy-compat`

### Iteration 5: Redis Coordination + Robustness

Scope:

- add lock and cache integration where needed
- harden concurrent workflows

Deliverables:

- per-asset ingest/delete locks
- optional query embedding cache
- idempotency key support for ingest operations

Acceptance criteria:

- concurrent writes for the same asset are controlled
- delete vs ingest races are prevented
- cache usage is optional and configurable

### Iteration 6: MCP Capability Rollout

Scope:

- implement MCP tools and resources over Streamable HTTP
- reuse same domain services as REST

Deliverables:

- tools for ingest, extract, query, delete, list
- resources for chunk/context retrieval
- auth and origin validation at MCP boundary

Acceptance criteria:

- MCP calls use the same core logic as REST
- no duplicated business logic in MCP adapter
- smoke test with one MCP client passes

## Core Domain Model

The reusable domain should not be centered only on `file_id`.

Use these canonical concepts:

- `tenant_id`
- `namespace`
- `asset_id`
- `source_type`
- `source_uri`
- `actor_id`
- `asset`
- `chunk`
- `query`
- `ingestion_job`

Legacy compatibility mapping:

- `file_id` -> `asset_id`
- JWT user id or `entity_id` -> `actor_id`
- default LibreChat context -> `namespace`

This keeps the core reusable for other services and backends.

## Supported Source Types

Support these source types as first-class inputs:

- `upload`
- `local_file`
- `website`
- `pdf`
- `image`
- `code`
- `text`
- `ide_buffer`

These need different extraction and chunking behavior, so they should not be treated as one generic file type.

## Ingestion Pipeline

Build ingestion as a shared, pluggable pipeline:

1. source acquisition
2. type detection
3. extraction
4. normalization
5. chunking
6. embedding
7. Qdrant upsert
8. cache invalidation and event hooks if needed

This pipeline must be shared by `rag-api.rs` and `rag-mcp.rs`.

## Extractor Strategy

Use extractor plugins rather than route-specific logic.

Initial extractor families:

- `plain_text_extractor`
- `pdf_extractor`
- `image_extractor`
- `html_website_extractor`
- `code_extractor`

Behavior guidelines:

- PDFs: preserve page metadata and extract text page-by-page
- Images: use OpenAI-compatible vision or OCR-style understanding when available
- Websites: fetch, clean boilerplate, preserve title and canonical URL
- Code: preserve language, file path, repo context, and symbols where possible
- IDE buffers: accept raw content plus metadata without assuming filesystem access

## OpenAI-Compatible Provider Layer

The upstream provider abstraction should support:

- embeddings for indexing and query vectorization
- optional document understanding or vision for images and image-heavy PDFs

Define two interfaces:

- `EmbeddingClient`
- `DocumentUnderstandingClient`

Both must be OpenAI-compatible only.

If vision support is not available in the configured provider, image ingestion should degrade gracefully rather than breaking the whole pipeline.

## Chunking Strategy

Do not use one generic chunker for every modality.

Use source-aware chunkers:

- prose: recursive token-aware chunking
- websites: section-aware chunking by heading or DOM block
- code: syntax-aware or symbol-aware chunking by file and language
- PDFs: page-aware chunking
- images: caption and region-derived semantic chunks if enabled

This is necessary if the service is meant to work well outside the original LibreChat-only context.

## Qdrant Data Model

Qdrant is the only persistent database, so chunk text must live in payload.

Suggested payload fields:

- `tenant_id`
- `namespace`
- `asset_id`
- `source_type`
- `source_uri`
- `actor_id`
- `digest`
- `chunk_index`
- `page`
- `path`
- `language`
- `mime_type`
- `title`
- `text`
- `created_at`
- `tags`

Point id should be deterministic, based on:

- `tenant_id`
- `namespace`
- `asset_id`
- `chunk_index`
- `digest`

That allows idempotent re-indexing and clean overwrite behavior.

### Qdrant Indexing

Create payload indexes immediately for:

- `tenant_id`
- `namespace`
- `asset_id`
- `actor_id`
- `source_type`

Optional indexes for code-heavy use cases:

- `path`
- `language`

## Collection Strategy

Avoid one collection per tenant.

Preferred default:

- one collection per embedding model and dimension family
- tenants and namespaces isolated via payload filters

Example collection naming:

- `chunks_te3_small`
- `chunks_1536`
- `chunks_3072`

Reason:

- collection explosion is operationally expensive
- embedding dimensions must remain consistent within a collection

## Redis Usage

Redis must stay transient and operational only.

Use it for:

- ingestion locks per `(tenant_id, namespace, asset_id)`
- coordination between delete and re-ingest
- short-lived idempotency keys
- optional query embedding cache
- optional website fetch cache

Do not use Redis as a metadata database.

## Canonical REST API

`rag-api.rs` should expose a clean API instead of inheriting legacy path design.

Suggested routes:

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

Suggested ingest request fields:

- `tenant_id`
- `namespace`
- `asset_id`
- `source_type`
- `source`
- `metadata`
- `options`

`source` should support multiple acquisition modes:

- multipart upload
- raw text
- URL
- local path
- IDE buffer payload

## Legacy Proxy Mapping

`legacy-proxy` should translate old routes into the canonical API.

Examples:

- `POST /embed` -> `POST /v1/assets:ingest`
- `POST /embed-upload` -> `POST /v1/assets:ingest`
- `POST /local/embed` -> `POST /v1/assets:ingest` with `source_type=local_file`
- `POST /text` -> `POST /v1/assets:extract`
- `POST /query` -> `POST /v1/query`
- `POST /query_multiple` -> `POST /v1/query:batch`
- `GET /documents?ids=` -> `GET /v1/assets/{asset_id}/chunks`
- `GET /documents/{id}/context` -> `GET /v1/assets/{asset_id}/context`
- `DELETE /documents` -> `DELETE /v1/assets`

This keeps LibreChat stable while the real platform evolves independently.

## MCP Surface

`rag-mcp.rs` should expose capabilities, not legacy HTTP route semantics.

Suggested MCP tools:

- `ingest_asset`
- `extract_asset_text`
- `query_asset`
- `query_assets`
- `delete_assets`
- `list_assets`

Suggested MCP resources:

- `rag://assets/{asset_id}/context`
- `rag://assets/{asset_id}/chunks`
- `rag://assets/{asset_id}/metadata`

Optional MCP prompts:

- `summarize_asset_context`
- `search_code_context`

This makes the MCP interface a first-class surface for IDEs and agentic systems.

## Auth Model

Do not hardcode JWT semantics into the core services.

Instead:

- edge or proxy middleware resolves authentication
- core services receive an auth context

Auth context should include:

- `tenant_id`
- `actor_id`
- `roles`
- allowed namespaces

This keeps the system reusable beyond LibreChat.

## Website and IDE Use Cases

These should be first-class, not add-ons.

### Websites

Support:

- URL ingestion
- URL canonicalization
- fetch with timeout and retry rules
- readable content extraction
- duplicate detection by digest and canonical URL

### IDE and Code

Support:

- raw buffer ingestion
- path and language metadata
- repo and branch metadata when available
- single-file and multi-file asset ingestion
- code-aware chunking and retrieval filters

Important metadata fields for code:

- `repo`
- `branch`
- `path`
- `language`
- `symbol`

## Migration Phases

### Phase 1: Freeze the legacy contract

- capture current route behavior exactly
- write golden tests for legacy request and response compatibility
- treat the existing Python API as the compatibility baseline only

### Phase 2: Build shared Rust core

- define canonical domain types
- define auth context
- define service traits for ingest, query, delete, extract, and list

### Phase 3: Implement Qdrant storage

- deterministic point ids
- payload schema
- filtered search
- delete by asset
- list and fetch operations

### Phase 4: Implement OpenAI-compatible clients

- embeddings support first
- vision or document understanding support second
- timeout, retry, and validation behavior

### Phase 5: Implement `rag-api.rs`

- canonical REST routes
- health and readiness endpoints
- structured errors
- operational configuration

### Phase 6: Implement `legacy-proxy`

- route translation
- auth translation
- legacy response shape preservation
- no domain logic

### Phase 7: Implement `rag-mcp.rs`

- tools and resources over Streamable HTTP MCP
- same core service layer as REST
- no duplicated ingestion or query logic

### Phase 8: Expand source support

- websites
- code and IDE buffers
- image-aware ingestion
- PDF improvements

### Phase 9: Dual-run and cutover

- compare Python responses with `legacy-proxy`
- move LibreChat to `legacy-proxy`
- retire the Python implementation once compatibility is proven

## Explicit Deferrals

To keep v1 focused, defer these unless they become necessary:

- advanced reranking
- hybrid BM25 plus vector search
- long-lived async job orchestration
- binary asset retention beyond ingestion
- per-tenant Qdrant collections
- provider matrix beyond OpenAI-compatible APIs

## Suggested Rust Workspace Layout

```text
/crates/core
/crates/storage-qdrant
/crates/openai-compat
/crates/ingest
/crates/cache-lock
/crates/http-api
/crates/mcp-api
/bins/rag-api.rs
/bins/rag-mcp.rs
/bins/legacy-proxy.rs
```

## Immediate Execution Backlog (Iteration 2 Active)

This section is the implementation checklist for replacing scaffold behavior with the real query path.

### Current Runtime Gap

- `crates/app-runtime/src/lib.rs` still wires `SampleQueryService`
- `crates/openai-compat/src/lib.rs` still returns `CoreError::NotImplemented`
- `crates/http-api/src/lib.rs` and `crates/legacy-compat/src/lib.rs` are wired to service traits, but only query is partially implemented
- `crates/storage-qdrant/src/lib.rs` has repository behavior ready, but is not used in runtime wiring yet

### Iteration 2 File-Level Plan

1. `crates/app-runtime/src/lib.rs`
   - replace `SampleQueryService` with a production `QueryService` implementation
   - inject `ChunkRepository`, `EmbeddingClient`, and optional `QueryCache`
   - keep `IngestService` and `ExtractService` placeholders until Iteration 3
2. `crates/openai-compat/src/lib.rs`
   - implement OpenAI-compatible `/v1/embeddings` client behavior
   - normalize upstream failures into `CoreError::Provider`
3. `crates/http-api/src/lib.rs`
   - keep request validation in adapter layer
   - map provider and storage failures to stable status and error payloads
4. `crates/legacy-compat/src/lib.rs`
   - ensure `POST /query` translation uses the same production query service
   - preserve legacy tuple response shape without leaking canonical schema details
5. `bins/rag-api.rs/src/main.rs`, `bins/legacy-proxy.rs/src/main.rs`, `bins/rag-mcp.rs/src/main.rs`
   - wire shared runtime config and dependency graph once
   - avoid per-binary divergence in provider/storage initialization

### Iteration 2 Done Conditions

- no sample query text appears in runtime query responses
- `POST /v1/query` and legacy `POST /query` both hit the same production query service path
- query path depends on real embedding vectors and Qdrant search
- invalid query and upstream failures map to stable, tested status codes
- one integration test proves end-to-end query flow against local Qdrant

## Runtime Configuration Contract (Draft v1)

### Already Used

- `RAG_SERVER_BIND_ADDR` default `0.0.0.0:8080`
- `RAG_MCP_BIND_ADDR` default `0.0.0.0:8081`
- `LEGACY_PROXY_BIND_ADDR` default `0.0.0.0:8000`
- `LEGACY_DEFAULT_TENANT` default `public`
- `LEGACY_DEFAULT_NAMESPACE` default `librechat`
- `RUST_LOG` service-specific default value per binary

### Required For Real Query Wiring

- `QDRANT_URL` default `http://localhost:6334`
- `QDRANT_API_KEY` optional
- `QDRANT_COLLECTION` default `chunks_te3_small`
- `QDRANT_VECTOR_SIZE` default `1536`
- `OPENAI_BASE_URL` required when real provider mode is enabled
- `OPENAI_API_KEY` required when real provider mode is enabled
- `OPENAI_EMBED_MODEL` default `text-embedding-3-small`
- `OPENAI_TIMEOUT_MS` default `30000`
- `OPENAI_MAX_RETRIES` default `2`
- `QUERY_TOP_K_DEFAULT` default `4`
- `QUERY_TOP_K_MAX` default `20`

### Startup Validation Rules

- fail fast if `QDRANT_VECTOR_SIZE` is not a positive integer
- fail fast if `QUERY_TOP_K_DEFAULT` is zero or greater than `QUERY_TOP_K_MAX`
- fail fast in production mode when OpenAI base URL or API key is missing
- verify Qdrant collection dimension matches configured embedding dimension
- emit startup log that includes selected embedding model and collection name

## Test and Cutover Gates

### Mandatory CI Gates Per Iteration

- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`

### Query Path Gates (Iteration 2)

- adapter tests for canonical `/v1/query` validation and error mapping
- adapter tests for legacy `/query` response shaping and header/env scope resolution
- integration test for real embed + Qdrant search path
- negative integration tests for provider timeout and empty-query validation

### Cutover Readiness Gates (Before Python Retirement)

- golden compatibility suite for all legacy routes is green
- canonical REST ingest/extract/query flows are green in integration tests
- MCP smoke test proves shared service behavior (no duplicate business logic)
- dual-run comparison has no unexplained response-shape differences for agreed traffic

## Guiding Rule

The migration should preserve backward compatibility at the edge while making the core cleaner and more reusable.

In short:

- `legacy-proxy` is allowed to be ugly
- `rag-api.rs` and `rag-mcp.rs` are not

## Documentation & Schema

- **OpenAPI 3 specification**: The canonical HTTP surface documented in `docs/api/openapi.yaml` mirrors the routes currently scaffolded in `crates/http-api` and will be the source of truth for client integration and future code generation.
- **Testing strategy**: `docs/testing/README.md` describes the unit/integration/e2e plan (including the documented embed→query→context flow) so the next contributor knows which layers are covered and what remains.

## Handoff Notes for Next LLM

### Progress
- rust workspace scaffolds and canonical REST + legacy query adapters are wired up, but the runtime still points at `SampleQueryService`. The `ChunkRepository` with deterministic chunk metadata and the Qdrant helpers for collection bootstrapping, upsert, search, and delete are implemented and passing `cargo check`/`cargo test`.
- there is still a placeholder `OpenAI-compatible` provider that returns `CoreError::NotImplemented`, and `legacy-proxy` plus `http-api` still rely on sample query behavior rather than the shared production query path.

### Context
- this document is the migration plan from the existing Python RAG API to a Rust-based trio of services (`legacy-proxy`, `rag-api.rs`, `rag-mcp.rs`), with iteration 2 focused on wiring the real query pipeline while preserving legacy compatibility at the edge.
- the current feature set targets Qdrant as the only persistent store, Redis for transient caches/locks, and OpenAI-compatible providers for embeddings/document understanding.

### Key Decisions
- persist all chunk metadata in Qdrant payloads, indexed by the canonical domain keys (`tenant_id`, `namespace`, `asset_id`, etc.) and grouped by `collection` names that stay consistent within an embedding dimension family; deterministic point IDs come from `(tenant_id, namespace, asset_id, chunk_index, digest)`.
- provider integration is limited to OpenAI-compatible clients; there are two abstractions (`EmbeddingClient` and `DocumentUnderstandingClient`), and configuration defaults to `text-embedding-3-small` with an ability to validate embedding dimension versus collection dimension per the runtime contract.
- legacy compatibility is confined to `legacy-proxy`, which translates old LibreChat/Azure routes into the canonical REST surface (`POST /v1/query`, `/assets:ingest`, etc.) so that the shared core can stay clean.

### Next Steps
1. Complete Iteration 2 by replacing `SampleQueryService` in `crates/app-runtime/src/lib.rs` with the production `QueryService`, wiring it to the `ChunkRepository`, the OpenAI `EmbeddingClient`, and the optional query cache, then propagate the same logic down into both the canonical `/v1/query` handler and the legacy `/query` translator.
2. Implement the real `OpenAI-compatible` provider in `crates/openai-compat/src/lib.rs`, normalizing upstream failures into `CoreError::Provider`, and ensure `crates/http-api/src/lib.rs`/`crates/legacy-compat/src/lib.rs` map those failures to stable API status codes without leaking schema details.
3. After the production query path is live, add integration tests (including negative cases for provider timeouts and empty queries) plus query filters that honor `tenant_id`, `namespace`, and runtime limits; keep monitoring `Runtime Configuration Contract (Draft v1)` to ensure environment validation rules and startup checks are in place.

### References
- [Iteration 2: Real Query Pipeline Wiring](#iteration-2-real-query-pipeline-wiring)
- [OpenAI-Compatible Provider Layer](#openai-compatible-provider-layer)
- [Runtime Configuration Contract (Draft v1)](#runtime-configuration-contract-draft-v1)
- [Core Domain Model](#core-domain-model)
