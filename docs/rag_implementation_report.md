# RAG Pipeline Implementation Report

> **Project**: `rag-api` — A custom Rust-based RAG pipeline for LibreChat  
> **Date**: April 2026  
> **Status**: Functional (MVP)

---

## Table of Contents

1. [How LibreChat Does RAG](#1-how-librechat-does-rag)
2. [Our Implementation: Architecture & Flow](#2-our-implementation-architecture--flow)
3. [RAG-MCP: The Model Context Protocol Server](#3-rag-mcp-the-model-context-protocol-server)
4. [Current Limitations](#4-current-limitations)
5. [Next Steps & Exploration Roadmap](#5-next-steps--exploration-roadmap)

---

## 1. How LibreChat Does RAG

### 1.1. The Two Phases of RAG

Retrieval-Augmented Generation (RAG) consists of two phases:

1.  **Retrieval Phase**: When a user asks a question, the system searches for snippets of information relevant to the user's prompt from an external knowledge base (in our case, a Qdrant vector database). The user's query is converted into a vector embedding, and a similarity search finds the most semantically relevant chunks of text.
2.  **Generative Phase**: The retrieved chunks are appended to the user's prompt and passed to the LLM (e.g., Kimi, GPT, Gemini). The model then synthesizes an answer based on both the retrieved context and its own training data.

### 1.2. How LibreChat Triggers RAG: Two Distinct Modes

LibreChat supports RAG through two mechanisms, and it is crucial to understand the difference:

#### Mode A: Context Injection ("Upload as Text")

This is the **simpler** mode. When a user uploads a small file, LibreChat may extract the full text content and inject it directly into the system prompt or conversation context. The LLM sees the entire file content in its context window.

- **How it works**: LibreChat calls the RAG API's `/text` endpoint to extract the file's text content. The extracted text is then inserted into the conversation as context for the model.
- **When it's used**: For small files where the entire content fits within the model's context window.
- **The model's perspective**: It sees the full file text as part of the conversation — it doesn't "know" it came from RAG. It just looks like a very long system message.
- **Config**: This is the default behavior. No special setup beyond `RAG_API_URL` is needed.

#### Mode B: Agentic File Search ("Tool Use")

This is the **intelligent** mode — and the one we use. The model is given a **tool** called "File Search." When it receives a user's question, it decides *on its own* whether to use the tool. If it does, it formulates its own search query, calls our RAG pipeline, reads the results, and then answers.

- **How it works**: LibreChat registers a "File Search" tool with the model. The model receives the user's question and thinks: *"I need to search the uploaded files to answer this."* It then generates a search query (which may be very different from the user's original phrasing), calls the `/query` endpoint, receives the top-k most relevant chunks, and uses them to formulate an answer.
- **When it's used**: When the "File Search" tool is enabled in the UI (the toggle at the bottom of the chat input). This is the standard mode for Agents.
- **The model's perspective**: It actively decides to use the tool. It sees the tool's response (the retrieved chunks) as structured data and uses it to answer.
- **Config**: Requires `RAG_API_URL` to be set. The user must enable the "File Search" toggle or use an Agent with the File Search capability.

> **Key Insight**: In Mode B, the *model* is the one doing the "searching" — not the user's raw prompt. If a user asks "Who is that guy in the book?", the model is smart enough to reformulate its search query as "main character names in Crime and Punishment" before calling our API. This is what makes Agentic RAG superior to simple keyword matching.

### 1.3. LibreChat RAG Configuration Reference

LibreChat expects a RAG API service that exposes a set of HTTP endpoints. Here are the key environment variables:

| Variable | Description | Our Value |
|---|---|---|
| `RAG_API_URL` | URL of the RAG API service | `http://legacy_proxy:8000` |
| `RAG_OPENAI_API_KEY` | API key for embeddings (overrides `OPENAI_API_KEY`) | Set in `.env` |
| `RAG_OPENAI_BASE_URL` | Custom base URL for embedding requests | Set in `.env` |
| `RAG_USE_FULL_CONTEXT` | If `true`, fetches entire file context instead of top-k results | `false` (default) |
| `EMBEDDINGS_PROVIDER` | Provider for embeddings: `openai`, `azure`, `huggingface`, `ollama` | N/A (we handle this ourselves) |

**Important**: LibreChat's **official** RAG API is a Python/FastAPI service that uses LangChain and PostgreSQL/pgvector. Our implementation is a **custom Rust replacement** that speaks the same HTTP protocol but uses Qdrant as the vector store and Ollama for local embeddings. LibreChat cannot tell the difference.

### 1.4. LibreChat File Config (`librechat.yaml`)

Our current configuration:

```yaml
fileConfig:
  endpoints:
    openAI:
      disabled: true
    default:
      fileLimit: 5         # Max 5 files per conversation
      fileSizeLimit: 20    # Max 20 MB per file
      totalSizeLimit: 50   # Max 50 MB total per request
```

The `fileSizeLimit` is the **effective upload limit** for end users. The backend proxy itself supports up to 100 MB (`DefaultBodyLimit::max(100 * 1024 * 1024)`).

---

## 2. Our Implementation: Architecture & Flow

### 2.1. System Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              USER'S BROWSER                                │
│                          (LibreChat Frontend)                              │
│                                                                             │
│  ┌─────────────┐  ┌────────────────┐  ┌──────────────────────────────────┐ │
│  │ Upload File  │  │  Ask Question  │  │  "File Search" Tool Toggle ✓   │ │
│  └──────┬──────┘  └───────┬────────┘  └──────────────────────────────────┘ │
└─────────┼─────────────────┼──────────────────────────────────────────────────┘
          │                 │
          ▼                 ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                        LibreChat API (Node.js)                              │
│                        container: LibreChat                                 │
│                        port: 3080                                           │
│                                                                             │
│  On Upload:                          On Question:                           │
│  POST /embed  ──────────┐            Model decides to use "File Search"    │
│  (multipart: file data) │            tool and formulates its own query     │
│                         │            POST /query ──────────┐               │
│                         │            (JSON: query, file_ids, k)            │
└─────────────────────────┼──────────────────────────────────┼────────────────┘
                          │                                  │
                          ▼                                  ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                    Legacy Proxy (Rust/Axum)                                 │
│                    container: legacy_proxy                                  │
│                    port: 8000                                               │
│                                                                             │
│  ┌──────────────────────────────────┐  ┌────────────────────────────────┐  │
│  │         INGESTION FLOW           │  │        RETRIEVAL FLOW          │  │
│  │                                  │  │                                │  │
│  │  1. Receive multipart upload     │  │  1. Receive query + file_ids   │  │
│  │  2. Extract text from file       │  │  2. Embed the query text       │  │
│  │  3. Chunk text (1000 chars,      │  │     → Ollama nomic-embed-text  │  │
│  │     200 overlap, UTF-8 safe)     │  │  3. Vector search in Qdrant    │  │
│  │  4. Embed each batch of 20      │  │     (top-k=4 by default)       │  │
│  │     chunks → Ollama              │  │  4. Return scored chunks       │  │
│  │  5. Upsert to Qdrant            │  │     to LibreChat               │  │
│  │  6. Return extracted text        │  │                                │  │
│  │     to LibreChat                 │  │  Brutal Logging:               │  │
│  │                                  │  │  • Chunk count                 │  │
│  │                                  │  │  • Score per chunk             │  │
│  │                                  │  │  • Text preview (200 chars)    │  │
│  └──────────────────────────────────┘  └────────────────────────────────┘  │
└─────────────────────────┬──────────────────────────────────┬────────────────┘
                          │                                  │
                          ▼                                  ▼
┌──────────────────────────────────┐  ┌──────────────────────────────────────┐
│       Ollama (Local LLM)         │  │          Qdrant (Vector DB)          │
│       host.docker.internal:11434 │  │          container: qdrant           │
│                                  │  │          port: 6333, 6334            │
│  Model: nomic-embed-text         │  │                                      │
│  Dimensions: 768                 │  │  Collection: chunks_nomic            │
│  API: OpenAI-compatible          │  │  Distance: Cosine                    │
│  (/v1/embeddings)                │  │  Vectors: 768 dimensions             │
│                                  │  │  Storage: Docker volume              │
└──────────────────────────────────┘  └──────────────────────────────────────┘
```

### 2.2. Service Inventory

| Service | Technology | Container | Port | Role |
|---|---|---|---|---|
| **LibreChat** | Node.js | `LibreChat` | 3080 | Frontend + API gateway |
| **Legacy Proxy** | Rust (Axum) | `legacy_proxy` | 8000 | RAG API bridge (ingestion + retrieval) |
| **RAG API** | Rust (Axum) | `rag_api` | 8080 | Native RAG API (future use) |
| **RAG MCP** | Rust | `rag_mcp` | 8081 | MCP protocol server (future use) |
| **Ollama** | Go | Host machine | 11434 | Local embedding model server |
| **Qdrant** | Rust | `qdrant` | 6333/6334 | Vector database |
| **MongoDB** | C++ | `chat-mongodb` | 27017 | LibreChat user/conversation storage |
| **Meilisearch** | Rust | `chat-meilisearch` | 7700 | Full-text search for conversations |
| **Redis** | C | `redis` | 6379 | Query caching / locking |

### 2.3. The Ingestion Flow (File Upload → Vectors)

When a user uploads a file through LibreChat's UI:

```
Step 1: LibreChat sends POST /embed to legacy_proxy
        (multipart form with file data, file_id, filename)

Step 2: Legacy Proxy extracts raw text from the file

Step 3: RecursiveChunker splits the text into overlapping chunks
        • chunk_size:    1000 characters
        • chunk_overlap:  200 characters
        • UTF-8 safe:     Slices on character boundaries, not bytes
        • Example: A 1MB file (~1M chars) → ~1,250 chunks

Step 4: Chunks are batched (20 per request) and sent to Ollama
        • Endpoint: POST http://host.docker.internal:11434/v1/embeddings
        • Model:    nomic-embed-text
        • Output:   768-dimensional float32 vectors per chunk

Step 5: Each chunk + its embedding vector is upserted to Qdrant
        • Collection: chunks_nomic
        • Metadata: tenant_id, namespace, asset_id, actor_id,
                    source_type, chunk_index, digest (SHA-256)

Step 6: Legacy Proxy returns the extracted text to LibreChat
        (LibreChat may also use this text for context injection)
```

### 2.4. The Retrieval Flow (Question → Answer)

When the model decides to use the "File Search" tool:

```
Step 1: Model formulates a search query based on the user's question
        Example: User says "Who reads the Bible to Raskolnikov?"
                 Model searches for "character reading Bible Gospel
                 Raskolnikov Crime and Punishment Sonia"

Step 2: LibreChat sends POST /query to legacy_proxy
        {
          "query": "character reading Bible Gospel Raskolnikov...",
          "file_ids": ["2ac3c233-7f4a-..."],
          "k": 4
        }

Step 3: Legacy Proxy embeds the query text using Ollama
        → Produces a single 768-dim query vector

Step 4: Qdrant performs cosine similarity search
        → Returns top-k (default 4) most similar chunks
        → Each result includes: text content + similarity score

Step 5: Legacy Proxy logs the results (Brutal Mode):
        "Chunk #1 [Score: 0.8521]: "Sonia read the Gospel of St. John..."
        "Chunk #2 [Score: 0.7934]: "Lazarus, come forth!..."

Step 6: Results are returned to LibreChat as structured JSON
        → Model reads the chunks and synthesizes its answer
```

### 2.5. Key Crate Responsibilities

| Crate | Path | Responsibility |
|---|---|---|
| `rag-core` | `crates/core/` | Trait definitions (`IngestService`, `QueryService`, `Chunker`, `EmbeddingClient`), data types |
| `rag-app-runtime` | `crates/app-runtime/` | Concrete implementations: `SimpleIngestService`, `RecursiveChunker`, environment config |
| `rag-legacy-compat` | `crates/legacy-compat/` | Axum HTTP handlers that translate LibreChat's legacy RAG API protocol into our core services |
| `rag-openai-compat` | `crates/openai-compat/` | OpenAI-compatible HTTP client for embedding requests (talks to Ollama) |
| `rag-storage-qdrant` | `crates/storage-qdrant/` | Qdrant gRPC client implementing `ChunkRepository` |
| `rag-mcp-api` | `crates/mcp-api/` | MCP JSON-RPC server exposing RAG capabilities as MCP tools (see [Section 3](#3-rag-mcp-the-model-context-protocol-server)) |

---

## 3. RAG-MCP: The Model Context Protocol Server

### 3.1. What is MCP?

The **Model Context Protocol (MCP)** is an open standard introduced by Anthropic (November 2024, protocol revision `2024-11-05`) that provides a universal way for LLM applications to connect to external tools, data sources, and services. Think of it as a "USB-C for AI" — instead of building custom integrations for every AI app + tool combination, both sides implement MCP once and gain interoperability.

MCP uses **JSON-RPC 2.0** over HTTP (or stdio) and defines three core primitives:

| Primitive | Purpose | Who Controls It | Example |
|---|---|---|---|
| **Tools** | Executable functions that perform actions or retrieve data | **Model-initiated** — the LLM decides when to call | `search_documents`, `ingest_file`, `delete_collection` |
| **Resources** | Read-only structured data that provides context | **Client-initiated** — the app or user requests | File contents, database schemas, index stats |
| **Prompts** | Reusable prompt templates with arguments | **User-initiated** — explicit selection | `summarize_document`, `compare_files` |

The communication flow:

```
┌──────────────┐       ┌──────────────┐       ┌──────────────┐
│   MCP Host   │       │  MCP Client  │       │  MCP Server  │
│  (LibreChat) │◄─────►│  (connector) │◄─────►│  (rag-mcp)   │
│              │       │              │       │              │
│  The AI app  │       │  Manages the │       │  Exposes our  │
│  the user    │       │  JSON-RPC    │       │  RAG tools,   │
│  interacts   │       │  connection  │       │  resources,   │
│  with        │       │  lifecycle   │       │  and prompts  │
└──────────────┘       └──────────────┘       └──────────────┘
```

### 3.2. Why Build an MCP Server for RAG?

Our `legacy_proxy` works today because it mimics the *exact HTTP protocol* that LibreChat's official Python RAG API uses. But this approach has a fundamental limitation: **it only works with LibreChat**. If we wanted to use the same RAG pipeline from Cursor, Claude Desktop, Windsurf, or any other MCP-compatible client, we would need to build a separate integration for each.

An MCP server solves this by providing a **protocol-level integration** that any MCP-compatible host can discover and use automatically:

| Approach | Works With | Integration Effort per Client |
|---|---|---|
| **Legacy Proxy** (current) | LibreChat only | N/A (bespoke) |
| **MCP Server** (future) | LibreChat, Claude Desktop, Cursor, Windsurf, Cline, any MCP host | Zero — plug and play |

Additionally, MCP provides capabilities our legacy proxy doesn't have:
- **Tool Discovery**: The host auto-discovers available tools — no hardcoded endpoint mapping
- **Schema Validation**: Tool input/output schemas are declared upfront in JSON Schema
- **Capability Negotiation**: Server and client negotiate features at connection time
- **Deferred Tools**: LibreChat supports "lazy-loading" MCP tools so they don't consume context window until needed

### 3.3. Current State: Scaffolding

The `rag-mcp` binary and `rag-mcp-api` crate are **deployed and running** (container `rag_mcp`, port `8081`) but currently contain only the protocol scaffolding. No functional tools are registered yet.

**Binary**: [`bins/rag-mcp.rs/src/main.rs`](file:///home/koufan/rag-api/bins/rag-mcp.rs/src/main.rs)

The binary boots an Axum HTTP server on port `8081`, initializes the same `build_container()` runtime used by the legacy proxy (so it has access to the same `IngestService`, `QueryService`, and `ExtractService`), and wires up the MCP router.

**Crate**: [`crates/mcp-api/src/lib.rs`](file:///home/koufan/rag-api/crates/mcp-api/src/lib.rs)

The MCP API crate implements the JSON-RPC 2.0 dispatch loop with the following methods already handled:

| Method | Status | Description |
|---|---|---|
| `initialize` | ✅ Responds | Returns server info, protocol version `2024-11-05`, capabilities |
| `notifications/initialized` | ✅ Responds | Acknowledges client initialization |
| `ping` | ✅ Responds | Health check |
| `tools/list` | ⚠️ Empty | Returns `{ "tools": [] }` — no tools registered yet |
| `resources/list` | ⚠️ Empty | Returns `{ "resources": [] }` — no resources registered yet |
| `prompts/list` | ⚠️ Empty | Returns `{ "prompts": [] }` — no prompts registered yet |
| `tools/call` | ❌ Not handled | Will dispatch to actual RAG operations |

**Health endpoint**: `GET /healthz` → `{"status":"ok","service":"rag-mcp.rs"}`

### 3.4. Development Plan: Tools to Implement

The MCP server has access to the same core services as the legacy proxy. The plan is to expose them as MCP tools:

#### Tool 1: `search_documents`

The most critical tool. Allows any MCP-connected model to perform semantic search against the Qdrant vector store.

```json
{
  "name": "search_documents",
  "description": "Search indexed documents using semantic similarity. Returns the most relevant text chunks for a given query.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "The search query to find relevant document chunks"
      },
      "file_ids": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Optional list of file IDs to restrict search scope"
      },
      "top_k": {
        "type": "integer",
        "default": 4,
        "description": "Number of results to return (1-20)"
      }
    },
    "required": ["query"]
  }
}
```

**Implementation**: Calls `QueryService::query_batch()` — the exact same code path as `POST /query` in the legacy proxy.

#### Tool 2: `ingest_text`

Allows a model or client to programmatically ingest text content into the vector store.

```json
{
  "name": "ingest_text",
  "description": "Ingest text content into the RAG vector store. The text will be chunked, embedded, and indexed for later retrieval.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "content": { "type": "string", "description": "The text content to ingest" },
      "file_id": { "type": "string", "description": "Unique identifier for this document" },
      "filename": { "type": "string", "description": "Original filename for metadata" }
    },
    "required": ["content", "file_id"]
  }
}
```

**Implementation**: Calls `IngestService::ingest()` — same chunking and embedding pipeline.

#### Tool 3: `list_indexed_documents`

Returns metadata about all documents currently indexed in the vector store.

```json
{
  "name": "list_indexed_documents",
  "description": "List all documents that have been indexed in the RAG vector store, with chunk counts and metadata.",
  "inputSchema": {
    "type": "object",
    "properties": {},
    "required": []
  }
}
```

#### Tool 4: `delete_document`

Removes a document and all its chunks from the vector store.

```json
{
  "name": "delete_document",
  "description": "Delete a document and all its chunks from the RAG vector store.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "file_id": { "type": "string", "description": "ID of the document to delete" }
    },
    "required": ["file_id"]
  }
}
```

### 3.5. Integration with LibreChat

Once the tools are implemented, connecting `rag-mcp` to LibreChat requires adding it to `librechat.yaml`:

```yaml
mcpServers:
  rag:
    type: sse                          # HTTP-based transport
    url: http://rag_mcp:8081/mcp       # Docker-internal URL
    timeout: 300000                    # 5 min (for long ingestion)
    serverInstructions: true           # Use server-provided instructions
