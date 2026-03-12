use std::{env, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use hex::encode;
use rag_core::{
    AssetId, BatchQueryRequest, BatchQueryResponse, ChunkRecord, ChunkRepository, CoreError,
    EmbeddingClient, ExtractRequest, ExtractResponse, ExtractService, IngestRequest,
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
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub qdrant: QdrantRepositoryConfig,
    pub openai: OpenAiCompatConfig,
    pub embedding_model: String,
    pub query_top_k_default: usize,
    pub query_top_k_max: usize,
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

    Ok(AppContainer {
        ingest_service: Arc::new(SimpleIngestService {
            repository: repository.clone(),
            embeddings: embedding_client.clone(),
            model: config.embedding_model,
        }),
        extract_service: Arc::new(SimpleExtractService),
        query_service,
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

struct SimpleIngestService {
    repository: Arc<dyn ChunkRepository>,
    embeddings: Arc<dyn EmbeddingClient>,
    model: String,
}

#[async_trait]
impl IngestService for SimpleIngestService {
    async fn ingest(
        &self,
        ctx: RequestContext,
        request: IngestRequest,
    ) -> Result<IngestResponse, CoreError> {
        let text = request
            .content
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| CoreError::Validation("ingest content cannot be empty".to_string()))?;

        let embedding = self
            .embeddings
            .embed_texts(&self.model, std::slice::from_ref(text))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::Provider("missing embedding vector".to_string()))?;

        let chunk = ChunkRecord {
            tenant_id: request.scope.tenant_id.clone(),
            namespace: request.scope.namespace.clone(),
            asset_id: request.asset_id.clone(),
            actor_id: ctx.actor_id.clone(),
            source_type: request.source_type,
            source_uri: request.source_uri.clone(),
            digest: chunk_digest(&request.scope, &request.asset_id, text, 0),
            chunk_index: 0,
            page: None,
            path: None,
            language: None,
            mime_type: request.mime_type.clone(),
            title: None,
            text: text.clone(),
            embedding,
            tags: vec![],
            created_at: Utc::now(),
        };

        let summary = self
            .repository
            .upsert_chunks(&request.scope, vec![chunk])
            .await?;

        Ok(IngestResponse {
            asset_id: request.asset_id,
            chunks_written: summary.points_written,
        })
    }
}

fn chunk_digest(scope: &Scope, asset_id: &AssetId, text: &str, chunk_index: u32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope.tenant_id.0.as_bytes());
    hasher.update(scope.namespace.0.as_bytes());
    hasher.update(asset_id.0.as_bytes());
    hasher.update(chunk_index.to_le_bytes());
    hasher.update(text.as_bytes());
    encode(hasher.finalize())
}

struct SimpleExtractService;

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
