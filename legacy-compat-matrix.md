# Legacy Compatibility Matrix

## Purpose

This matrix defines how `legacy-proxy` maps the old Python `rag_api` surface into:

- canonical REST (`rag-api.rs`)
- canonical MCP (`rag-mcp.rs`)

It is the implementation contract for compatibility behavior.

## Mapping Table

| Legacy Route | Method | Canonical REST Target | Canonical MCP Equivalent | Notes |
|---|---|---|---|---|
| `/health` | `GET` | `GET /healthz` | transport health check | Return `{"status":"UP"}` for legacy clients. |
| `/ids` | `GET` | `GET /v1/assets` | `list_assets` tool | Legacy expects list of ids only; proxy extracts `asset_id`. |
| `/documents?ids=...` | `GET` | `GET /v1/assets/{asset_id}/chunks` (fan-out) | `rag://assets/{asset_id}/chunks` resource or `query_assets` tool | Proxy aggregates chunk records and returns legacy shape. |
| `/documents` | `DELETE` | `DELETE /v1/assets` | `delete_assets` tool | Legacy body is list of ids; canonical is structured delete request. |
| `/query` | `POST` | `POST /v1/query` | `query_asset` tool | Proxy translates `{query,file_id,k,entity_id}` into canonical request scope and filters. |
| `/query_multiple` | `POST` | `POST /v1/query:batch` | `query_assets` tool | Legacy body `{query,file_ids,k}` maps to canonical batch query. |
| `/documents/{id}/context` | `GET` | `GET /v1/assets/{asset_id}/context` | `rag://assets/{asset_id}/context` resource | Legacy response is plain text; preserve exactly. |
| `/embed` | `POST` multipart | `POST /v1/assets:ingest` | `ingest_asset` tool | Legacy fields `file_id`, `entity_id`, `file` map to canonical ingest payload. |
| `/embed-upload` | `POST` multipart | `POST /v1/assets:ingest` | `ingest_asset` tool | Same canonical target as `/embed`; preserve legacy response keys. |
| `/local/embed` | `POST` JSON | `POST /v1/assets:ingest` | `ingest_asset` tool | Source type becomes `local_file`. |
| `/text` | `POST` multipart | `POST /v1/assets:extract` | `extract_asset_text` tool | Extract-only path; no vector writes. |

## Request Translation Rules

### Auth and Identity

- If legacy JWT is present and valid:
  - map token subject or id to `actor_id`
  - derive `tenant_id` from configured policy
- If legacy `entity_id` is present:
  - map to canonical override field used for scope or actor selection
- If no auth exists and anonymous mode is enabled:
  - assign configured anonymous identity and namespace

### Scope Derivation

- Legacy `file_id` maps to canonical `asset_id`
- Default namespace for legacy traffic: `librechat` unless configured otherwise
- Default tenant for legacy traffic: configured `LEGACY_DEFAULT_TENANT`

### Source Mapping

- `/embed` and `/embed-upload`: `source_type=upload`
- `/local/embed`: `source_type=local_file`
- `/text`: extraction request only, no ingestion side effects

## Response Shaping Rules

### Strict Legacy Shapes

`legacy-proxy` must preserve these outputs:

- `GET /ids` -> `string[]`
- `GET /documents` -> `[{ page_content, metadata }]`
- `POST /query` -> `[[{ page_content, metadata }, score], ...]`
- `POST /query_multiple` -> `[[{ page_content, metadata }, score], ...]`
- `GET /documents/{id}/context` -> text body
- `POST /embed` and `/embed-upload` -> `{ status, message, file_id, filename, known_type }`
- `POST /local/embed` -> `{ status, file_id, filename, known_type }`
- `POST /text` -> `{ text, file_id, filename, known_type }`

### Error Compatibility

When possible, preserve legacy status semantics:

- `400` validation or file-processing errors
- `401` auth errors
- `404` not found for missing ids
- `500` internal processing failures

Proxy-internal failures that do not map cleanly should default to:

- `500` with legacy-compatible `detail` message

## Known Legacy Quirks To Preserve Initially

- Legacy query response tuple shape `[[document, score]]` instead of object-based matches
- Legacy health body uses `{"status":"UP"}` and not canonical health schema
- Legacy routes mix JSON, multipart, and plain-text outputs

These quirks should stay in `legacy-proxy` only.

## Behavior Differences Allowed In Canonical Services

The following improvements are allowed in canonical APIs and must be hidden by the proxy when needed:

- structured error envelopes
- typed query responses instead of tuple arrays
- richer scope and tenant controls
- stricter validation rules

## Test Matrix For `legacy-proxy`

Required compatibility tests:

1. `GET /health` returns `200` and legacy body shape.
2. `GET /ids` returns list of strings only.
3. `GET /documents` round-trips legacy chunk shape.
4. `DELETE /documents` accepts legacy body list and returns legacy message shape.
5. `POST /query` returns tuple-array format.
6. `POST /query_multiple` returns tuple-array format.
7. `GET /documents/{id}/context` returns text body.
8. `POST /embed` handles multipart and preserves response keys.
9. `POST /embed-upload` handles multipart and preserves response keys.
10. `POST /local/embed` accepts JSON and maps to `local_file` source.
11. `POST /text` extracts text without vector writes.
12. JWT and no-JWT modes follow legacy auth behavior.

## Implementation Priority

Implement proxy mappings in this order:

1. `/health`
2. `/query`
3. `/embed`
4. `/text`
5. `/documents/{id}/context`
6. `/ids`
7. `/documents` get and delete
8. `/embed-upload`
9. `/query_multiple`
10. `/local/embed`

This order delivers the highest-value LibreChat compatibility path first.