```

After a restart, LibreChat will:
1. Connect to `http://rag_mcp:8081/mcp` and call `initialize`
2. Call `tools/list` to discover available tools
3. Make `search_documents`, `ingest_text`, etc. available to any model or Agent

This means **any model endpoint** in LibreChat (not just Agents) can use our RAG tools via the MCP dropdown in the chat input — no File Search toggle needed.

### 3.6. MCP vs. Legacy Proxy: Coexistence

The two systems are designed to **coexist**, not replace each other:

```
                    LibreChat
                    ┌────────────────────────────┐
                    │                            │
                    │  File Search tool          │──► legacy_proxy:8000
                    │  (built-in RAG API)        │    (current, working)
                    │                            │
                    │  MCP Tools dropdown        │──► rag_mcp:8081
                    │  (protocol-based tools)    │    (future, universal)
                    │                            │
                    └────────────────────────────┘
```

- **Legacy Proxy**: Continues to serve LibreChat's built-in File Search feature. This is the production path.
- **MCP Server**: Opens the same RAG pipeline to any MCP-compatible client. This is the extensibility path.

Both share the same `app-runtime` container (same chunker, same embedding client, same Qdrant storage). The difference is only in the **protocol layer**.

---

## 4. Current Limitations

### 3.1. Performance: Embedding Speed

