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

## Guiding Rule

The migration should preserve backward compatibility at the edge while making the core cleaner and more reusable.

In short:

- `legacy-proxy` is allowed to be ugly
- `rag-api.rs` and `rag-mcp.rs` are not
