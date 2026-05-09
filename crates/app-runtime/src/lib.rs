use std::{env, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::{self, StreamExt};
use hex::encode;
use rag_core::{
    AssetId, BatchQueryRequest, BatchQueryResponse, Chunk, ChunkRecord, ChunkRepository, Chunker,
    CoreError, EmbeddingClient, ExtractRequest, ExtractResponse, ExtractService, IngestRequest,
    IngestResponse, IngestService, QueryCache, QueryRequest, QueryResponse, QueryService,
    RequestContext, Scope, SearchRequest,
};
use rag_openai_compat::{OpenAiCompatClient, OpenAiCompatConfig};
use rag_storage_qdrant::{QdrantChunkRepository, QdrantRepositoryConfig};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct AppContainer {
    pub ingest_service: Arc<dyn IngestService>,
    pub extract_service: Arc<dyn ExtractService>,
    pub query_service: Arc<dyn QueryService>,
    /// Exposed so MCP tools that need direct repository access (delete, list)
    /// can be wired without going through a service trait.
    pub repository: Arc<dyn ChunkRepository>,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub qdrant: QdrantRepositoryConfig,
    pub openai: OpenAiCompatConfig,
    pub embedding_model: String,
    pub query_top_k_default: usize,
    pub query_top_k_max: usize,
    /// Number of embedding batches to send to the provider concurrently.
    /// Higher values reduce wall-clock ingestion time at the cost of more
    /// simultaneous HTTP connections. Defaults to 4.
    pub embed_concurrency: usize,
    /// Number of chunks per embedding request. Defaults to 50.
    pub embed_batch_size: usize,
    /// Number of Qdrant upsert calls to issue concurrently. Defaults to 4.
    pub upsert_concurrency: usize,
    /// Number of chunks per Qdrant upsert call. Defaults to 100.
    pub upsert_batch_size: usize,
}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self, CoreError> {
        let qdrant_vector_size = parse_u64_env("QDRANT_VECTOR_SIZE", 1536)?;
        if qdrant_vector_size == 0 {
            return Err(CoreError::Validation(
                "QDRANT_VECTOR_SIZE must be greater than 0".to_string(),
            ));
        }

        let query_top_k_default = parse_usize_env("QUERY_TOP_K_DEFAULT", 4)?;
        let query_top_k_max = parse_usize_env("QUERY_TOP_K_MAX", 20)?;
        if query_top_k_default == 0 || query_top_k_default > query_top_k_max {
            return Err(CoreError::Validation(
                "QUERY_TOP_K_DEFAULT must be between 1 and QUERY_TOP_K_MAX".to_string(),
            ));
        }

        let openai_timeout_ms = parse_u64_env("OPENAI_TIMEOUT_MS", 30_000)?;
        if openai_timeout_ms == 0 {
            return Err(CoreError::Validation(
                "OPENAI_TIMEOUT_MS must be greater than 0".to_string(),
            ));
        }

        let embed_concurrency = parse_usize_env("INGEST_EMBED_CONCURRENCY", 4)?;
        let embed_batch_size = parse_usize_env("INGEST_EMBED_BATCH_SIZE", 50)?;
        let upsert_concurrency = parse_usize_env("INGEST_UPSERT_CONCURRENCY", 4)?;
        let upsert_batch_size = parse_usize_env("INGEST_UPSERT_BATCH_SIZE", 100)?;

        if embed_concurrency == 0 {
            return Err(CoreError::Validation(
                "INGEST_EMBED_CONCURRENCY must be greater than 0".to_string(),
            ));
        }
        if embed_batch_size == 0 {
            return Err(CoreError::Validation(
                "INGEST_EMBED_BATCH_SIZE must be greater than 0".to_string(),
            ));
        }
        if upsert_concurrency == 0 {
            return Err(CoreError::Validation(
                "INGEST_UPSERT_CONCURRENCY must be greater than 0".to_string(),
            ));
        }
        if upsert_batch_size == 0 {
            return Err(CoreError::Validation(
                "INGEST_UPSERT_BATCH_SIZE must be greater than 0".to_string(),
            ));
        }

        Ok(Self {
            qdrant: QdrantRepositoryConfig {
                url: env::var("QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".to_string()),
                api_key: env::var("QDRANT_API_KEY").ok(),
                collection_name: env::var("QDRANT_COLLECTION")
                    .unwrap_or_else(|_| "chunks_te3_small".to_string()),
                vector_size: qdrant_vector_size,
            },
            openai: OpenAiCompatConfig {
                base_url: env::var("OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com".to_string()),
                api_key: env::var("OPENAI_API_KEY").ok(),
                timeout_ms: openai_timeout_ms,
                max_retries: parse_u32_env("OPENAI_MAX_RETRIES", 2)?,
            },
            embedding_model: env::var("OPENAI_EMBED_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".to_string()),
            query_top_k_default,
            query_top_k_max,
            embed_concurrency,
            embed_batch_size,
            upsert_concurrency,
            upsert_batch_size,
        })
    }
}