**The Problem**: Ingesting a 1 MB text file (e.g., *Crime and Punishment*) takes approximately **18 minutes**.

**Root Cause Analysis**:

| Component | Time per Batch | Batches for 1MB | Total |
|---|---|---|---|
| Ollama embedding (20 chunks × 1000 chars) | ~13 seconds | ~63 batches | ~13.6 min |
| Qdrant upsert | ~0.1 seconds | ~63 batches | ~6 sec |
| Network overhead | ~0.2 seconds | ~63 batches | ~12 sec |
| **Total** | | | **~14-18 min** |

The bottleneck is **Ollama's embedding throughput on CPU**. The `nomic-embed-text` model (137M parameters) runs on CPU by default, processing 20 chunks (~20,000 characters) in approximately 13 seconds.

**Potential Improvements**:

| Strategy | Expected Speedup | Complexity | Notes |
|---|---|---|---|
| **GPU Acceleration** (Ollama with CUDA) | 10-20× | Low | If a GPU is available, Ollama can embed in <1s per batch |
| **Parallel Batching** (concurrent HTTP requests) | 2-4× | Medium | Send multiple batches to Ollama simultaneously. We attempted this but hit Rust lifetime issues and Ollama connection limits |
| **Larger Batch Size** (50-100 chunks per request) | 1.5-2× | Low | Reduces HTTP overhead but increases memory per request |
| **Remote Embedding API** (OpenAI, Voyage) | 5-10× | Low | Trade latency for throughput; costs money per token |
| **Pre-chunking with Async Queue** | N/A (UX) | Medium | Accept the upload instantly, process in background, notify when ready |

