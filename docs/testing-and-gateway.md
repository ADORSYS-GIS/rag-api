# RAG MCP — Testing Guide & Docker MCP Gateway Integration

## Table of Contents

1. [Testing the MCP server locally](#1-testing-the-mcp-server-locally)
   - 1.1 [Prerequisites](#11-prerequisites)
   - 1.2 [Start the stack](#12-start-the-stack)
   - 1.3 [Verify health](#13-verify-health)
   - 1.4 [Connect an agent on localhost](#14-connect-an-agent-on-localhost)
   - 1.5 [Test with MCP Inspector](#15-test-with-mcp-inspector)
   - 1.6 [Manual tool calls with curl](#16-manual-tool-calls-with-curl)
   - 1.7 [Inspect Qdrant](#17-inspect-qdrant)
   - 1.8 [Tear down](#18-tear-down)
2. [Docker MCP Gateway integration](#2-docker-mcp-gateway-integration)
   - 2.1 [How the gateway works](#21-how-the-gateway-works)
   - 2.2 [Why our server needs changes](#22-why-our-server-needs-changes)
   - 2.3 [What needs to be built](#23-what-needs-to-be-built)
   - 2.4 [Step 1 — Add stdio transport to the binary](#24-step-1--add-stdio-transport-to-the-binary)
   - 2.5 [Step 2 — Add the `io.docker.server.metadata` image label](#25-step-2--add-the-iodockerservermetadata-image-label)
   - 2.6 [Step 3 — Write the server entry YAML](#26-step-3--write-the-server-entry-yaml)
   - 2.7 [Step 4 — Register with the gateway](#27-step-4--register-with-the-gateway)
   - 2.8 [Step 5 — Connect a client](#28-step-5--connect-a-client)
   - 2.9 [Qdrant sidecar problem](#29-qdrant-sidecar-problem)

---

## 1. Testing the MCP server locally

### 1.1 Prerequisites

- Docker Engine (or Docker Desktop) running
- The `rag-api-rag_mcp:latest` image already built (it is — check with
  `docker images | grep rag_mcp`)
- An embedding provider reachable from the container:
  - **Ollama on the host machine** — use `http://host.docker.internal:11434/v1`
  - **Remote OpenAI-compatible API** — use your provider's base URL

Set the following environment variables before starting. The easiest way is a
`.env` file in the project root (copy `.env.example` and fill it in):

```bash
# .env — values for the test stack

# Embedding provider
OPENAI_BASE_URL=http://host.docker.internal:11434/v1   # or your remote URL
OPENAI_API_KEY=ollama                                   # placeholder for Ollama; real key for remote
OPENAI_EMBED_MODEL=nomic-embed-text                     # or text-embedding-3-small, etc.

# Qdrant collection — must match the embedding model's output dimensions
QDRANT_COLLECTION=chunks_nomic      # nomic-embed-text → 768 dims
QDRANT_VECTOR_SIZE=768              # must match the model

# Optional performance tuning (defaults shown)
INGEST_EMBED_BATCH_SIZE=50
INGEST_EMBED_CONCURRENCY=4
INGEST_UPSERT_BATCH_SIZE=100
INGEST_UPSERT_CONCURRENCY=4
```

If you are using `text-embedding-3-small` (OpenAI), use:

```bash
QDRANT_COLLECTION=chunks_te3_small
QDRANT_VECTOR_SIZE=1536
```

### 1.2 Start the stack

```bash
docker compose -f compose.mcp-test.yml up
```

This starts exactly two containers:

| Container | Image | Port |
|---|---|---|
| `qdrant` | `qdrant/qdrant:v1.13.6` | 6333 (HTTP/UI), 6334 (gRPC) |
| `rag-mcp` | `rag-api-rag_mcp:latest` | 8081 |

The MCP container waits for Qdrant's health check to pass before starting, so
startup order is guaranteed.

To run in the background:

```bash
docker compose -f compose.mcp-test.yml up -d
```

To follow logs:

```bash
docker compose -f compose.mcp-test.yml logs -f rag-mcp
```

### 1.3 Verify health

```bash
curl http://localhost:8081/healthz
```

Expected response:

```json
{"status":"ok","service":"rag-mcp"}
```

```bash
curl http://localhost:8081/readyz
```

Expected response:

```json
{"status":"ready","service":"rag-mcp"}
```

### 1.4 Connect an agent on localhost

The server speaks **Streamable HTTP** (the current MCP transport standard,
replacing the older SSE transport). The MCP endpoint is:

```
http://localhost:8081/mcp
```

Add this entry to your agent's MCP configuration file:

```json
{
  "mcpServers": {
		"rag": {
			"url": "http://localhost:8090/mcp",
			"type": "streamable-http",
			"alwaysAllow": [
				"query_asset"
			]
		}
  }
}
```

**Where the config file lives per agent:**

| Agent | Config file path |
|---|---|
| Claude Desktop | `~/.config/claude/claude_desktop_config.json` (Linux) |
| Cursor | `.cursor/mcp.json` in your project, or `~/.cursor/mcp.json` globally |
| VS Code Copilot | `.vscode/mcp.json` in your project |
| Kiro | `.kiro/settings/mcp.json` in your project |
| Windsurf | `~/.codeium/windsurf/mcp_config.json` |

After saving the config, restart the agent. It will call `initialize` on the
server, then `tools/list`, and surface the six tools automatically.

**The six tools the agent will see:**

| Tool | What it does |
|---|---|
| `ingest_asset` | Chunk, embed, and index text content into Qdrant |
| `extract_asset_text` | Extract plain text without storing vectors |
| `query_asset` | Semantic search over a single asset |
| `query_assets` | Semantic search across multiple assets |
| `delete_assets` | Delete all chunks for one or more assets |
| `list_assets` | List all indexed assets with chunk counts |

### 1.5 Test with MCP Inspector

MCP Inspector is the official interactive testing tool for MCP servers. It
requires Node.js.

```bash
npx @modelcontextprotocol/inspector
```

Open the URL it prints (usually `http://localhost:6274`). In the UI:

1. Set transport to **Streamable HTTP**
2. Set URL to `http://localhost:8081/mcp`
3. Click **Connect**

You will see the server info, the list of tools with their full JSON schemas,
and a form to call each tool interactively.

### 1.6 Manual tool calls with curl

The Streamable HTTP transport accepts standard JSON-RPC 2.0 POST requests to
`/mcp`. You can test individual tools directly.

**Initialize a session:**

```bash
curl -X POST http://localhost:8081/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
      "protocolVersion": "2024-11-05",
      "capabilities": {},
      "clientInfo": { "name": "curl-test", "version": "0.1" }
    }
  }'
```

**List tools:**

```bash
curl -X POST http://localhost:8081/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
```

**Ingest a document:**

```bash
curl -X POST http://localhost:8081/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
      "name": "ingest_asset",
      "arguments": {
        "asset_id": "test-doc-1",
        "content": "The Qdrant vector database stores high-dimensional vectors and supports fast approximate nearest-neighbour search.",
        "source_type": "text",
        "tenant_id": "public",
        "namespace": "default"
      }
    }
  }'
```

Expected response (chunks_written will vary by content length):

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "content": [{ "type": "text", "text": "{\"result\":{\"asset_id\":\"test-doc-1\",\"chunks_written\":1}}" }]
  }
}
```

**Query the document:**

```bash
curl -X POST http://localhost:8081/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 4,
    "method": "tools/call",
    "params": {
      "name": "query_asset",
      "arguments": {
        "query": "vector similarity search",
        "asset_id": "test-doc-1",
        "k": 3,
        "tenant_id": "public",
        "namespace": "default"
      }
    }
  }'
```

**List all assets:**

```bash
curl -X POST http://localhost:8081/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 5,
    "method": "tools/call",
    "params": {
      "name": "list_assets",
      "arguments": {
        "tenant_id": "public",
        "namespace": "default"
      }
    }
  }'
```

**Delete an asset:**

```bash
curl -X POST http://localhost:8081/mcp \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 6,
    "method": "tools/call",
    "params": {
      "name": "delete_assets",
      "arguments": {
        "asset_ids": ["test-doc-1"],
        "tenant_id": "public",
        "namespace": "default"
      }
    }
  }'
```

### 1.7 Inspect Qdrant

The Qdrant web dashboard is available at `http://localhost:6333/dashboard` while
the stack is running. You can browse collections, inspect individual points, and
run test queries from the UI.

The REST API is also available:

```bash
# List collections
curl http://localhost:6333/collections

# Get collection info (replace chunks_nomic with your collection name)
curl http://localhost:6333/collections/chunks_nomic
```

### 1.8 Tear down

```bash
docker compose -f compose.mcp-test.yml down
```

To also delete the Qdrant data volume (wipes all indexed vectors):

```bash
docker compose -f compose.mcp-test.yml down -v
```

---

## 2. Docker MCP Gateway integration

### 2.1 How the gateway works

The [docker/mcp-gateway](https://github.com/docker/mcp-gateway) is a Docker CLI
plugin (`docker mcp`) that acts as a single gateway between AI clients and a
collection of MCP servers. Its architecture is:

```
AI Client (Claude Desktop, Cursor, etc.)
        │
        │  stdio or HTTP (SSE / Streamable)
        ▼
  docker mcp gateway run
        │
        │  spawns and manages
        ▼
  Docker containers (one per MCP server)
        │
        │  stdio (stdin/stdout JSON-RPC)
        ▼
  MCP server process inside container
```

The critical point: **the gateway communicates with each MCP server container
over stdio**, not over HTTP. It spawns the container, pipes JSON-RPC messages
through stdin, and reads responses from stdout. The gateway itself can expose
its aggregated interface to clients over stdio, SSE, or Streamable HTTP — but
the server-side transport is always stdio.

This is different from our current setup, where `rag-mcp` binds a TCP port and
speaks Streamable HTTP directly. That works perfectly for direct client
connections (section 1 above), but it is the wrong transport for the gateway's
container model.

### 2.2 Why our server needs changes

Our binary currently only supports one transport: Streamable HTTP (it calls
`axum::serve` on a TCP listener). The gateway needs to be able to run the
container with `docker run -i` and exchange JSON-RPC over stdin/stdout.

`rmcp` (the library we use) supports both transports. Adding stdio is a small
change to the binary — roughly 20 lines — but it requires a `--transport` flag
or `TRANSPORT` env var so the same image can serve both use cases:

- `TRANSPORT=http` (default) → current behaviour, binds port 8081
- `TRANSPORT=stdio` → reads from stdin, writes to stdout, no port binding

### 2.3 What needs to be built

Three things are required, in order:

1. **Stdio transport mode in the binary** — `bins/rag-mcp.rs/src/main.rs`
2. **`io.docker.server.metadata` OCI label on the image** — `Dockerfile`
3. **A server entry YAML file** — for registering with the gateway via profiles
   or catalogs

### 2.4 Step 1 — Add stdio transport to the binary

The `bins/rag-mcp.rs/src/main.rs` binary needs to branch on a `TRANSPORT` env
var. When `stdio`, it uses `rmcp`'s stdio transport instead of starting an HTTP
server.

```rust
// bins/rag-mcp.rs/src/main.rs  (after the change)
use anyhow::Result;
use rag_app_runtime::build_container;
use rag_mcp_api::build_router;
use rmcp::transport::stdio;

#[tokio::main]
async fn main() -> Result<()> {
    // When running under docker mcp gateway, all logging must go to stderr.
    // The gateway reads stdout as JSON-RPC; anything else there breaks the protocol.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,rag_mcp_rs=debug".to_string()),
        )
        .json()
        .init();

    let container = build_container()?;
    let transport = std::env::var("TRANSPORT").unwrap_or_else(|_| "http".to_string());

    match transport.as_str() {
        "stdio" => {
            // Gateway mode: communicate over stdin/stdout.
            // No port binding, no HTTP server.
            use rag_mcp_api::build_handler;
            let handler = build_handler(
                container.ingest_service,
                container.extract_service,
                container.query_service,
                container.repository,
            );
            let server = rmcp::ServiceExt::serve(handler, stdio()).await?;
            server.waiting().await?;
        }
        _ => {
            // Direct HTTP mode: current behaviour.
            let app = build_router(
                container.ingest_service,
                container.extract_service,
                container.query_service,
                container.repository,
            );
            let bind_addr = std::env::var("RAG_MCP_BIND_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8081".to_string());
            let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
            tracing::info!(%bind_addr, "starting rag-mcp (http)");
            axum::serve(listener, app).await?;
        }
    }

    Ok(())
}
```

`build_handler` is a new public function in `crates/mcp-api/src/lib.rs` that
constructs the `RagMcpHandler` without wrapping it in an Axum router — the
handler is what `rmcp::ServiceExt::serve` needs directly.

**Important:** In stdio mode, the process must never write anything to stdout
except valid JSON-RPC messages. All `tracing` output must go to stderr. The
`.with_writer(std::io::stderr)` call above handles this.

### 2.5 Step 2 — Add the `io.docker.server.metadata` image label

For the gateway to run the image without a catalog entry (using
`docker mcp gateway run --server docker://...`), the image must carry a
`io.docker.server.metadata` label containing a JSON blob that describes the
server.

Add this to the `rag-mcp-runtime` stage in `Dockerfile`:

```dockerfile
FROM runtime-base AS rag-mcp-runtime

# ... existing COPY and RUN lines ...

LABEL io.docker.server.metadata='{ \
  "name": "rag-mcp", \
  "description": "RAG MCP server — ingest, query, delete, and list assets backed by Qdrant", \
  "command": [], \
  "env": [ \
    {"name": "QDRANT_URL",         "value": "{{rag-mcp.qdrant_url}}"}, \
    {"name": "OPENAI_BASE_URL",    "value": "{{rag-mcp.openai_base_url}}"}, \
    {"name": "OPENAI_EMBED_MODEL", "value": "{{rag-mcp.openai_embed_model}}"}, \
    {"name": "QDRANT_COLLECTION",  "value": "{{rag-mcp.qdrant_collection}}"}, \
    {"name": "QDRANT_VECTOR_SIZE", "value": "{{rag-mcp.qdrant_vector_size}}"}, \
    {"name": "TRANSPORT",          "value": "stdio"} \
  ], \
  "secrets": [ \
    {"name": "rag-mcp.openai_api_key", "env": "OPENAI_API_KEY", "example": "sk-..."} \
  ] \
}'
```

The `{{rag-mcp.property}}` template syntax is how the gateway injects
user-supplied configuration values at runtime. The gateway reads these from
`docker mcp profile config` or `docker mcp secret set` commands.

### 2.6 Step 3 — Write the server entry YAML

The server entry YAML is the canonical way to register a custom server with the
gateway using the Profiles system (the current, non-deprecated approach). Create
this file at `docs/rag-mcp-server-entry.yaml`:

```yaml
# docs/rag-mcp-server-entry.yaml
#
# Server entry for docker/mcp-gateway Profiles.
# Usage:
#   docker mcp profile create --name rag \
#     --server file://./docs/rag-mcp-server-entry.yaml
#   docker mcp gateway run --profile rag

name: rag-mcp
title: RAG MCP
type: server
description: >
  Retrieval-Augmented Generation MCP server. Ingests text into a Qdrant vector
  database, embeds it via an OpenAI-compatible provider, and exposes semantic
  search, delete, and list operations as MCP tools.
image: ghcr.io/adorsys-gis/ai-helm/rag-mcp:latest

tools:
  - name: ingest_asset
    description: Chunk, embed, and index text content into the RAG vector store.
  - name: extract_asset_text
    description: Extract plain text from content without writing to the vector store.
  - name: query_asset
    description: Semantic similarity search over a single indexed asset.
  - name: query_assets
    description: Semantic similarity search across multiple indexed assets.
  - name: delete_assets
    description: Delete all indexed chunks for one or more assets.
  - name: list_assets
    description: List all assets indexed in the vector store for a given scope.

# Secrets — stored in Docker Desktop's secure secret store via:
#   docker mcp secret set rag-mcp.openai_api_key=sk-...
secrets:
  - name: rag-mcp.openai_api_key
    env: OPENAI_API_KEY
    example: sk-your-key-here

# Environment variables — set via:
#   docker mcp profile config <profile-id> --set rag-mcp.qdrant_url=http://qdrant:6334
env:
  - name: QDRANT_URL
    value: "{{rag-mcp.qdrant_url}}"
  - name: OPENAI_BASE_URL
    value: "{{rag-mcp.openai_base_url}}"
  - name: OPENAI_EMBED_MODEL
    value: "{{rag-mcp.openai_embed_model}}"
  - name: QDRANT_COLLECTION
    value: "{{rag-mcp.qdrant_collection}}"
  - name: QDRANT_VECTOR_SIZE
    value: "{{rag-mcp.qdrant_vector_size}}"
  - name: TRANSPORT
    value: "stdio"

# Configuration schema — defines what values the user must supply
config:
  - name: rag-mcp
    description: RAG MCP server configuration
    type: object
    properties:
      qdrant_url:
        type: string
        description: "gRPC URL of the Qdrant instance (e.g. http://qdrant:6334)"
      openai_base_url:
        type: string
        description: "Base URL of the OpenAI-compatible embedding provider"
      openai_embed_model:
        type: string
        description: "Embedding model name (e.g. text-embedding-3-small, nomic-embed-text)"
      qdrant_collection:
        type: string
        description: "Qdrant collection name (must match the embedding model's dimensions)"
      qdrant_vector_size:
        type: string
        description: "Embedding vector dimensions (e.g. 1536 for text-embedding-3-small, 768 for nomic-embed-text)"
    required:
      - qdrant_url
      - openai_base_url
      - openai_embed_model
      - qdrant_collection
      - qdrant_vector_size

metadata:
  category: AI
  tags: [rag, vector-search, qdrant, embeddings]
  license: MIT
  owner: adorsys-gis
```

### 2.7 Step 4 — Register with the gateway

Once the image is built with the stdio transport and the label, and the server
entry YAML exists, register it with the gateway using Profiles (the current
approach — the old `catalog` commands are deprecated when the profiles feature
flag is on):

```bash
# Enable profiles if not already enabled (only needed once, not required in Docker Desktop)
docker mcp feature enable profiles

# Create a profile that includes our RAG MCP server
docker mcp profile create --name rag \
  --server file://./docs/rag-mcp-server-entry.yaml

# Supply the required configuration values
docker mcp profile config rag \
  --set rag-mcp.qdrant_url=http://host.docker.internal:6334 \
  --set rag-mcp.openai_base_url=http://host.docker.internal:11434/v1 \
  --set rag-mcp.openai_embed_model=nomic-embed-text \
  --set rag-mcp.qdrant_collection=chunks_nomic \
  --set rag-mcp.qdrant_vector_size=768

# Supply the API key as a secret (stored securely, never in plain config)
docker mcp secret set rag-mcp.openai_api_key=ollama

# Verify the profile looks correct
docker mcp profile show rag --format yaml

# Dry-run to validate without starting
docker mcp gateway run --profile rag --dry-run

# Start the gateway with the RAG profile
docker mcp gateway run --profile rag
```

The gateway will pull the image if needed, spawn the container with
`docker run -i`, inject the configured env vars, and pipe JSON-RPC over stdio.

**Note on Qdrant:** The gateway runs the MCP container in isolation. Qdrant must
be reachable from inside that container. If you are running Qdrant via
`compose.mcp-test.yml`, use `host.docker.internal:6334` as the URL (the
`host.docker.internal` hostname resolves to the Docker host from inside any
container on Linux when `extra_hosts: ["host.docker.internal:host-gateway"]` is
set, or automatically on Docker Desktop for Mac/Windows).

### 2.8 Step 5 — Connect a client

Once the gateway is running, connect your AI client to it. The gateway exposes
a single aggregated MCP interface that multiplexes all servers in the profile.

**stdio mode (default — for Claude Desktop, Cursor, etc.):**

```json
{
  "mcpServers": {
    "MCP_DOCKER": {
      "command": "docker",
      "args": ["mcp", "gateway", "run", "--profile", "rag"]
    }
  }
}
```

**Streaming HTTP mode (for agents that prefer HTTP):**

```bash
# Start the gateway in streaming mode on port 8080
docker mcp gateway run --profile rag --transport streaming --port 8080
```

```json
{
  "mcpServers": {
    "rag-gateway": {
      "url": "http://localhost:8080/mcp"
    }
  }
}
```

You can also connect a client directly without editing config files:

```bash
docker mcp client connect cursor --profile rag
```

This writes the correct entry into Cursor's MCP config automatically.

### 2.9 Qdrant sidecar problem

The gateway runs each MCP server as an isolated container. It does not
orchestrate sidecars — it has no concept of "also start a Qdrant container
alongside this one". This means Qdrant must be running and reachable
independently before the gateway starts the MCP container.

**Options:**

**Option A — Keep Qdrant running separately (simplest)**

Run `docker compose -f compose.mcp-test.yml up qdrant -d` to keep Qdrant
running as a background service, then use the gateway for the MCP server. The
MCP container connects to Qdrant via `host.docker.internal:6334`.

**Option B — Use the gateway's Docker Compose support**

The gateway can itself run inside a Compose stack. This lets you define Qdrant
as a service alongside the gateway:

```yaml
# compose.gateway.yml
services:
  qdrant:
    image: qdrant/qdrant:v1.13.6
    ports:
      - "6334:6334"
    volumes:
      - qdrant_data:/qdrant/storage

  mcp-gateway:
    image: docker/mcp-gateway
    command:
      - --servers=docker://ghcr.io/adorsys-gis/ai-helm/rag-mcp:latest
      - --transport=streaming
      - --port=8080
    environment:
      QDRANT_URL: http://qdrant:6334
      OPENAI_BASE_URL: "${OPENAI_BASE_URL}"
      OPENAI_API_KEY: "${OPENAI_API_KEY}"
      OPENAI_EMBED_MODEL: "${OPENAI_EMBED_MODEL}"
      QDRANT_COLLECTION: "${QDRANT_COLLECTION}"
      QDRANT_VECTOR_SIZE: "${QDRANT_VECTOR_SIZE}"
      TRANSPORT: stdio
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
    depends_on:
      - qdrant
    ports:
      - "8080:8080"

volumes:
  qdrant_data:
```

In this setup the gateway container has access to the Docker socket and spawns
the MCP server container itself. The `QDRANT_URL` env var is passed through to
the spawned container. The client connects to `http://localhost:8080/mcp`.

**Option C — Long-lived container mode**

Set `longLived: true` in the server entry YAML. This tells the gateway to keep
the MCP container running rather than spawning it on demand. Combined with a
network that includes Qdrant, this is the cleanest production topology.

---

*Sources: [docker/mcp-gateway README](https://github.com/docker/mcp-gateway),
[mcp-gateway.md](https://github.com/docker/mcp-gateway/blob/main/docs/mcp-gateway.md),
[profiles.md](https://github.com/docker/mcp-gateway/blob/main/docs/profiles.md),
[catalog.md](https://github.com/docker/mcp-gateway/blob/main/docs/catalog.md),
[server-entry-spec.md](https://github.com/docker/mcp-gateway/blob/main/docs/server-entry-spec.md),
[self-configured.md](https://github.com/docker/mcp-gateway/blob/main/docs/self-configured.md)*
