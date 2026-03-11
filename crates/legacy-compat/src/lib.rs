use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Json as ExtractJson, State},
    http::HeaderMap,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use rag_core::{
    ActorId, AssetId, BatchQueryRequest, CoreError, ExtractService, IngestService, Namespace,
    QueryRequest, QueryService, RequestContext, Scope, TenantId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct LegacyCompatState {
    pub ingest_service: Arc<dyn IngestService>,
    pub extract_service: Arc<dyn ExtractService>,
    pub query_service: Arc<dyn QueryService>,
}

pub fn router(state: LegacyCompatState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ids", get(not_implemented))
        .route("/documents", get(not_implemented).delete(not_implemented))
        .route("/documents/{id}/context", get(not_implemented))
        .route("/embed", post(not_implemented))
        .route("/embed-upload", post(not_implemented))
        .route("/local/embed", post(not_implemented))
        .route("/query", post(query))
        .route("/query_multiple", post(query_multiple))
        .route("/text", post(not_implemented))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status":"UP"}))
}

async fn not_implemented(State(_state): State<LegacyCompatState>) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "message": "legacy endpoint scaffolded but not implemented"
        })),
    )
}

#[derive(Debug, Deserialize)]
struct LegacyQueryBody {
    query: String,
    file_id: String,
    k: Option<usize>,
    entity_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LegacyQueryMultipleBody {
    query: String,
    file_ids: Vec<String>,
    k: Option<usize>,
    entity_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct LegacyDocument {
    page_content: String,
    metadata: serde_json::Value,
}

async fn query(
    State(state): State<LegacyCompatState>,
    headers: HeaderMap,
    ExtractJson(body): ExtractJson<LegacyQueryBody>,
) -> impl IntoResponse {
    if body.query.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"detail":"query cannot be empty"})),
        )
            .into_response();
    }

    let tenant_id = header_string(&headers, "x-tenant-id")
        .or_else(|| std::env::var("LEGACY_DEFAULT_TENANT").ok())
        .unwrap_or_else(|| "public".to_string());
    let namespace = header_string(&headers, "x-namespace")
        .or_else(|| std::env::var("LEGACY_DEFAULT_NAMESPACE").ok())
        .unwrap_or_else(|| "librechat".to_string());

    let scope = Scope {
        tenant_id: TenantId(tenant_id.clone()),
        namespace: Namespace(namespace.clone()),
    };

    let actor_id = body
        .entity_id
        .clone()
        .or_else(|| header_string(&headers, "x-actor-id"));

    let ctx = RequestContext {
        tenant_id: TenantId(tenant_id),
        actor_id: actor_id.clone().map(ActorId),
        roles: vec![],
        allowed_namespaces: vec![Namespace(namespace)],
        request_id: header_string(&headers, "x-request-id")
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
    };

    let request = QueryRequest {
        scope,
        query: body.query,
        asset_id: Some(AssetId(body.file_id.clone())),
        k: body.k.unwrap_or(4),
    };

    match state.query_service.query(ctx, request).await {
        Ok(response) => Json(into_legacy_matches(response.matches, actor_id)).into_response(),
        Err(err) => core_error_to_response(err),
    }
}

async fn query_multiple(
    State(state): State<LegacyCompatState>,
    headers: HeaderMap,
    ExtractJson(body): ExtractJson<LegacyQueryMultipleBody>,
) -> impl IntoResponse {
    if body.query.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"detail":"query cannot be empty"})),
        )
            .into_response();
    }

    if body.file_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"detail":"file_ids cannot be empty"})),
        )
            .into_response();
    }

    let tenant_id = header_string(&headers, "x-tenant-id")
        .or_else(|| std::env::var("LEGACY_DEFAULT_TENANT").ok())
        .unwrap_or_else(|| "public".to_string());
    let namespace = header_string(&headers, "x-namespace")
        .or_else(|| std::env::var("LEGACY_DEFAULT_NAMESPACE").ok())
        .unwrap_or_else(|| "librechat".to_string());

    let scope = Scope {
        tenant_id: TenantId(tenant_id.clone()),
        namespace: Namespace(namespace.clone()),
    };

    let actor_id = body
        .entity_id
        .clone()
        .or_else(|| header_string(&headers, "x-actor-id"));

    let ctx = RequestContext {
        tenant_id: TenantId(tenant_id),
        actor_id: actor_id.clone().map(ActorId),
        roles: vec![],
        allowed_namespaces: vec![Namespace(namespace)],
        request_id: header_string(&headers, "x-request-id")
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
    };

    let request = BatchQueryRequest {
        scope,
        query: body.query,
        asset_ids: body.file_ids.into_iter().map(AssetId).collect(),
        k: body.k.unwrap_or(4),
    };

    match state.query_service.query_batch(ctx, request).await {
        Ok(response) => Json(into_legacy_matches(response.matches, actor_id)).into_response(),
        Err(err) => core_error_to_response(err),
    }
}