### 3.2. File Type Support

**Currently Supported**: Plain text files (`.txt`) only.

**Not Yet Supported**:
- PDF documents (requires a PDF parsing library like `pdf-extract` or `lopdf`)
- Word documents (`.docx`)
- Markdown (`.md`) — partially works as plain text
- HTML — needs tag stripping
- Code files — work as plain text but no syntax-aware chunking
- Images — would require OCR (Tesseract) or multimodal embeddings

### 3.3. Chunking Strategy

**Current**: Fixed-size character chunking (1000 chars, 200 overlap).

**Limitations**:
- **No semantic awareness**: Chunks may split mid-sentence or mid-paragraph
- **No metadata extraction**: Chapter titles, headers, and structural information are lost
- **One-size-fits-all**: The same chunk size is used for poetry, prose, and technical docs
- **No deduplication**: Re-uploading the same file creates duplicate chunks (though the SHA-256 digest could be used for dedup)

### 3.4. Retrieval Quality

**Current**: Basic cosine similarity search with `nomic-embed-text` (768 dimensions).

**Limitations**:
- **No re-ranking**: Results are returned in order of raw cosine similarity, with no secondary scoring
- **No hybrid search**: We use dense vector search only — no sparse/keyword fallback for exact matches
- **No metadata filtering**: Cannot filter by date, author, chapter, or other structured attributes
- **Fixed top-k**: Default `k=4` may be too few for complex questions or too many for simple ones