pub fn build_container() -> Result<AppContainer, CoreError> {
    let config = RuntimeConfig::from_env()?;
    tracing::info!(
        qdrant_url = %config.qdrant.url,
        qdrant_collection = %config.qdrant.collection_name,
        qdrant_vector_size = config.qdrant.vector_size,
        embedding_model = %config.embedding_model,
        embed_concurrency = config.embed_concurrency,
        embed_batch_size = config.embed_batch_size,
        upsert_concurrency = config.upsert_concurrency,
        upsert_batch_size = config.upsert_batch_size,
        "initializing runtime container"
    );

    let repository: Arc<dyn ChunkRepository> =
        Arc::new(QdrantChunkRepository::new(config.qdrant.clone())?);
    let embedding_client: Arc<dyn EmbeddingClient> =
        Arc::new(OpenAiCompatClient::new(config.openai.clone())?);

    let query_service = Arc::new(RuntimeQueryService {
        repository: repository.clone(),
        embeddings: embedding_client.clone(),
        cache: None,
        model: config.embedding_model.clone(),
        default_k: config.query_top_k_default,
        max_k: config.query_top_k_max,
    });

    let chunker = Arc::new(RecursiveChunker::new(1000, 200));

    Ok(AppContainer {
        ingest_service: Arc::new(ParallelIngestService {
            repository: repository.clone(),
            embeddings: embedding_client.clone(),
            chunker,
            model: config.embedding_model,
            embed_concurrency: config.embed_concurrency,
            embed_batch_size: config.embed_batch_size,
            upsert_concurrency: config.upsert_concurrency,
            upsert_batch_size: config.upsert_batch_size,
        }),
        extract_service: Arc::new(SimpleExtractService),
        query_service,
        repository,
    })
}

fn parse_u64_env(name: &str, default: u64) -> Result<u64, CoreError> {
    match env::var(name) {
        Ok(raw) => raw
            .parse::<u64>()
            .map_err(|e| CoreError::Validation(format!("{name} must be a valid u64: {e}"))),
        Err(_) => Ok(default),
    }
}

fn parse_u32_env(name: &str, default: u32) -> Result<u32, CoreError> {
    match env::var(name) {
        Ok(raw) => raw
            .parse::<u32>()
            .map_err(|e| CoreError::Validation(format!("{name} must be a valid u32: {e}"))),
        Err(_) => Ok(default),
    }
}

fn parse_usize_env(name: &str, default: usize) -> Result<usize, CoreError> {
    match env::var(name) {
        Ok(raw) => raw
            .parse::<usize>()
            .map_err(|e| CoreError::Validation(format!("{name} must be a valid usize: {e}"))),
        Err(_) => Ok(default),
    }
}

/// High-throughput ingest service.
///
/// Improvements over the original sequential implementation:
///
/// 1. **Larger embedding batches** — configurable via `INGEST_EMBED_BATCH_SIZE`
///    (default 50 vs the old 20), reducing HTTP round-trips by ~60 %.
/// 2. **Concurrent embedding requests** — `INGEST_EMBED_CONCURRENCY` (default 4)
///    batches are sent to the provider simultaneously using `futures::stream`
///    buffered concurrency, saturating the provider's throughput.
/// 3. **Pipelined Qdrant upserts** — once all embeddings for a window of batches
///    are ready, upserts are issued concurrently (`INGEST_UPSERT_CONCURRENCY`,
///    default 4) with configurable point counts per call
///    (`INGEST_UPSERT_BATCH_SIZE`, default 100).
///
/// For a 1 MB text file (~1 250 chunks) against a local Ollama model the
/// expected wall-clock improvement is roughly 4–6× compared to the old
/// sequential 20-chunk loop.
struct ParallelIngestService {
    repository: Arc<dyn ChunkRepository>,
    embeddings: Arc<dyn EmbeddingClient>,
    chunker: Arc<dyn Chunker>,
    model: String,
    embed_concurrency: usize,
    embed_batch_size: usize,
    upsert_concurrency: usize,
    upsert_batch_size: usize,
}

