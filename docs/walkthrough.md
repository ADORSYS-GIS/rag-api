# Ingestion Flow Verification Walkthrough

We have successfully verified the full RAG ingestion flow through the `legacy-proxy` using a local Ollama instance for embeddings.

## Changes Made

### 1. Environment Configuration
- Updated [.env](file:///home/koufan/rag-api/.env) to use local Ollama:
    - `OPENAI_BASE_URL=http://host.docker.internal:11434/v1`
    - `OPENAI_EMBED_MODEL=nomic-embed-text`
    - `QDRANT_VECTOR_SIZE=768`
    - `QDRANT_COLLECTION=chunks_nomic`

### 2. Service Networking
- Modified [compose.yaml](file:///home/koufan/rag-api/compose.yaml) to add `extra_hosts` to `legacy_proxy` and `rag_api`, allowing them to reach the host's Ollama instance via `host.docker.internal`.

## Verification Results

### 1. Endpoint Check
- Confirmed that `/text` is intentionally extraction-only per the project roadmap.
- Verified that `/embed` is the correct endpoint for full ingestion.

### 2. End-to-End Ingestion Test
Performed ingestion via `curl`:
```bash
curl -X POST http://localhost:8000/embed \
  -F "file_id=test-final-1776357994" \
  -F "file=@test_final.txt"
```
**Result**: `{"status":"success","message":"legacy embed completed"}`

### 3. Vector Storage Check
Verified that the vectors are correctly stored in Qdrant:
- **Collection**: `chunks_nomic`
- **Vector Dimension**: 768
- **Point Count**: 1

```json
{
  "result": {
    "count": 1
  },
  "status": "ok"
}
```

## Conclusion
The `legacy-proxy` implementation correctly preserves the existing backend behavior while providing a clear path for ingestion through the `/embed` endpoint. The system is now robust against OpenAI quota limits by utilizing local Ollama embeddings.