### 3.5. Embedding Model Quality

Based on independent benchmarks (MTEB, Tiger Data, Milvus):

| Model | Overall Accuracy | Dimensions | Cost | Notes |
|---|---|---|---|---|
| **nomic-embed-text** (ours) | 71.0% | 768 | Free (local) | Budget option, runs on CPU |
| OpenAI text-embedding-3-small | 75.8% | 768 | $0.02/1M tokens | Best value commercial |
| OpenAI text-embedding-3-large | 80.5% | 3072 | $0.13/1M tokens | Industry standard |
| Voyage Multimodal 3.5 | 88.0%+ | 2048 | $0.12/1M tokens | State-of-the-art |
| Gemini Embedding 2 | 99.7% (easy) | 3072 | $0.20/1M tokens | Google's best |

**Key takeaway**: Our `nomic-embed-text` model performs within **5-10%** of OpenAI's small model on detailed questions (88-97% accuracy for both), but the gap widens significantly on **vague or context-dependent queries** (42-57% accuracy). For a local, free, zero-cost solution, it is remarkably capable.

### 3.6. Scalability

| Dimension | Current State | Production Target |
|---|---|---|
| Concurrent Users | 1 (development) | 10-100 |
| Documents in Qdrant | ~1,250 chunks (1 book) | 100,000+ chunks |
| Qdrant RAM usage | ~50 MB | Would need monitoring at scale |
| Embedding throughput | ~1.1 chunks/sec (CPU) | 100+ chunks/sec (GPU) |

