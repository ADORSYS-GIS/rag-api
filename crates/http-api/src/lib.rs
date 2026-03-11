use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Json as ExtractJson, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use rag_core::{
    ActorId, AssetId, BatchQueryRequest, CoreError, ExtractService, IngestService, Namespace,
    QueryRequest, QueryService, RequestContext, Scope, TenantId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct HttpApiState {
    pub ingest_service: Arc<dyn IngestService>,
    pub extract_service: Arc<dyn ExtractService>,
    pub query_service: Arc<dyn QueryService>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

pub fn router(state: HttpApiState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/assets:ingest", post(not_implemented))
        .route("/v1/assets:extract", post(not_implemented))
        .route("/v1/query", post(query))
        .route("/v1/query:batch", post(query_batch))
        .route("/v1/assets", get(not_implemented).delete(not_implemented))
        .route("/v1/assets/{asset_id}/chunks", get(not_implemented))
        .route("/v1/assets/{asset_id}/context", get(not_implemented))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        service: "rag-api.rs",
    })
}

async fn readyz(State(_state): State<HttpApiState>) -> impl IntoResponse {
    Json(HealthResponse {
        status: "ready",
        service: "rag-api.rs",
    })
}

async fn not_implemented() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "endpoint scaffolded but not implemented"
        })),
    )
}

#[derive(Debug, Deserialize)]
struct QueryBody {
    query: String,
    k: Option<usize>,
    asset_id: Option<String>,
    tenant_id: Option<String>,
    namespace: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QueryBatchBody {
    query: String,
    k: Option<usize>,
    asset_ids: Vec<String>,
    tenant_id: Option<String>,
    namespace: Option<String>,
}

async fn query(
    State(state): State<HttpApiState>,
    headers: HeaderMap,
    ExtractJson(body): ExtractJson<QueryBody>,
) -> impl IntoResponse {
    if body.query.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"query cannot be empty"})),
        )
            .into_response();
    }

    let scope = Scope {
        tenant_id: TenantId(
            body.tenant_id
                .or_else(|| header_string(&headers, "x-tenant-id"))
                .unwrap_or_else(|| "public".to_string()),
        ),
        namespace: Namespace(
            body.namespace
                .or_else(|| header_string(&headers, "x-namespace"))
                .unwrap_or_else(|| "default".to_string()),
        ),
    };

    let ctx = RequestContext {
        tenant_id: scope.tenant_id.clone(),
        actor_id: header_string(&headers, "x-actor-id").map(ActorId),
        roles: vec![],
        allowed_namespaces: vec![scope.namespace.clone()],
        request_id: header_string(&headers, "x-request-id")
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
    };

    let request = QueryRequest {
        scope,
        query: body.query,
        asset_id: body.asset_id.map(AssetId),
        k: body.k.unwrap_or(4),
    };

    match state.query_service.query(ctx, request).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => core_error_to_response(err),
    }
}

async fn query_batch(
    State(state): State<HttpApiState>,
    headers: HeaderMap,
    ExtractJson(body): ExtractJson<QueryBatchBody>,
) -> impl IntoResponse {
    if body.query.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"query cannot be empty"})),
        )
            .into_response();
    }

    if body.asset_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"asset_ids cannot be empty"})),
        )
            .into_response();
    }

    let scope = Scope {
        tenant_id: TenantId(
            body.tenant_id
                .or_else(|| header_string(&headers, "x-tenant-id"))
                .unwrap_or_else(|| "public".to_string()),
        ),
        namespace: Namespace(
            body.namespace
                .or_else(|| header_string(&headers, "x-namespace"))
                .unwrap_or_else(|| "default".to_string()),
        ),
    };

    let ctx = RequestContext {
        tenant_id: scope.tenant_id.clone(),
        actor_id: header_string(&headers, "x-actor-id").map(ActorId),
        roles: vec![],
        allowed_namespaces: vec![scope.namespace.clone()],
        request_id: header_string(&headers, "x-request-id")
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
    };

    let request = BatchQueryRequest {
        scope,
        query: body.query,
        asset_ids: body.asset_ids.into_iter().map(AssetId).collect(),
        k: body.k.unwrap_or(4),
    };

    match state.query_service.query_batch(ctx, request).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => core_error_to_response(err),
    }
}

