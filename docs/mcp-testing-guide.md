# RAG MCP Server — Testing Guide

This guide covers how to run and manually test the `rag-mcp` server and all six of its
tools. It supersedes the earlier legacy-proxy testing guide for anything MCP-related.

---

## 1. Prerequisites

### Stack

You need three things running before any tool call will succeed:

| Service | Default address | Purpose |
|---|---|---|
| Qdrant | `http://localhost:6334` | Vector storage |
| Embedding provider | `http://localhost:11434/v1` (Ollama) or OpenAI | Chunk embeddings |
| `rag-mcp` | `http://localhost:8081` | The MCP server itself |

### Quick start with Docker

```bash
# Start Qdrant
docker run -d --name qdrant -p 6333:6333 -p 6334:6334 qdrant/qdrant

# Start rag-mcp (adjust env vars to match your embedding provider)
docker run -d --name rag-mcp \
  -p 8081:8081 \
  --add-host host.docker.internal:host-gateway \
  -e OPENAI_BASE_URL=http://host.docker.internal:11434/v1 \
  -e OPENAI_EMBED_MODEL=nomic-embed-text \
  -e QDRANT_URL=http://host.docker.internal:6334 \
  -e QDRANT_VECTOR_SIZE=768 \
  -e QDRANT_COLLECTION=chunks_nomic \
  rag-mcp:latest
```

For OpenAI embeddings swap in:
```bash
  -e OPENAI_BASE_URL=https://api.openai.com \
  -e OPENAI_API_KEY=sk-... \
  -e OPENAI_EMBED_MODEL=text-embedding-3-small \
  -e QDRANT_VECTOR_SIZE=1536 \
  -e QDRANT_COLLECTION=chunks_te3_small \
```

### Health check

```bash
curl -s http://localhost:8081/healthz
# → {"status":"ok","service":"rag-mcp"}

curl -s http://localhost:8081/readyz
# → {"status":"ready","service":"rag-mcp"}
```

---

## 2. Connecting a Client

### MCP Inspector (easiest — recommended for manual testing)

```bash
npx @modelcontextprotocol/inspector http://localhost:8081/mcp
```

Open the URL it prints. You get a full UI showing all tools, their input schemas, and
live responses. No JSON wrangling needed.

### Claude Desktop