#[async_trait]
impl IngestService for ParallelIngestService {
    async fn ingest(
        &self,
        ctx: RequestContext,
        request: IngestRequest,
    ) -> Result<IngestResponse, CoreError> {
        let text = request
            .content
            .as_ref()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| CoreError::Validation("ingest content cannot be empty".to_string()))?;

        let chunks = self.chunker.chunk_text(text);
        let total_chunks = chunks.len();

        tracing::info!(
            asset_id = %request.asset_id.0,
            total_chunks,
            embed_batch_size = self.embed_batch_size,
            embed_concurrency = self.embed_concurrency,
            upsert_batch_size = self.upsert_batch_size,
            upsert_concurrency = self.upsert_concurrency,
            "starting parallel ingest"
        );

        // ── Step 1: embed all chunks concurrently ────────────────────────────
        //
        // Split chunks into batches, then run up to `embed_concurrency` batches
        // in parallel. Each batch produces a Vec<Vec<f32>> of embeddings.
        let embed_batches: Vec<Vec<Chunk>> = chunks
            .chunks(self.embed_batch_size)
            .map(|b| b.to_vec())
            .collect();

        let embeddings_client = self.embeddings.clone();
        let model = self.model.clone();

        let embedded_batches: Vec<(Vec<Chunk>, Vec<Vec<f32>>)> = stream::iter(embed_batches)
            .map(|batch| {
                let client = embeddings_client.clone();
                let model = model.clone();
                async move {
                    let texts: Vec<String> = batch.iter().map(|c| c.text.clone()).collect();
                    let vectors = client.embed_texts(&model, &texts).await?;
                    if vectors.len() != batch.len() {
                        return Err(CoreError::Provider(format!(
                            "embedding count mismatch: expected {}, got {}",
                            batch.len(),
                            vectors.len()
                        )));
                    }
                    Ok((batch, vectors))
                }
            })
            .buffer_unordered(self.embed_concurrency)
            .collect::<Vec<Result<_, CoreError>>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, CoreError>>()?;

        // ── Step 2: build ChunkRecords ────────────────────────────────────────
        let mut records: Vec<ChunkRecord> = Vec::with_capacity(total_chunks);
        for (batch, vectors) in embedded_batches {
            for (chunk, embedding) in batch.into_iter().zip(vectors) {
                records.push(ChunkRecord {
                    tenant_id: request.scope.tenant_id.clone(),
                    namespace: request.scope.namespace.clone(),
                    asset_id: request.asset_id.clone(),
                    actor_id: ctx.actor_id.clone(),
                    source_type: request.source_type.clone(),
                    source_uri: request.source_uri.clone(),
                    digest: chunk_digest(
                        &request.scope,
                        &request.asset_id,
                        &chunk.text,
                        chunk.chunk_index,
                    ),
                    chunk_index: chunk.chunk_index,
                    page: None,
                    path: None,
                    language: None,
                    mime_type: request.mime_type.clone(),
                    title: None,
                    text: chunk.text,
                    embedding,
                    tags: vec![],
                    created_at: Utc::now(),
                });
            }
        }

        // ── Step 3: upsert to Qdrant concurrently ────────────────────────────
        let upsert_batches: Vec<Vec<ChunkRecord>> = records
            .chunks(self.upsert_batch_size)
            .map(|b| b.to_vec())
            .collect();

        let repository = self.repository.clone();
        let scope = request.scope.clone();

        let points_written: usize = stream::iter(upsert_batches)
            .map(|batch| {
                let repo = repository.clone();
                let scope = scope.clone();
                async move {
                    let summary = repo.upsert_chunks(&scope, batch).await?;
                    Ok::<usize, CoreError>(summary.points_written)
                }
            })
            .buffer_unordered(self.upsert_concurrency)
            .collect::<Vec<Result<usize, CoreError>>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, CoreError>>()?
            .into_iter()
            .sum();

