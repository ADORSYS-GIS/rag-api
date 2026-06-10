# MCP Gateway — Guide

The `mcp-gateway` binary solves the host-filesystem-path problem for MCP clients that
run the server in Docker. When a client passes a path like `/home/user/report.pdf`, a
containerised server cannot access it — the file lives outside the container.

The gateway fixes this by mounting the entire host filesystem read-only at `/host` inside
the container and prepending that mount point to every local path before delegating to the
extractor. The client never has to know.

---

## How it works

```
Client                  mcp-gateway (container)           SourceExtractService
  │                            │                                    │
  │  source_uri=/home/u/f.pdf  │                                    │
  │ ─────────────────────────► │                                    │
  │                            │  /host/home/u/f.pdf                │
  │                            │ ─────────────────────────────────► │
  │                            │          extracted text            │
  │                            │ ◄───────────────────────────────── │
  │       tool response        │                                    │
  │ ◄───────────────────────── │
```

`GATEWAY_HOST_ROOT=/host` is prepended to every absolute path that is not an
`http://`, `https://`, or `file://` URI. HTTP URIs pass through unchanged.
Setting `GATEWAY_HOST_ROOT` to an empty string disables remapping (useful when
running the binary natively on the host).

---

## Environment variables

| Variable | Default | Notes |
|---|---|---|
| `GATEWAY_TRANSPORT` | `http` | `http` (port 8090) or `stdio` (Claude Desktop) |
| `GATEWAY_BIND_ADDR` | `0.0.0.0:8090` | HTTP mode only. |
| `GATEWAY_HOST_ROOT` | _(empty)_ | Prepended to local paths. Set to `/host` when using the compose volume mount. |
| `QDRANT_URL` | `http://localhost:6334` | |
| `QDRANT_COLLECTION` | `chunks_te3_small` | |
| `QDRANT_VECTOR_SIZE` | `1536` | Must match your embedding model. |
| `OPENAI_BASE_URL` | `https://api.openai.com` | Ollama: `http://host.docker.internal:11434/v1` |
| `OPENAI_API_KEY` | _(empty)_ | Required for OpenAI; leave blank for Ollama. |
| `OPENAI_EMBED_MODEL` | `text-embedding-3-small` | Ollama: `nomic-embed-text` |

All other `INGEST_*`, `QUERY_*`, and `OPENAI_*` tuning knobs from the main
`RuntimeConfig` are also respected.

---

## Running with Docker Compose

The `compose.yaml` at the repo root includes a `mcp_gateway` service pre-configured
with the `/:/host:ro` volume mount. Start the full stack:

```bash
cp .env.example .env
# Edit .env — fill in OPENAI_API_KEY / OPENAI_BASE_URL / OPENAI_EMBED_MODEL
# and adjust QDRANT_VECTOR_SIZE to match your model.

docker compose up --build -d
```

The gateway is available at `http://localhost:8090/mcp`.

---

## Connecting Claude Desktop (stdio mode)

For Claude Desktop you want the gateway to speak stdio rather than HTTP, so Claude
can launch it directly without a running server.

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "rag-gateway": {
      "command": "docker",
      "args": [
        "run", "--rm", "-i",
        "-v", "/:/host:ro",
        "-e", "GATEWAY_TRANSPORT=stdio",
        "-e", "GATEWAY_HOST_ROOT=/host",
        "-e", "OPENAI_BASE_URL=http://host.docker.internal:11434/v1",
        "-e", "OPENAI_EMBED_MODEL=nomic-embed-text",
        "-e", "QDRANT_URL=http://host.docker.internal:6334",
        "-e", "QDRANT_VECTOR_SIZE=768",
        "-e", "QDRANT_COLLECTION=chunks_nomic",
        "--add-host", "host.docker.internal:host-gateway",
        "mcp-gateway:latest"
      ]
    }
  }
}
```

For OpenAI embeddings swap:

```
"-e", "OPENAI_BASE_URL=https://api.openai.com",
"-e", "OPENAI_API_KEY=sk-...",
"-e", "OPENAI_EMBED_MODEL=text-embedding-3-small",
"-e", "QDRANT_VECTOR_SIZE=1536",
"-e", "QDRANT_COLLECTION=chunks_te3_small",
```

---

## Passing local file paths

Because `GATEWAY_HOST_ROOT=/host` is set, you pass **host-side absolute paths**
exactly as they appear on your machine:

```json
{ "asset_id": "my-doc", "source_uri": "/home/koufan/docs/report.pdf" }
```

The gateway rewrites this to `/host/home/koufan/docs/report.pdf` before reading it
inside the container. No need to use `file:///` — bare absolute paths work.

HTTP/S URIs are always passed through unchanged:

```json
{ "asset_id": "rust-book", "source_uri": "https://doc.rust-lang.org/book/" }
```

---

## Building the image manually

```bash
docker build --target mcp-gateway-runtime -t mcp-gateway:latest .
```

---

## Health check (HTTP mode)

```bash
curl http://localhost:8090/healthz
# → {"status":"ok","service":"rag-mcp"}
```

The gateway reuses the same `/healthz` and `/readyz` endpoints as `rag-mcp`.
