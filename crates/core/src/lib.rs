use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TenantId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Namespace(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AssetId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ActorId(pub String);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scope {
    pub tenant_id: TenantId,
    pub namespace: Namespace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestContext {
    pub tenant_id: TenantId,
    pub actor_id: Option<ActorId>,
    pub roles: Vec<String>,
    pub allowed_namespaces: Vec<Namespace>,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceType {
    Upload,
    LocalFile,
    Website,
    Pdf,
    Image,
    Code,
    Text,
    IdeBuffer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    pub scope: Scope,
    pub asset_id: AssetId,
    pub source_type: SourceType,
    pub source_uri: Option<String>,
    pub content: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestResponse {
    pub asset_id: AssetId,
    pub chunks_written: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractRequest {
    pub scope: Scope,
    pub source_type: SourceType,
    pub source_uri: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractResponse {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    pub scope: Scope,
    pub query: String,
    pub asset_id: Option<AssetId>,
    pub k: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchQueryRequest {
    pub scope: Scope,
    pub query: String,
    pub asset_ids: Vec<AssetId>,
    pub k: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub matches: Vec<ScoredChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchQueryResponse {
    pub matches: Vec<ScoredChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub tenant_id: TenantId,
    pub namespace: Namespace,
    pub asset_id: AssetId,
    pub actor_id: Option<ActorId>,
    pub source_type: SourceType,
    pub source_uri: Option<String>,
    pub digest: String,
    pub chunk_index: u32,
    pub page: Option<u32>,
    pub path: Option<String>,
    pub language: Option<String>,
    pub mime_type: Option<String>,
    pub title: Option<String>,
    pub text: String,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredChunk {
    pub chunk: ChunkRecord,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetSummary {
    pub asset_id: AssetId,
    pub chunk_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AssetFilter {
    pub source_type: Option<SourceType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query_vector: Vec<f32>,
    pub k: usize,
    pub asset_ids: Vec<AssetId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertSummary {
    pub points_written: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteSummary {
    pub points_deleted: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("provider error: {0}")]
    Provider(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("lock error: {0}")]
    Lock(String),
    #[error("not implemented: {0}")]
    NotImplemented(String),
}

#[async_trait]
pub trait ChunkRepository: Send + Sync {
    async fn upsert_chunks(
        &self,
        scope: &Scope,
        chunks: Vec<ChunkRecord>,
    ) -> Result<UpsertSummary, CoreError>;

    async fn delete_asset(
        &self,
        scope: &Scope,
        asset_id: &AssetId,
    ) -> Result<DeleteSummary, CoreError>;

    async fn get_asset_chunks(
        &self,
        scope: &Scope,
        asset_id: &AssetId,
    ) -> Result<Vec<ChunkRecord>, CoreError>;

    async fn list_assets(
        &self,
        scope: &Scope,
        filter: AssetFilter,
    ) -> Result<Vec<AssetSummary>, CoreError>;

    async fn search(
        &self,
        scope: &Scope,
        request: SearchRequest,
    ) -> Result<Vec<ScoredChunk>, CoreError>;
}

#[async_trait]
pub trait EmbeddingClient: Send + Sync {
    async fn embed_texts(&self, model: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>, CoreError>;

    async fn embed_query(&self, model: &str, input: &str) -> Result<Vec<f32>, CoreError>;
}

#[async_trait]
pub trait DocumentUnderstandingClient: Send + Sync {
    async fn describe_image(&self, uri: &str) -> Result<String, CoreError>;
}

#[async_trait]
pub trait AssetLockManager: Send + Sync {
    async fn acquire_asset_lock(
        &self,
        tenant_id: &TenantId,
        namespace: &Namespace,
        asset_id: &AssetId,
    ) -> Result<(), CoreError>;
}

#[async_trait]
pub trait QueryCache: Send + Sync {
    async fn get_query_embedding(&self, key: &str) -> Result<Option<Vec<f32>>, CoreError>;

    async fn put_query_embedding(&self, key: &str, vector: Vec<f32>, ttl_secs: u64)
        -> Result<(), CoreError>;
}

#[async_trait]
pub trait IngestService: Send + Sync {
    async fn ingest(
        &self,
        ctx: RequestContext,
        request: IngestRequest,
    ) -> Result<IngestResponse, CoreError>;
}

#[async_trait]
pub trait ExtractService: Send + Sync {
    async fn extract(
        &self,
        ctx: RequestContext,
        request: ExtractRequest,
    ) -> Result<ExtractResponse, CoreError>;
}

#[async_trait]
pub trait QueryService: Send + Sync {
    async fn query(
        &self,
        ctx: RequestContext,
        request: QueryRequest,
    ) -> Result<QueryResponse, CoreError>;

    async fn query_batch(
        &self,
        ctx: RequestContext,
        request: BatchQueryRequest,
    ) -> Result<BatchQueryResponse, CoreError>;
}
