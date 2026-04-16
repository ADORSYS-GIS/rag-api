# RAG Compatibility Layer: Design Specification

This document defines the contract and boundaries for the first iteration of the Rust-based RAG compatibility layer (`legacy-proxy`).

## 1. Core Objectives
- **Protocol Parity**: Replicate the exact HTTP interface expected by legacy clients (LibreChat).
- **Domain Independence**: Maintain a thin translation layer with zero direct ownership of embedding or vector storage logic.
- **Stability**: Preserve existing backend behavior while migrating to the new Rust runtime.

## 2. Request/Response Contracts

### A. Ingestion (`POST /embed` and `/embed-upload`)
Used for full document processing into the RAG system.

- **Request**: `multipart/form-data`
    - `file_id`: UUID/String (Required)
    - `file`: Binary file data (Required)
- **Mapping**: Calls `SimpleIngestService::ingest`.
- **Response**: `application/json`
    ```json
    {
      "status": "success",
      "file_id": "...",
      "filename": "...",
      "message": "legacy embed completed"
    }
    ```

### B. Extraction (`POST /text`)
Used for text extraction only, without updating the vector database.

- **Request**: `multipart/form-data`
    - `file_id`: UUID/String (Required)
    - `file`: Binary file data (Required)
- **Mapping**: Calls `SimpleExtractService::extract`.
- **Response**: `application/json`
    ```json
    {
      "status": "success",
      "content": "extracted text content..."
    }
    ```

## 3. Service Boundaries

| Feature | `/text` (Extraction) | `/embed` (Ingestion) |
| :--- | :---: | :---: |
| Text Extraction | ✅ | ✅ |
| Embedding Generation | ❌ | ✅ |
| Vector DB Storage | ❌ | ✅ |
| Searchable in RAG | ❌ | ✅ |

### Key Constraints:
- **Non-Side-Effecting**: The `/text` endpoint MUST remain stateless and NOT write to Qdrant.
- **Preserved Schema**: Vector dimensions (e.g., 768 for Ollama/Nomic) must match the collection configuration.

## 4. Non-Goals for First Interaction
- **Hybrid Search**: MVP focuses on pure vector retrieval; keyword/semantic hybrid logic is deferred.
- **Auth Layer**: Authentication is handled by proxy/gateway layers; the compatibility crate assumes an authenticated context.
- **Custom Chunking**: Uses system default chunking sizes; per-request overrides are not supported in the legacy contract.

## 5. Next Steps
- **UI Validation**: Verify LibreChat file uploads using the proxy.
- **Search Parity**: Implement `/query` compatibility to complete the retrieval cycle.
