use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Json as ExtractJson, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use rag_core::{
    ActorId, AssetId, BatchQueryRequest, CoreError, ExtractRequest, ExtractService, IngestRequest,
    IngestService, Namespace, QueryRequest, QueryService, RequestContext, Scope, SourceType,
    TenantId,
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
        .route("/v1/assets:ingest", post(ingest))
        .route("/v1/assets:extract", post(extract))
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

#[derive(Debug, Deserialize)]
struct IngestBody {
    tenant_id: Option<String>,
    namespace: Option<String>,
    asset_id: String,
    source_type: String,
    source_uri: Option<String>,
    content: Option<String>,
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtractBody {
    tenant_id: Option<String>,
    namespace: Option<String>,
    source_type: String,
    source_uri: Option<String>,
    content: Option<String>,
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

async fn ingest(
    State(state): State<HttpApiState>,
    headers: HeaderMap,
    ExtractJson(body): ExtractJson<IngestBody>,
) -> impl IntoResponse {
    let tenant_id = body
        .tenant_id
        .or_else(|| header_string(&headers, "x-tenant-id"))
        .unwrap_or_else(|| "public".to_string());
    let namespace = body
        .namespace
        .or_else(|| header_string(&headers, "x-namespace"))
        .unwrap_or_else(|| "default".to_string());

    let source_type = match SourceType::parse(&body.source_type) {
        Some(value) => value,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"invalid source_type"})),
            )
                .into_response();
        }
    };

    let content = match body.content.as_ref().filter(|v| !v.trim().is_empty()) {
        Some(value) => value.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"content cannot be empty"})),
            )
                .into_response();
        }
    };

    let scope = Scope {
        tenant_id: TenantId(tenant_id.clone()),
        namespace: Namespace(namespace.clone()),
    };

    let ctx = RequestContext {
        tenant_id: TenantId(tenant_id.clone()),
        actor_id: header_string(&headers, "x-actor-id").map(ActorId),
        roles: vec![],
        allowed_namespaces: vec![scope.namespace.clone()],
        request_id: header_string(&headers, "x-request-id")
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
    };

    let request = IngestRequest {
        scope,
        asset_id: AssetId(body.asset_id.clone()),
        source_type,
        source_uri: body.source_uri.clone(),
        content: Some(content),
        mime_type: body.mime_type.clone(),
    };

    match state.ingest_service.ingest(ctx, request).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => core_error_to_response(err),
    }
}

async fn extract(
    State(state): State<HttpApiState>,
    headers: HeaderMap,
    ExtractJson(body): ExtractJson<ExtractBody>,
) -> impl IntoResponse {
    let tenant_id = body
        .tenant_id
        .or_else(|| header_string(&headers, "x-tenant-id"))
        .unwrap_or_else(|| "public".to_string());
    let namespace = body
        .namespace
        .or_else(|| header_string(&headers, "x-namespace"))
        .unwrap_or_else(|| "default".to_string());

    let source_type = match SourceType::parse(&body.source_type) {
        Some(value) => value,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"invalid source_type"})),
            )
                .into_response();
        }
    };

    let content = match body.content.as_ref().filter(|v| !v.trim().is_empty()) {
        Some(value) => value.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"content cannot be empty"})),
            )
                .into_response();
        }
    };

    let scope = Scope {
        tenant_id: TenantId(tenant_id.clone()),
        namespace: Namespace(namespace.clone()),
    };

    let ctx = RequestContext {
        tenant_id: TenantId(tenant_id),
        actor_id: header_string(&headers, "x-actor-id").map(ActorId),
        roles: vec![],
        allowed_namespaces: vec![scope.namespace.clone()],
        request_id: header_string(&headers, "x-request-id")
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
    };

    let request = ExtractRequest {
        scope,
        source_type,
        source_uri: body.source_uri.clone(),
        content: Some(content),
    };

    match state.extract_service.extract(ctx, request).await {
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

    struct DummyIngest {
        seen_request: Arc<Mutex<Option<IngestRequest>>>,
    }
    struct DummyExtract {
        seen_request: Arc<Mutex<Option<ExtractRequest>>>,
    }
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
            *self.seen_request.lock().unwrap() = Some(request.clone());
            Ok(IngestResponse {
                asset_id: request.asset_id,
                chunks_written: 1,
            })
        }
    }

    #[async_trait]
    impl ExtractService for DummyExtract {
        async fn extract(
            &self,
            _ctx: RequestContext,
            request: ExtractRequest,
        ) -> Result<ExtractResponse, CoreError> {
            *self.seen_request.lock().unwrap() = Some(request.clone());
            Ok(ExtractResponse {
                text: request.content.unwrap_or_else(|| "ok".to_string()),
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
            ingest_service: Arc::new(DummyIngest {
                seen_request: Arc::new(Mutex::new(None)),
            }),
            extract_service: Arc::new(DummyExtract {
                seen_request: Arc::new(Mutex::new(None)),
            }),
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
            ingest_service: Arc::new(DummyIngest {
                seen_request: Arc::new(Mutex::new(None)),
            }),
            extract_service: Arc::new(DummyExtract {
                seen_request: Arc::new(Mutex::new(None)),
            }),
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

    #[tokio::test]
    async fn post_v1_assets_ingest_returns_ok() {
        let seen_ingest = Arc::new(Mutex::new(None));
        let state = HttpApiState {
            ingest_service: Arc::new(DummyIngest {
                seen_request: seen_ingest.clone(),
            }),
            extract_service: Arc::new(DummyExtract {
                seen_request: Arc::new(Mutex::new(None)),
            }),
            query_service: Arc::new(DummyQuery {
                seen_batch: Arc::new(Mutex::new(None)),
            }),
        };
        let app = router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/assets:ingest")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{
                    "tenant_id":"public",
                    "namespace":"default",
                    "asset_id":"asset-1",
                    "source_type":"text",
                    "content":"payload"
                }"#
                .to_string(),
            ))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let captured = seen_ingest.lock().unwrap().clone().unwrap();
        assert_eq!(captured.asset_id.0, "asset-1");
        assert_eq!(captured.scope.namespace.0, "default");
    }

    #[tokio::test]
    async fn post_v1_assets_extract_returns_text() {
        let seen_extract = Arc::new(Mutex::new(None));
        let state = HttpApiState {
            ingest_service: Arc::new(DummyIngest {
                seen_request: Arc::new(Mutex::new(None)),
            }),
            extract_service: Arc::new(DummyExtract {
                seen_request: seen_extract.clone(),
            }),
            query_service: Arc::new(DummyQuery {
                seen_batch: Arc::new(Mutex::new(None)),
            }),
        };
        let app = router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/assets:extract")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{
                    "tenant_id":"public",
                    "namespace":"default",
                    "source_type":"text",
                    "content":"payload"
                }"#
                .to_string(),
            ))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["text"], "payload");
    }
}