fn header_string(headers: &HeaderMap, key: &str) -> Option<String> {
    headers
        .get(key)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn core_error_to_response(err: CoreError) -> axum::response::Response {
    let status = match err {
        CoreError::Validation(_) => StatusCode::BAD_REQUEST,
        CoreError::Unauthorized => StatusCode::UNAUTHORIZED,
        CoreError::Forbidden => StatusCode::FORBIDDEN,
        CoreError::NotFound => StatusCode::NOT_FOUND,
        CoreError::Provider(_) | CoreError::Storage(_) | CoreError::Lock(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        CoreError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
    };

    (
        status,
        Json(serde_json::json!({
            "error": err.to_string()
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use rag_core::{
        BatchQueryRequest, BatchQueryResponse, ChunkRecord, CoreError, ExtractRequest,
        ExtractResponse, ExtractService, IngestRequest, IngestResponse, IngestService,
        QueryRequest, QueryResponse, QueryService, RequestContext, ScoredChunk,
    };
    use tower::ServiceExt;

    use super::{HttpApiState, router};

    struct DummyIngest;
    struct DummyExtract;
    struct DummyQuery {
        seen_batch: Arc<Mutex<Option<BatchQueryRequest>>>,
    }

    #[async_trait]
    impl IngestService for DummyIngest {
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

    #[async_trait]
    impl ExtractService for DummyExtract {
        async fn extract(
            &self,
            _ctx: RequestContext,
            _request: ExtractRequest,
        ) -> Result<ExtractResponse, CoreError> {
            Ok(ExtractResponse {
                text: "ok".to_string(),
            })
        }
    }

    #[async_trait]
    impl QueryService for DummyQuery {
        async fn query(
            &self,
            _ctx: RequestContext,
            request: QueryRequest,
        ) -> Result<QueryResponse, CoreError> {
            let chunk = ChunkRecord {
                tenant_id: request.scope.tenant_id,
                namespace: request.scope.namespace,
                asset_id: request
                    .asset_id
                    .unwrap_or_else(|| rag_core::AssetId("a".to_string())),
                actor_id: None,
                source_type: rag_core::SourceType::Text,
                source_uri: None,
                digest: "d".to_string(),
                chunk_index: 0,
                page: None,
                path: None,
                language: None,
                mime_type: None,
                title: None,
                text: "hello".to_string(),
                embedding: vec![],
                tags: vec![],
                created_at: chrono::Utc::now(),
            };
            Ok(QueryResponse {
                matches: vec![ScoredChunk { chunk, score: 0.7 }],
            })
        }

        async fn query_batch(
            &self,
            _ctx: RequestContext,
            request: BatchQueryRequest,
        ) -> Result<BatchQueryResponse, CoreError> {
            *self.seen_batch.lock().unwrap() = Some(request.clone());

            let chunk = ChunkRecord {
                tenant_id: request.scope.tenant_id,
                namespace: request.scope.namespace,
                asset_id: request.asset_ids[0].clone(),
                actor_id: None,
                source_type: rag_core::SourceType::Text,
                source_uri: None,
                digest: "d-batch".to_string(),
                chunk_index: 0,
                page: None,
                path: None,
                language: None,
                mime_type: None,
                title: None,
                text: "hello-batch".to_string(),
                embedding: vec![],
                tags: vec![],
                created_at: chrono::Utc::now(),
            };
            Ok(BatchQueryResponse {
                matches: vec![ScoredChunk { chunk, score: 0.8 }],
            })
        }
    }

    #[tokio::test]
    async fn post_v1_query_returns_matches() {
        let seen_batch = Arc::new(Mutex::new(None));
        let state = HttpApiState {
            ingest_service: Arc::new(DummyIngest),
            extract_service: Arc::new(DummyExtract),
            query_service: Arc::new(DummyQuery {
                seen_batch: seen_batch.clone(),
            }),
        };
        let app = router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/query")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"hello","asset_id":"file-1","k":4}"#.to_string(),
            ))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["matches"].is_array());
        assert_eq!(value["matches"][0]["chunk"]["asset_id"], "file-1");
    }

    #[tokio::test]
    async fn post_v1_query_batch_returns_matches() {
        let seen_batch = Arc::new(Mutex::new(None));
        let state = HttpApiState {
            ingest_service: Arc::new(DummyIngest),
            extract_service: Arc::new(DummyExtract),
            query_service: Arc::new(DummyQuery {
                seen_batch: seen_batch.clone(),
            }),
        };
        let app = router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/query:batch")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"hello","asset_ids":["file-1","file-2"],"k":3}"#.to_string(),
            ))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["matches"].is_array());
        assert_eq!(value["matches"][0]["chunk"]["asset_id"], "file-1");

        let seen = seen_batch.lock().unwrap().clone().unwrap();
        assert_eq!(seen.asset_ids.len(), 2);
        assert_eq!(seen.k, 3);
    }
}