---

## 5. Next Steps & Exploration Roadmap

### 4.1. Near-Term: PDF & Multi-Format Support

**Priority: HIGH** — This is the most requested feature.

| Format | Library/Approach | Effort |
|---|---|---|
| **PDF** | `pdf-extract` or `lopdf` crate for text extraction. For scanned PDFs, use Tesseract OCR via `leptess` | Medium |
| **DOCX** | `docx-rs` or shell out to `pandoc` for conversion | Low |
| **Markdown** | Strip frontmatter, parse headers as metadata, preserve code blocks | Low |
| **HTML** | `scraper` crate for DOM parsing, extract text content | Low |
| **EPUB** | Unzip + HTML extraction | Medium |

**Architecture**: Add a `FileProcessor` trait with implementations for each format. The `ExtractService` would dispatch based on MIME type.

### 4.2. Near-Term: Background Ingestion Queue

**Priority: HIGH** — Eliminates the "18-minute wait" UX problem.

Instead of processing files synchronously during upload:

1. Accept the upload immediately and return a "processing" status
2. Push the file into a Redis-backed job queue
3. A background worker processes the queue (chunking + embedding)
4. LibreChat can poll for completion status

This decouples the upload UX from the embedding bottleneck.

### 4.3. Mid-Term: Embedding Model Upgrades

