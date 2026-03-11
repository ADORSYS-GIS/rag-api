use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use rag_core::{
    AssetId, BatchQueryRequest, BatchQueryResponse, ChunkRecord, CoreError, ExtractRequest,
    ExtractResponse, IngestRequest, IngestResponse, IngestService, QueryRequest, QueryResponse,
    QueryService, RequestContext, ScoredChunk, SourceType, ExtractService, ActorId,
};

#[derive(Clone)]
pub struct AppContainer {
    pub ingest_service: Arc<dyn IngestService>,
    pub extract_service: Arc<dyn ExtractService>,
    pub query_service: Arc<dyn QueryService>,
}

pub fn build_container() -> AppContainer {
    AppContainer {
        ingest_service: Arc::new(NoopIngestService),
        extract_service: Arc::new(NoopExtractService),
        query_service: Arc::new(SampleQueryService),
    }
}

struct NoopIngestService;

#[async_trait]
impl IngestService for NoopIngestService {
    async fn ingest(
        &self,
        _ctx: RequestContext,
        request: IngestRequest,
    ) -> Result<IngestResponse, CoreError> {
        Ok(IngestResponse {
            asset_id: request.asset_id,
            chunks_written: 0,
        })
    }
}

struct NoopExtractService;

#[async_trait]
impl ExtractService for NoopExtractService {
    async fn extract(
        &self,
        _ctx: RequestContext,
        _request: ExtractRequest,
    ) -> Result<ExtractResponse, CoreError> {
        Ok(ExtractResponse {
            text: String::new(),
        })
    }
}

struct SampleQueryService;

#[async_trait]
impl QueryService for SampleQueryService {
    async fn query(
        &self,
        ctx: RequestContext,
        request: QueryRequest,
    ) -> Result<QueryResponse, CoreError> {
        if request.query.trim().is_empty() {
            return Err(CoreError::Validation(
                "query cannot be empty".to_string(),
            ));
        }

        let asset_id = request
            .asset_id
            .unwrap_or_else(|| AssetId("default-asset".to_string()));

        let chunk = ChunkRecord {
            tenant_id: request.scope.tenant_id,
            namespace: request.scope.namespace,
            asset_id: asset_id.clone(),
            actor_id: ctx.actor_id.or(Some(ActorId("system".to_string()))),
            source_type: SourceType::Text,
            source_uri: None,
            digest: format!("sample-{}", asset_id.0),
            chunk_index: 0,
            page: None,
            path: None,
            language: None,
            mime_type: Some("text/plain".to_string()),
            title: Some("sample".to_string()),
            text: format!("sample match for: {}", request.query),
            embedding: vec![],
            tags: vec!["sample".to_string()],
            created_at: Utc::now(),
        };

        Ok(QueryResponse {
            matches: vec![ScoredChunk { chunk, score: 0.9 }],
        })
    }

    async fn query_batch(
        &self,
        _ctx: RequestContext,
        request: BatchQueryRequest,
    ) -> Result<BatchQueryResponse, CoreError> {
        if request.query.trim().is_empty() {
            return Err(CoreError::Validation(
                "query cannot be empty".to_string(),
            ));
        }

        let now = Utc::now();
        let mut matches = Vec::new();
        for (idx, asset_id) in request.asset_ids.into_iter().enumerate() {
            matches.push(ScoredChunk {
                chunk: ChunkRecord {
                    tenant_id: request.scope.tenant_id.clone(),
                    namespace: request.scope.namespace.clone(),
                    asset_id: asset_id.clone(),
                    actor_id: None,
                    source_type: SourceType::Text,
                    source_uri: None,
                    digest: format!("sample-{}-{}", asset_id.0, idx),
                    chunk_index: idx as u32,
                    page: None,
                    path: None,
                    language: None,
                    mime_type: Some("text/plain".to_string()),
                    title: Some("sample".to_string()),
                    text: format!("sample match for: {}", request.query),
                    embedding: vec![],
                    tags: vec!["sample".to_string()],
                    created_at: now,
                },
                score: 0.9,
            });
        }

        Ok(BatchQueryResponse { matches })
    }
}
