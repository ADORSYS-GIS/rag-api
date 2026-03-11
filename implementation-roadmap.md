# Implementation Roadmap

## Purpose

This document turns the migration plan into a delivery sequence with milestones, acceptance criteria, and explicit cut boundaries.

The target deliverables are:

- `legacy-proxy`
- `rag-api.rs`
- `rag-mcp.rs`
- shared reusable Rust core

## Delivery Strategy

Build from the inside out:

1. define the reusable domain
2. implement storage and provider integrations
3. expose canonical REST
4. expose canonical MCP
5. add the legacy compatibility layer
6. expand modality support

This order prevents the proxy from shaping the core architecture.

## Milestone 0: Project Skeleton

### Scope

Create the initial Rust workspace and repo conventions.

### Deliverables

- Cargo workspace
- crate layout
- formatting and lint configuration
- CI baseline
- local development setup with Qdrant and Redis
- environment variable contract draft

### Acceptance Criteria

- `cargo check` passes for all crates
- `cargo fmt --check` passes
- `cargo clippy --workspace --all-targets -- -D warnings` passes
- local `docker compose` can boot Qdrant and Redis
- README describes how to run the workspace

### Out of Scope

- no real business logic
- no HTTP routes beyond placeholder health endpoints

## Milestone 1: Canonical Domain and Contracts

### Scope

Define the reusable model that both REST and MCP will share.

### Deliverables

- canonical types for auth, assets, chunks, queries, and source types
- trait boundaries for ingestion, extraction, query, delete, and list
- structured error model
- request and response DTOs for the canonical REST API
- tool request and response types for MCP

### Acceptance Criteria

- domain types compile cleanly without HTTP framework dependencies
- traits can be mocked in unit tests
- error model distinguishes validation, auth, provider, lock, extraction, and storage failures
- canonical models support these source types:
  - `upload`
  - `local_file`
  - `website`
  - `pdf`
  - `image`
  - `code`
  - `text`
  - `ide_buffer`

### Design Notes

At this point, preserve `asset_id` as the canonical equivalent of legacy `file_id`, but do not expose `file_id` in the shared core.

## Milestone 2: Qdrant Storage Layer

### Scope

Implement Qdrant as the only persistent backend.

### Deliverables

- collection management
- payload schema
- deterministic point id generation
- filtered vector search
- fetch chunks by `asset_id`
- delete by `asset_id`
- list assets by scope
- readiness and connectivity checks

### Acceptance Criteria

- chunks are stored with payload text in Qdrant
- search supports filters on:
  - `tenant_id`
  - `namespace`
  - `asset_id`
  - `actor_id`
  - `source_type`
- upsert is idempotent for a re-indexed asset
- delete removes all chunks for an asset
- integration tests run against a real Qdrant container

### Risks

- payload schema drift
- embedding dimension mismatch between collection and provider

### Mitigation

- pin collection names to embedding model and dimension family
- validate dimensions at startup and before writes

## Milestone 3: Redis Locking and Cache Layer

### Scope

Implement transient coordination only.

### Deliverables

- ingestion lock service
- delete-vs-ingest lock coordination
- idempotency key support
- optional query embedding cache
- optional website fetch cache

### Acceptance Criteria

- concurrent ingest of the same `(tenant_id, namespace, asset_id)` is rejected or serialized predictably
- delete cannot race with re-ingest silently
- lock TTLs are configurable
- all Redis usage is optional except where explicitly required by deployment policy

### Out of Scope

- no business metadata persistence in Redis
- no durable job queue in v1

## Milestone 4: OpenAI-Compatible Clients

### Scope

Implement upstream provider integrations.

### Deliverables

- embedding client using OpenAI-compatible `/v1/embeddings`
- optional vision or document-understanding client for image workflows
- timeout and retry policies
- batching support
- provider error normalization

### Acceptance Criteria

- query and indexing both use the same embedding model contract
- batching is configurable
- upstream 4xx and 5xx are mapped into structured internal errors
- provider base URL and API key are configurable
- integration tests can run against a mocked OpenAI-compatible endpoint

### Cut Decision

If image support is not ready, ship with clear capability flags rather than a partial silent implementation.

## Milestone 5: Extraction and Chunking Pipeline

### Scope

Implement the shared ingestion pipeline and source-aware chunkers.

### Deliverables

- extractor selection by source type and MIME signal
- normalization stage
- chunkers for prose, PDF, website, code, and image-derived text
- ingestion orchestration service
- extraction-only service for text-returning workflows

### Acceptance Criteria

- text files can be ingested end-to-end
- PDFs preserve page metadata
- websites preserve source URL and title metadata
- code ingestion preserves path and language metadata
- extraction-only requests do not write vectors
- pipeline stages are unit-testable independently