Add this block to `claude_desktop_config.json`
(`~/Library/Application Support/Claude/` on Mac, `%APPDATA%\Claude\` on Windows):

```json
{
  "mcpServers": {
    "rag": {
      "command": "docker",
      "args": [
        "run", "--rm", "-i",
        "--add-host", "host.docker.internal:host-gateway",
        "-e", "OPENAI_BASE_URL=http://host.docker.internal:11434/v1",
        "-e", "OPENAI_EMBED_MODEL=nomic-embed-text",
        "-e", "QDRANT_URL=http://host.docker.internal:6334",
        "-e", "QDRANT_VECTOR_SIZE=768",
        "-e", "QDRANT_COLLECTION=chunks_nomic",
        "rag-mcp:latest"
      ]
    }
  }
}
```

### Direct HTTP (advanced)

The server uses the Streamable HTTP MCP transport. Each request is a JSON-RPC 2.0
envelope posted to `/mcp`. You must first initialize a session, then issue tool calls
using the session ID returned in the `mcp-session-id` response header.

```bash
# Step 1 — initialize
SESSION=$(curl -s -D - -X POST http://localhost:8081/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"curl-test","version":"1"}}}' \
  | grep -i mcp-session-id | awk '{print $2}' | tr -d '\r')

# Step 2 — call a tool (replace the method/params as needed)
curl -s -X POST http://localhost:8081/mcp \
  -H "Content-Type: application/json" \
  -H "mcp-session-id: $SESSION" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_assets","arguments":{}}}'
```

---

## 3. Tool Reference

### 3.1 `ingest_asset`

Chunks, embeds, and stores content into the vector database.

#### Parameters

| Parameter | Type | Required | Default | Notes |
|---|---|---|---|---|
| `asset_id` | string | ✅ | — | Stable identifier for this document. Reingest with the same ID to overwrite. |
| `content` | string | ✴️ | — | Pre-extracted plain text. Required if `source_uri` is absent. |
| `source_uri` | string | ✴️ | — | File path or `http(s)://` URL to read and extract text from. Required if `content` is absent. |
| `source_type` | string | ❌ | `text` | Metadata tag — see table below. Does **not** change extraction behaviour. |
| `mime_type` | string | ❌ | — | MIME hint stored with each chunk (e.g. `application/pdf`). |
| `tenant_id` | string | ❌ | `public` | Scope isolation. |
| `namespace` | string | ❌ | `default` | Sub-scope within a tenant. |
| `actor_id` | string | ❌ | — | User/actor identifier stored with each chunk. |

✴️ Exactly one of `content` or `source_uri` must be provided.

#### `source_type` values

This is a **metadata tag** that labels how the content originated. It does not control
which extractor runs — that is determined by the file extension or HTTP `Content-Type`
of whatever `source_uri` points to.

| Value | When to use |
|---|---|
| `text` | Plain text content you provide directly in `content`. Default. |
| `upload` | File uploaded by a user (e.g. via a UI). |
| `local_file` | File read from the local filesystem via `source_uri`. |
| `website` | Web page fetched via an `http(s)://` `source_uri`. |
| `pdf` | PDF document, regardless of whether you pass `content` or `source_uri`. |
| `code` | Source code file (any language). |
| `image` | Image file (reserved for future multimodal use). |
| `ide_buffer` | Content streamed live from an IDE buffer. |

#### Extraction behaviour (what `source_uri` actually does)

When `source_uri` is provided, the server resolves the content as follows:

| URI form | What happens | Format detected by |
|---|---|---|
| `/absolute/path` or `./relative/path` | Read from local filesystem | File extension (e.g. `.pdf`, `.html`) |
| `file:///absolute/path` | Same as above after stripping prefix | File extension |
| `http://…` or `https://…` | HTTP GET, body read | `Content-Type` response header |

Supported formats and their extractors:

| Detected MIME | Extractor used |
|---|---|
| `application/pdf` | `pdf-extract` (pure-Rust, no native deps) |
| `text/html` | `scraper` — body text stripped, whitespace normalized |
| Everything else | Read as UTF-8 text (plain text, markdown, JSON, CSV, code, …) |

Maximum source size: **5 MiB**. Requests exceeding this are rejected before extraction.

#### Examples

**Ingest plain text directly:**
```json
{
  "asset_id": "doc-001",
  "content": "Rust is a systems programming language focused on safety and performance.",
  "source_type": "text"
}
```

**Ingest a local plain-text file:**
```json
{
  "asset_id": "notes-2024",
  "source_uri": "/home/user/notes/meeting-2024-05.txt",
  "source_type": "local_file"
}
```

**Ingest a local PDF:**
```json
{
  "asset_id": "report-q1",
  "source_uri": "/home/user/docs/quarterly-report.pdf",
  "source_type": "pdf",
  "mime_type": "application/pdf"
}
```

**Ingest a web page:**
```json
{
  "asset_id": "rust-book-ch4",
  "source_uri": "https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html",
  "source_type": "website"
}
```

**Ingest a code file:**
```json
{
  "asset_id": "main-rs",
  "source_uri": "/home/user/project/src/main.rs",
  "source_type": "code",
  "mime_type": "text/x-rust"
}
```

**Ingest with tenant/namespace scoping:**
```json
{
  "asset_id": "internal-policy",
  "source_uri": "/data/policy.txt",
  "source_type": "upload",
  "tenant_id": "acme-corp",
  "namespace": "hr",
  "actor_id": "user-42"
}
```

**Expected response:**
```json
{
  "result": {
    "asset_id": "doc-001",
    "chunks_written": 3
  }
}
```

---

### 3.2 `extract_asset_text`

Extracts plain text from a source without storing anything in the vector database.
Useful for previewing what would be indexed before committing to an ingest.

#### Parameters

| Parameter | Type | Required | Default | Notes |
|---|---|---|---|---|
| `content` | string | ✴️ | — | Pre-extracted text to pass through. |
| `source_uri` | string | ✴️ | — | File path or URL to extract from. Same rules as `ingest_asset`. |
| `source_type` | string | ❌ | `text` | Metadata hint only. |
| `tenant_id` | string | ❌ | `public` | |
| `namespace` | string | ❌ | `default` | |

✴️ Exactly one of `content` or `source_uri` must be provided.

#### Examples

**Preview extraction from a PDF:**
```json
{
  "source_uri": "/home/user/docs/contract.pdf",
  "source_type": "pdf"
}
```

**Preview a web page:**
```json
{
  "source_uri": "https://example.com/article",
  "source_type": "website"
}
```

**Expected response:**
```json
{
  "result": {
    "text": "Extracted plain text content…"
  }
}
```

---

### 3.3 `query_asset`

Semantic similarity search over a single asset.

#### Parameters

| Parameter | Type | Required | Default | Notes |
|---|---|---|---|---|
| `query` | string | ✅ | — | Natural-language search query. |
| `asset_id` | string | ✅ | — | Asset to search within. |
| `k` | integer | ❌ | `4` | Number of results to return (max `QUERY_TOP_K_MAX`, default 20). |
| `tenant_id` | string | ❌ | `public` | |
| `namespace` | string | ❌ | `default` | |
| `actor_id` | string | ❌ | — | |

#### Example

```json
{
  "query": "what are the ownership rules?",
  "asset_id": "rust-book-ch4",
  "k": 3
}
```

**Expected response:**
```json
{
  "result": {
    "matches": [
      {
        "chunk": {
          "text": "Each value in Rust has an owner…",
          "chunk_index": 2,
          "asset_id": "rust-book-ch4",
          "source_type": "Website",
          "score": 0.91
        }
      }
    ]
  }
}
```

---

### 3.4 `query_assets`

Same as `query_asset` but searches across multiple assets at once. Results are ranked
globally by similarity score.

#### Parameters

| Parameter | Type | Required | Default | Notes |
|---|---|---|---|---|
| `query` | string | ✅ | — | |
| `asset_ids` | string[] | ✅ | — | Must not be empty. |
| `k` | integer | ❌ | `4` | |
| `tenant_id` | string | ❌ | `public` | |
| `namespace` | string | ❌ | `default` | |
| `actor_id` | string | ❌ | — | |

#### Example

```json
{
  "query": "memory safety guarantees",
  "asset_ids": ["rust-book-ch4", "rust-book-ch5", "main-rs"],
  "k": 5
}
```

---

### 3.5 `delete_assets`

Permanently removes all chunks for the given asset IDs from the vector store.

#### Parameters

| Parameter | Type | Required | Default |
|---|---|---|---|
| `asset_ids` | string[] | ✅ | — |
| `tenant_id` | string | ❌ | `public` |
| `namespace` | string | ❌ | `default` |

#### Example

```json
{
  "asset_ids": ["doc-001", "notes-2024"]
}
```

**Expected response:**
```json
{
  "result": {
    "deleted": [
      { "asset_id": "doc-001", "points_deleted": 3 },
      { "asset_id": "notes-2024", "points_deleted": 7 }
    ]
  }
}
```

---

### 3.6 `list_assets`

Lists all indexed assets in a scope, with chunk counts. Optionally filtered by
`source_type`.

#### Parameters

| Parameter | Type | Required | Default | Notes |
|---|---|---|---|---|
| `tenant_id` | string | ❌ | `public` | |
| `namespace` | string | ❌ | `default` | |
| `source_type` | string | ❌ | — | Filter by source type. Same values as `ingest_asset`. |

#### Example — list everything

```json
{}
```

#### Example — list only PDFs

```json
{
  "source_type": "pdf"
}
```

**Expected response:**
```json
{
  "result": {
    "assets": [
      { "asset_id": "report-q1", "chunk_count": 42 },
      { "asset_id": "contract",  "chunk_count": 18 }
    ]
  }
}
```

---

## 4. End-to-End Workflow

A complete ingest → query → delete cycle to smoke-test the full stack:

```bash
# 1. Ingest a local file
#    (replace /path/to/file.txt with a real file on your machine)

# 2. Verify it was indexed
#    (list_assets should show it with chunk_count > 0)

# 3. Query it

# 4. Clean up
```

Using MCP Inspector, run these tool calls in order:

**Step 1 — ingest**
```json
{ "asset_id": "smoke-test-1", "source_uri": "/etc/hostname", "source_type": "local_file" }
```

**Step 2 — list**
```json
{}
```
→ Confirm `smoke-test-1` appears with `chunk_count: 1`.

**Step 3 — query**
```json
{ "query": "hostname", "asset_id": "smoke-test-1" }
```
→ Confirm a match is returned.

**Step 4 — delete**
```json
{ "asset_ids": ["smoke-test-1"] }
```
→ Confirm `points_deleted: 1`.

---

## 5. Common Errors

| Error message | Cause | Fix |
|---|---|---|
| `either content or source_uri must be provided` | Both fields absent | Supply one of them. |
| `source exceeds 5 MiB limit` | File or URL too large | Split the source or increase the limit (not currently configurable). |
| `cannot access '/path': No such file or directory` | Bad file path | Use an absolute path and confirm the file exists inside the container if running Docker. |
| `HTTP 404 fetching https://…` | URL not found | Verify the URL is reachable from the server's network. |
| `extracted text is empty` | PDF has no selectable text (scanned) or HTML body was empty | Provide `content` directly instead. |
| `unknown source_type 'Pdf'` | Wrong casing | All `source_type` values are lowercase with underscores: `pdf`, `local_file`, etc. |
| `ingest content cannot be empty` | `content` was provided but is whitespace-only | Check the value being passed. |

---

## 6. Verifying Storage Directly in Qdrant

Bypass the MCP layer and confirm chunks are physically stored:

```bash
# Count all points in the collection
curl -s -X POST http://localhost:6333/collections/chunks_te3_small/points/count \
  -H "Content-Type: application/json" \
  -d '{"exact": true}'

# Search for a specific asset_id
curl -s -X POST http://localhost:6333/collections/chunks_te3_small/points/scroll \
  -H "Content-Type: application/json" \
  -d '{
    "filter": {
      "must": [{ "key": "asset_id", "match": { "value": "smoke-test-1" } }]
    },
    "limit": 5,
    "with_payload": true
  }'
```

Adjust the collection name to match your `QDRANT_COLLECTION` env var.