**Priority: MEDIUM** — Significant quality improvement with minimal code changes.

Our `openai-compat` crate already speaks the OpenAI embedding API. Switching models is a config change:

| Model | What Changes | Expected Impact |
|---|---|---|
| **OpenAI text-embedding-3-small** | Set `OPENAI_BASE_URL=https://api.openai.com/v1`, `OPENAI_EMBED_MODEL=text-embedding-3-small` | +5% accuracy, $0.02/1M tokens |
| **OpenAI text-embedding-3-large** | Same as above with `text-embedding-3-large` | +10% accuracy, $0.13/1M tokens, needs Qdrant collection with 3072 dims |
| **Voyage 3** | Point to Voyage API, same OpenAI-compatible format | +17% accuracy, $0.06/1M tokens |
| **Gemini Embedding 2** | Use Google's embedding API | Best-in-class, $0.20/1M tokens |

> **Important**: Switching embedding models requires **re-indexing all existing documents**. Vectors from different models are incompatible. A new Qdrant collection must be created for each model.

### 4.4. Mid-Term: Hybrid Search (Dense + Sparse)

**Priority: MEDIUM** — Dramatically improves retrieval for exact-match queries.

Dense vector search (what we have) is excellent for semantic similarity but struggles with exact keyword matches. For example, searching for "Article 42" might not return the chunk containing that exact phrase if the embedding doesn't capture the number precisely.

**Solution**: Combine dense vectors with sparse vectors (BM25/SPLADE) in Qdrant:
- **Dense search**: "What are the tenant's obligations?" → Finds semantically similar text
- **Sparse search**: "Article 42" → Finds the exact phrase
- **Hybrid**: Combine both scores for the best of both worlds