        tracing::info!(
            asset_id = %request.asset_id.0,
            points_written,
            "parallel ingest complete"
        );

        Ok(IngestResponse {
            asset_id: request.asset_id,
            chunks_written: points_written,
        })
    }
}

pub struct RecursiveChunker {
    chunk_size: usize,
    chunk_overlap: usize,
}

impl RecursiveChunker {
    pub fn new(chunk_size: usize, chunk_overlap: usize) -> Self {
        Self {
            chunk_size,
            chunk_overlap,
        }
    }
}

impl Chunker for RecursiveChunker {
    fn chunk_text(&self, text: &str) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut start = 0;
        let mut index = 0;

        while start < chars.len() {
            let end = (start + self.chunk_size).min(chars.len());
            let chunk_text: String = chars[start..end].iter().collect();

            chunks.push(Chunk {
                text: chunk_text,
                chunk_index: index,
            });
            index += 1;

            if end == chars.len() {
                break;
            }

            // Advance by (chunk_size - overlap)
            start += self.chunk_size.saturating_sub(self.chunk_overlap).max(1);
        }
        chunks
    }
}

struct SimpleExtractService;

fn chunk_digest(scope: &Scope, asset_id: &AssetId, text: &str, chunk_index: u32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope.tenant_id.0.as_bytes());
    hasher.update(scope.namespace.0.as_bytes());
    hasher.update(asset_id.0.as_bytes());
    hasher.update(chunk_index.to_le_bytes());
    hasher.update(text.as_bytes());
    encode(hasher.finalize())
}

#[async_trait]
impl ExtractService for SimpleExtractService {
    async fn extract(
        &self,
        _ctx: RequestContext,
        request: ExtractRequest,
    ) -> Result<ExtractResponse, CoreError> {
        let text = request
            .content
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CoreError::Validation("extract content cannot be empty".to_string()))?;

        Ok(ExtractResponse { text })
    }
}

struct RuntimeQueryService {
    repository: Arc<dyn ChunkRepository>,
    embeddings: Arc<dyn EmbeddingClient>,
    cache: Option<Arc<dyn QueryCache>>,
    model: String,
    default_k: usize,
    max_k: usize,
}

impl RuntimeQueryService {
    fn validate_scope_access(&self, ctx: &RequestContext, scope: &Scope) -> Result<(), CoreError> {
        if ctx.tenant_id != scope.tenant_id {
            return Err(CoreError::Forbidden);
        }

        if !ctx.allowed_namespaces.is_empty()
            && !ctx
                .allowed_namespaces
                .iter()
                .any(|ns| ns == &scope.namespace)
        {
            return Err(CoreError::Forbidden);
        }

        Ok(())
    }

    fn normalize_k(&self, requested_k: usize) -> Result<usize, CoreError> {
        let k = if requested_k == 0 {
            self.default_k
        } else {
            requested_k
        };

        if k > self.max_k {
            return Err(CoreError::Validation(format!(
                "k cannot exceed QUERY_TOP_K_MAX ({})",
                self.max_k
            )));
        }
        Ok(k)
    }

    async fn embedding_for_query(&self, query: &str) -> Result<Vec<f32>, CoreError> {
        if let Some(cache) = &self.cache {
            let key = format!("embed:{}:{}", self.model, query);
            if let Some(vector) = cache.get_query_embedding(&key).await? {
                return Ok(vector);
            }

            let vector = self.embeddings.embed_query(&self.model, query).await?;
            cache.put_query_embedding(&key, vector.clone(), 300).await?;
            return Ok(vector);
        }

        self.embeddings.embed_query(&self.model, query).await
    }
}

#[async_trait]
impl QueryService for RuntimeQueryService {
    async fn query(
        &self,
        ctx: RequestContext,
        request: QueryRequest,
    ) -> Result<QueryResponse, CoreError> {
        if request.query.trim().is_empty() {
            return Err(CoreError::Validation("query cannot be empty".to_string()));
        }

        self.validate_scope_access(&ctx, &request.scope)?;
        let k = self.normalize_k(request.k)?;
        let query_vector = self.embedding_for_query(&request.query).await?;
        let asset_ids = request.asset_id.into_iter().collect::<Vec<AssetId>>();

        let matches = self
            .repository
            .search(
                &request.scope,
                SearchRequest {
                    query_vector,
                    k,
                    asset_ids,
                },
            )
            .await?;

        Ok(QueryResponse { matches })
    }

