# Testing Guide: RAG Ingestion Flow

This guide outlines the steps to verify the ingestion flow using local Ollama embeddings and the `legacy-proxy`.

## 1. Prerequisites: Ollama Setup
On the host machine, Ollama must be configured to allow connections from Docker containers.

1.  **Stop Ollama**:
    ```bash
    pkill ollama  # or sudo systemctl stop ollama
    ```
2.  **Start Ollama with 0.0.0.0**:
    ```bash
    OLLAMA_HOST=0.0.0.0 ollama serve
    ```
3.  **Pull Embedding Model**:
    ```bash
    OLLAMA_HOST=0.0.0.0 ollama pull nomic-embed-text
    ```

## 2. System Configuration
Ensure your `.env` and `compose.yaml` match the current verified state:

- **.env**:
    - `OPENAI_BASE_URL=http://host.docker.internal:11434/v1`
    - `OPENAI_EMBED_MODEL=nomic-embed-text`
    - `QDRANT_VECTOR_SIZE=768`
    - `QDRANT_COLLECTION=chunks_nomic`

- **compose.yaml**:
    - `extra_hosts` should include `"host.docker.internal:host-gateway"` for the `legacy_proxy` and `rag_api` services.

## 3. Testing Commands

### A. Manual Ingestion (The "Success" Path)
Use this command to verify that text extraction, embedding, and storage are all working.
```bash
curl -X POST http://localhost:8000/embed \
  -F "file_id=verify-$(date +%s)" \
  -F "file=@/path/to/your/document.txt"
```
**Expected Response**: `{"status":"success", "message":"legacy embed completed", ...}`

### B. Verification via Qdrant
Check that the point was actually stored in the vector database.
```bash
curl -s -X POST http://localhost:6333/collections/chunks_nomic/points/count \
  -H "Content-Type: application/json" \
  -d '{"exact": true}'
```
**Expected Result**: `"count": X` (where X increases with each test).

### C. Extraction-Only Test (No Side Effects)
Verify that the legacy `/text` endpoint remains extraction-only.
```bash
curl -X POST http://localhost:8000/text \
  -F "file_id=test-extract" \
  -F "file=@/path/to/your/document.txt"
```
**Expected Result**: Plain text content returned; **no** new points in Qdrant.

## 4. Monitoring
Monitor the logs to troubleshoot any provider or networking issues:
```bash
docker compose logs -f legacy_proxy
```