Qdrant natively supports [named vectors](https://qdrant.tech/documentation/) and [sparse vectors](https://qdrant.tech/articles/sparse-vectors/), making this a natural extension.

### 4.5. Mid-Term: Smarter Chunking

**Priority: MEDIUM** — Better chunks = better retrieval.

| Strategy | Description | Benefit |
|---|---|---|
| **Sentence-aware splitting** | Split on sentence boundaries instead of fixed character count | No more mid-sentence cuts |
| **Recursive character splitting** | Try to split on `\n\n` first, then `\n`, then `.`, then by character | Preserves paragraph structure |
| **Header-aware chunking** | For Markdown/HTML, use headers as chunk boundaries | Each chunk has a meaningful title |
| **Sliding window with summary** | Prepend a summary of the previous chunk to maintain context | Better for narrative text |
| **Semantic chunking** | Use an LLM to identify topic boundaries | Most accurate but expensive |

### 4.6. Long-Term: Re-Ranking

**Priority: LOW** — Polish layer for retrieval quality.

After retrieving the top-k chunks via vector search, pass them through a **cross-encoder re-ranker** that scores each chunk against the original query more precisely. This is significantly more compute-intensive than embedding similarity but dramatically improves precision.

Options:
- **Cohere Rerank** ($1/1000 searches)
- **BGE Reranker** (open source, runs locally)
- **Jina Reranker** (API or local)

### 4.7. Long-Term: Multi-Document Synthesis

**Priority: LOW** — Enable cross-document reasoning.

Our `/query_multiple` endpoint is already scaffolded. The vision:
- Upload a contract, a legal memo, and a regulatory document
- Ask: "Does the contract comply with the regulation?"
- The system retrieves relevant chunks from *all three documents* and synthesizes an answer

### 4.8. Long-Term: Semantic Caching

**Priority: LOW** — Performance optimization for repeated queries.

Store previously-asked questions and their retrieved results in a separate Qdrant collection. When a new question arrives, first check if a semantically similar question has been asked before. If so, return the cached results instead of re-embedding and re-searching.

Our Redis infrastructure is already in place for this.

### 4.9. Exploration: Multimodal Embeddings

**Priority: EXPLORATORY** — Enable RAG over images and diagrams.

Models like **Nomic Embed Vision**, **Jina CLIP v2**, and **Voyage Multimodal 3.5** can embed both text and images into the same vector space. This would allow:
- Upload a PDF with diagrams → The diagrams are embedded alongside the text
- Ask "What does the architecture diagram show?" → The system retrieves the diagram's embedding

### 4.10. Exploration: Fine-Tuned Embeddings

**Priority: EXPLORATORY** — Domain-specific accuracy improvements.

For specialized domains (legal, medical, financial), generic embedding models may underperform. Fine-tuning on domain-specific data can improve retrieval accuracy by 10-20%.

Options:
- **Nomic Embed** supports fine-tuning (open source)
- **OpenAI** offers managed fine-tuning for `text-embedding-3-*` models
- **Sentence Transformers** + custom training data

---

## Appendix A: Environment Variables (`.env`)

```bash
# Embedding Configuration
OPENAI_BASE_URL=http://host.docker.internal:11434/v1
OPENAI_EMBED_MODEL=nomic-embed-text
OPENAI_API_KEY=sk-placeholder       # Ollama doesn't need a real key

# Qdrant Configuration
QDRANT_COLLECTION=chunks_nomic
# QDRANT_URL is set per-container in compose.yaml

# Legacy Proxy Defaults
LEGACY_DEFAULT_TENANT=public
LEGACY_DEFAULT_NAMESPACE=librechat
```

## Appendix B: Key API Endpoints (Legacy Proxy)

| Method | Path | Description |
|---|---|---|
| `GET` | `/health` | Health check → `{"status":"UP"}` |
| `POST` | `/embed` | Upload + ingest a file (multipart) |
| `POST` | `/embed-upload` | Alternative upload endpoint |
| `POST` | `/local/embed` | Local file embedding |
| `POST` | `/text` | Extract text from a file |
| `POST` | `/query` | Single-file semantic search |
| `POST` | `/query_multiple` | Multi-file semantic search |
| `GET` | `/ids` | List indexed asset IDs (not yet implemented) |
| `GET` | `/documents` | List documents (not yet implemented) |
| `DELETE` | `/documents` | Delete documents (not yet implemented) |
| `GET` | `/documents/{id}/context` | Get document context (not yet implemented) |

## Appendix C: Verified Benchmark — Crime and Punishment

| Metric | Value |
|---|---|
| File size | 1.0 MB (plain text) |
| Character count | ~1,000,000 |
| Chunks generated | ~1,250 (1000 chars, 200 overlap) |
| Embedding batches | ~63 (20 chunks each) |
| Time per batch | ~13 seconds (CPU, Ollama) |
| Total ingestion time | ~18 minutes |
| Vector dimensions | 768 (nomic-embed-text) |
| Retrieval latency | <2 seconds (query embedding + Qdrant search) |
| Retrieval accuracy | Verified correct answers for literary analysis questions |