    async fn query_batch(
        &self,
        ctx: RequestContext,
        request: BatchQueryRequest,
    ) -> Result<BatchQueryResponse, CoreError> {
        if request.query.trim().is_empty() {
            return Err(CoreError::Validation("query cannot be empty".to_string()));
        }

        if request.asset_ids.is_empty() {
            return Err(CoreError::Validation(
                "asset_ids cannot be empty for batch query".to_string(),
            ));
        }

        self.validate_scope_access(&ctx, &request.scope)?;
        let k = self.normalize_k(request.k)?;
        let query_vector = self.embedding_for_query(&request.query).await?;

        let matches = self
            .repository
            .search(
                &request.scope,
                SearchRequest {
                    query_vector,
                    k,
                    asset_ids: request.asset_ids,
                },
            )
            .await?;

        Ok(BatchQueryResponse { matches })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use rag_core::{
        AssetFilter, AssetSummary, ChunkRecord, DeleteSummary, Namespace, ScoredChunk, TenantId,
        UpsertSummary,
    };

    use super::*;

    struct FakeRepo {
        seen_scope: Arc<Mutex<Option<Scope>>>,
        seen_search: Arc<Mutex<Option<SearchRequest>>>,
    }

    #[async_trait]
    impl ChunkRepository for FakeRepo {
        async fn upsert_chunks(
            &self,
            _scope: &Scope,
            _chunks: Vec<ChunkRecord>,
        ) -> Result<UpsertSummary, CoreError> {
            Ok(UpsertSummary { points_written: 0 })
        }

        async fn delete_asset(
            &self,
            _scope: &Scope,
            _asset_id: &AssetId,
        ) -> Result<DeleteSummary, CoreError> {
            Ok(DeleteSummary { points_deleted: 0 })
        }

        async fn get_asset_chunks(
            &self,
            _scope: &Scope,
            _asset_id: &AssetId,
        ) -> Result<Vec<ChunkRecord>, CoreError> {
            Ok(vec![])
        }

        async fn list_assets(
            &self,
            _scope: &Scope,
            _filter: AssetFilter,
        ) -> Result<Vec<AssetSummary>, CoreError> {
            Ok(vec![])
        }

        async fn search(
            &self,
            scope: &Scope,
            request: SearchRequest,
        ) -> Result<Vec<ScoredChunk>, CoreError> {
            *self.seen_scope.lock().unwrap() = Some(scope.clone());
            *self.seen_search.lock().unwrap() = Some(request);
            Ok(vec![])
        }
    }

    struct FakeEmbed;

    #[async_trait]
    impl EmbeddingClient for FakeEmbed {
        async fn embed_texts(
            &self,
            _model: &str,
            _inputs: &[String],
        ) -> Result<Vec<Vec<f32>>, CoreError> {
            Ok(vec![vec![0.1, 0.2, 0.3]])
        }

        async fn embed_query(&self, _model: &str, _input: &str) -> Result<Vec<f32>, CoreError> {
            Ok(vec![0.1, 0.2, 0.3])
        }
    }

    #[tokio::test]
    async fn query_uses_embedding_and_scoped_search() {
        let seen_scope = Arc::new(Mutex::new(None));
        let seen_search = Arc::new(Mutex::new(None));
        let service = RuntimeQueryService {
            repository: Arc::new(FakeRepo {
                seen_scope: seen_scope.clone(),
                seen_search: seen_search.clone(),
            }),
            embeddings: Arc::new(FakeEmbed),
            cache: None,
            model: "text-embedding-3-small".to_string(),
            default_k: 4,
            max_k: 20,
        };

        let ctx = RequestContext {
            tenant_id: TenantId("tenant-a".to_string()),
            actor_id: None,
            roles: vec![],
            allowed_namespaces: vec![Namespace("default".to_string())],
            request_id: "req-1".to_string(),
        };
        let scope = Scope {
            tenant_id: TenantId("tenant-a".to_string()),
            namespace: Namespace("default".to_string()),
        };
        let request = QueryRequest {
            scope: scope.clone(),
            query: "hello".to_string(),
            asset_id: Some(AssetId("asset-1".to_string())),
            k: 4,
        };

        let response = service.query(ctx, request).await.unwrap();
        assert!(response.matches.is_empty());

        let captured_scope = seen_scope.lock().unwrap().clone().unwrap();
        assert_eq!(captured_scope.tenant_id.0, "tenant-a");
        assert_eq!(captured_scope.namespace.0, "default");

        let captured_search = seen_search.lock().unwrap().clone().unwrap();
        assert_eq!(captured_search.k, 4);
        assert_eq!(captured_search.asset_ids.len(), 1);
        assert_eq!(captured_search.asset_ids[0].0, "asset-1");
        assert_eq!(captured_search.query_vector, vec![0.1, 0.2, 0.3]);
    }

    #[tokio::test]
    async fn query_rejects_tenant_mismatch() {
        let service = RuntimeQueryService {
            repository: Arc::new(FakeRepo {
                seen_scope: Arc::new(Mutex::new(None)),
                seen_search: Arc::new(Mutex::new(None)),
            }),
            embeddings: Arc::new(FakeEmbed),
            cache: None,
            model: "text-embedding-3-small".to_string(),
            default_k: 4,
            max_k: 20,
        };

        let ctx = RequestContext {
            tenant_id: TenantId("tenant-a".to_string()),
            actor_id: None,
            roles: vec![],
            allowed_namespaces: vec![Namespace("default".to_string())],
            request_id: "req-1".to_string(),
        };
        let request = QueryRequest {
            scope: Scope {
                tenant_id: TenantId("tenant-b".to_string()),
                namespace: Namespace("default".to_string()),
            },
            query: "hello".to_string(),
            asset_id: None,
            k: 4,
        };

        let err = service.query(ctx, request).await.unwrap_err();
        assert!(matches!(err, CoreError::Forbidden));
    }

    #[tokio::test]
    async fn query_rejects_k_over_limit() {
        let service = RuntimeQueryService {
            repository: Arc::new(FakeRepo {
                seen_scope: Arc::new(Mutex::new(None)),
                seen_search: Arc::new(Mutex::new(None)),
            }),
            embeddings: Arc::new(FakeEmbed),
            cache: None,
            model: "text-embedding-3-small".to_string(),
            default_k: 4,
            max_k: 5,
        };

        let ctx = RequestContext {
            tenant_id: TenantId("tenant-a".to_string()),
            actor_id: None,
            roles: vec![],
            allowed_namespaces: vec![Namespace("default".to_string())],
            request_id: "req-1".to_string(),
        };
        let request = QueryRequest {
            scope: Scope {
                tenant_id: TenantId("tenant-a".to_string()),
                namespace: Namespace("default".to_string()),
            },
            query: "hello".to_string(),
            asset_id: None,
            k: 6,
        };

        let err = service.query(ctx, request).await.unwrap_err();
        assert!(matches!(err, CoreError::Validation(_)));
    }

    #[tokio::test]
    async fn batch_query_requires_asset_ids() {
        let service = RuntimeQueryService {
            repository: Arc::new(FakeRepo {
                seen_scope: Arc::new(Mutex::new(None)),
                seen_search: Arc::new(Mutex::new(None)),
            }),
            embeddings: Arc::new(FakeEmbed),
            cache: None,
            model: "text-embedding-3-small".to_string(),
            default_k: 4,
            max_k: 5,
        };

        let ctx = RequestContext {
            tenant_id: TenantId("tenant-a".to_string()),
            actor_id: None,
            roles: vec![],
            allowed_namespaces: vec![Namespace("default".to_string())],
            request_id: "req-1".to_string(),
        };
        let request = BatchQueryRequest {
            scope: Scope {
                tenant_id: TenantId("tenant-a".to_string()),
                namespace: Namespace("default".to_string()),
            },
            query: "hello".to_string(),
            asset_ids: vec![],
            k: 4,
        };

        let err = service.query_batch(ctx, request).await.unwrap_err();
        assert!(matches!(err, CoreError::Validation(_)));
    }
}