### Priority Order

Implement in this order:

1. text
2. PDF text
3. website
4. code and IDE buffers
5. image-aware ingestion

## Milestone 6: `rag-api.rs` Canonical REST Server

### Scope

Build the reusable REST surface.

### Deliverables

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

### Acceptance Criteria

- canonical auth context is available to handlers
- all endpoints return structured JSON errors except explicit text responses
- multipart upload, raw text, URL, and IDE buffer ingestion paths are supported as defined in the API contract
- OpenAPI spec can be generated and reviewed
- integration tests cover ingest, query, get context, delete, and extract-only flows

### Non-Goals

- do not inherit legacy route names
- do not shape the API around LibreChat-specific payload assumptions

## Milestone 7: `rag-mcp.rs` Canonical MCP Server

### Scope

Expose the same capabilities through Streamable HTTP MCP.

### Deliverables

- MCP transport endpoint
- tools:
  - `ingest_asset`
  - `extract_asset_text`
  - `query_asset`
  - `query_assets`
  - `delete_assets`
  - `list_assets`
- resources:
  - `rag://assets/{asset_id}/context`
  - `rag://assets/{asset_id}/chunks`
  - `rag://assets/{asset_id}/metadata`

### Acceptance Criteria

- MCP server uses the same core services as REST
- no duplicated query or ingestion logic exists outside adapters
- streaming responses work for large results where appropriate
- auth and origin validation are enforced at the transport boundary
- at least one IDE or agent client can connect end-to-end in a smoke test

## Milestone 8: `legacy-proxy`

### Scope

Add the compatibility layer after the canonical surfaces are stable.

### Deliverables

- route mapping for the current Python-compatible surface
- legacy request parsing
- legacy response shaping
- LibreChat-focused auth translation
- compatibility test suite based on the current Python API behavior

### Acceptance Criteria

- these routes are supported:
  - `GET /health`
  - `GET /ids`
  - `GET /documents`
  - `DELETE /documents`
  - `POST /query`
  - `POST /query_multiple`
  - `GET /documents/{id}/context`
  - `POST /embed`
  - `POST /embed-upload`
  - `POST /local/embed`
  - `POST /text`
- response shapes are compatible with existing LibreChat expectations
- golden tests compare proxy responses against captured behavior from the Python service

### Non-Goals

- do not expose Qdrant-specific concepts
- do not add direct storage logic to the proxy

## Milestone 9: Cross-Cutting Hardening

### Scope

Raise the quality bar before production cutover.

### Deliverables

- observability
- structured logs
- request tracing
- metrics
- rate limits if needed
- payload size limits
- timeout policy
- configuration validation
- benchmark suite

### Acceptance Criteria

- startup fails fast on invalid configuration
- health and readiness checks differentiate provider, Redis, and Qdrant readiness
- ingest and query paths emit stable metrics
- large-file and concurrent-ingest scenarios are tested
- operational docs exist for deployment and rollback

## Milestone 10: Cutover

### Scope

Replace the Python service in production safely.

### Deliverables

- dual-run comparison environment
- traffic replay or shadowing strategy
- rollback procedure
- migration checklist

### Acceptance Criteria

- legacy-proxy behavior matches the Python service for the agreed route set
- LibreChat works against the proxy without client changes
- canonical REST and MCP are both exercised by at least one non-LibreChat consumer
- rollback to the Python stack is documented and tested

## Acceptance Matrix By Component

### Shared Core

Must have:

- no HTTP framework types in domain services
- deterministic ids
- source-aware extraction and chunking
- testable service traits

### `rag-api.rs`

Must have:

- canonical REST only
- OpenAPI generation
- upload, text, URL, and IDE buffer ingest paths
- health and readiness endpoints

### `rag-mcp.rs`

Must have:

- Streamable HTTP MCP transport
- tools and resources backed by shared services
- no REST-to-MCP network hop in-process by default

### `legacy-proxy`

Must have:

- protocol and shape compatibility only
- zero direct domain ownership
- zero storage ownership

## Recommended Execution Order

Implement in this exact order:

1. workspace and CI skeleton
2. domain contracts
3. Qdrant storage
4. Redis lock and cache layer
5. embeddings client
6. extraction and chunking pipeline
7. canonical REST server
8. canonical MCP server
9. legacy proxy
10. hardening and cutover

## Explicit Deferred Items

These should not block v1:

- hybrid lexical plus vector search
- reranking layer
- asynchronous durable job queue
- long-term binary storage
- multi-provider support beyond OpenAI-compatible APIs
- per-tenant collection sharding
