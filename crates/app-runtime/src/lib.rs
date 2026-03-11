use std::sync::Arc;

use async_trait::async_trait;
use rag_core::{
    BatchQueryRequest, BatchQueryResponse, CoreError, ExtractRequest, ExtractResponse,
    IngestRequest, IngestResponse, IngestService, QueryRequest, QueryResponse, QueryService,
    RequestContext, ExtractService,
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
        query_service: Arc::new(NoopQueryService),
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

struct NoopQueryService;

#[async_trait]
impl QueryService for NoopQueryService {
    async fn query(
        &self,
        _ctx: RequestContext,
        _request: QueryRequest,
    ) -> Result<QueryResponse, CoreError> {
        Ok(QueryResponse { matches: vec![] })
    }

    async fn query_batch(
        &self,
        _ctx: RequestContext,
        _request: BatchQueryRequest,
    ) -> Result<BatchQueryResponse, CoreError> {
        Ok(BatchQueryResponse { matches: vec![] })
    }
}