fn into_legacy_matches(
    matches: Vec<rag_core::ScoredChunk>,
    actor_id: Option<String>,
) -> Vec<(LegacyDocument, f32)> {
    matches
        .into_iter()
        .map(|m| {
            let metadata = serde_json::json!({
                "file_id": m.chunk.asset_id.0,
                "user_id": m.chunk.actor_id.map(|a| a.0).or(actor_id.clone()),
                "digest": m.chunk.digest,
                "chunk_index": m.chunk.chunk_index,
                "source_type": m.chunk.source_type.as_str(),
                "source_uri": m.chunk.source_uri,
                "page": m.chunk.page,
                "path": m.chunk.path,
                "language": m.chunk.language,
                "mime_type": m.chunk.mime_type,
                "title": m.chunk.title,
                "tags": m.chunk.tags,
            });
            (
                LegacyDocument {
                    page_content: m.chunk.text,
                    metadata,
                },
                m.score,
            )
        })
        .collect()
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
            "detail": err.to_string()
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
        QueryRequest, QueryResponse, QueryService, RequestContext, ScoredChunk, SourceType,
    };
    use tower::ServiceExt;

    use super::{LegacyCompatState, router};

    struct DummyIngest;
    struct DummyExtract;
    struct DummyQuery {
        seen_request: Arc<Mutex<Option<QueryRequest>>>,
        seen_batch_request: Arc<Mutex<Option<BatchQueryRequest>>>,
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
            *self.seen_request.lock().unwrap() = Some(request.clone());
            let chunk = ChunkRecord {
                tenant_id: request.scope.tenant_id,
                namespace: request.scope.namespace,
                asset_id: request.asset_id.unwrap(),
                actor_id: None,
                source_type: SourceType::Text,
                source_uri: None,
                digest: "digest".to_string(),
                chunk_index: 0,
                page: None,
                path: None,
                language: None,
                mime_type: None,
                title: None,
                text: "legacy response".to_string(),
                embedding: vec![],
                tags: vec![],
                created_at: chrono::Utc::now(),
            };
            Ok(QueryResponse {
                matches: vec![ScoredChunk { chunk, score: 0.8 }],
            })
        }

        async fn query_batch(
            &self,
            _ctx: RequestContext,
            request: BatchQueryRequest,
        ) -> Result<BatchQueryResponse, CoreError> {
            *self.seen_batch_request.lock().unwrap() = Some(request.clone());

            let chunk = ChunkRecord {
                tenant_id: request.scope.tenant_id,
                namespace: request.scope.namespace,
                asset_id: request.asset_ids[0].clone(),
                actor_id: None,
                source_type: SourceType::Text,
                source_uri: None,
                digest: "digest-batch".to_string(),
                chunk_index: 0,
                page: None,
                path: None,
                language: None,
                mime_type: None,
                title: None,
                text: "legacy batch response".to_string(),
                embedding: vec![],
                tags: vec![],
                created_at: chrono::Utc::now(),
            };
            Ok(BatchQueryResponse {
                matches: vec![ScoredChunk { chunk, score: 0.6 }],
            })
        }
    }

    #[tokio::test]
    async fn post_legacy_query_translates_file_id_and_shape() {
        let seen = Arc::new(Mutex::new(None));
        let seen_batch = Arc::new(Mutex::new(None));
        let state = LegacyCompatState {
            ingest_service: Arc::new(DummyIngest),
            extract_service: Arc::new(DummyExtract),
            query_service: Arc::new(DummyQuery {
                seen_request: seen.clone(),
                seen_batch_request: seen_batch.clone(),
            }),
        };
        let app = router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/query")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"hello","file_id":"abc-123","k":4}"#.to_string(),
            ))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let captured = seen.lock().unwrap().clone().unwrap();
        assert_eq!(captured.asset_id.unwrap().0, "abc-123");

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.is_array());
        assert_eq!(value[0][0]["page_content"], "legacy response");
        assert_eq!(value[0][0]["metadata"]["file_id"], "abc-123");
    }

    #[tokio::test]
    async fn post_legacy_query_multiple_translates_file_ids_and_shape() {
        let seen = Arc::new(Mutex::new(None));
        let seen_batch = Arc::new(Mutex::new(None));
        let state = LegacyCompatState {
            ingest_service: Arc::new(DummyIngest),
            extract_service: Arc::new(DummyExtract),
            query_service: Arc::new(DummyQuery {
                seen_request: seen.clone(),
                seen_batch_request: seen_batch.clone(),
            }),
        };
        let app = router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/query_multiple")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"query":"hello","file_ids":["abc-123","xyz-999"],"k":5}"#.to_string(),
            ))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.is_array());
        assert_eq!(value[0][0]["page_content"], "legacy batch response");
        assert_eq!(value[0][0]["metadata"]["file_id"], "abc-123");

        let captured_batch = seen_batch.lock().unwrap().clone().unwrap();
        assert_eq!(captured_batch.asset_ids.len(), 2);
        assert_eq!(captured_batch.asset_ids[0].0, "abc-123");
        assert_eq!(captured_batch.k, 5);
    }
}
