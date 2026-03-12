use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Json as ExtractJson, Multipart, State},
    http::HeaderMap,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rag_core::{
    ActorId, AssetId, BatchQueryRequest, CoreError, ExtractRequest, ExtractService, IngestRequest,
    IngestService, Namespace, QueryRequest, QueryService, RequestContext, Scope, SourceType,
    TenantId,
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
        .route("/embed", post(embed))
        .route("/embed-upload", post(embed_upload))
        .route("/local/embed", post(local_embed))
        .route("/query", post(query))
        .route("/query_multiple", post(query_multiple))
        .route("/text", post(text))
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

#[derive(Debug, Deserialize)]
struct LegacyLocalEmbedBody {
    file_id: String,
    entity_id: Option<String>,
    content: Option<String>,
    known_type: Option<String>,
    path: Option<String>,
}

struct LegacyUploadPayload {
    file_id: Option<String>,
    entity_id: Option<String>,
    filename: Option<String>,
    known_type: Option<String>,
    content: Option<String>,
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

async fn embed(
    State(state): State<LegacyCompatState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> impl IntoResponse {
    legacy_multipart_ingest(&state, &headers, multipart, SourceType::Upload).await
}

async fn embed_upload(
    State(state): State<LegacyCompatState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> impl IntoResponse {
    legacy_multipart_ingest(&state, &headers, multipart, SourceType::Upload).await
}

async fn local_embed(
    State(state): State<LegacyCompatState>,
    headers: HeaderMap,
    ExtractJson(body): ExtractJson<LegacyLocalEmbedBody>,
) -> impl IntoResponse {
    if body
        .content
        .as_ref()
        .filter(|v| !v.trim().is_empty())
        .is_none()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"detail":"content cannot be empty"})),
        )
            .into_response();
    }

    let scope = legacy_scope(&headers);
    let actor = body
        .entity_id
        .clone()
        .or_else(|| header_string(&headers, "x-actor-id"));
    let ctx = legacy_ctx(&headers, &scope, actor.clone());

    let request = IngestRequest {
        scope,
        asset_id: AssetId(body.file_id.clone()),
        source_type: SourceType::LocalFile,
        source_uri: body.path.clone(),
        content: body.content.clone(),
        mime_type: body.known_type.clone(),
    };

    match state.ingest_service.ingest(ctx, request).await {
        Ok(response) => Json(serde_json::json!({
            "status": "success",
            "message": "legacy embed completed",
            "file_id": response.asset_id.0,
            "filename": body.path.clone().unwrap_or_else(|| response.asset_id.0.clone()),
            "known_type": body.known_type.unwrap_or_else(|| "unknown".to_string()),
        }))
        .into_response(),
        Err(err) => core_error_to_response(err),
    }
}

async fn text(
    State(state): State<LegacyCompatState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> impl IntoResponse {
    let mut multipart = multipart;
    let payload = match parse_legacy_upload(&mut multipart).await {
        Ok(payload) => payload,
        Err(resp) => return resp.into_response(),
    };

    if payload.file_id.is_none() || payload.content.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"detail":"file_id and file are required"})),
        )
            .into_response();
    }

    let scope = legacy_scope(&headers);
    let actor = payload
        .entity_id
        .clone()
        .or_else(|| header_string(&headers, "x-actor-id"));
    let ctx = legacy_ctx(&headers, &scope, actor.clone());

    let request = ExtractRequest {
        scope,
        source_type: SourceType::Text,
        source_uri: None,
        content: payload.content.clone(),
    };

    match state.extract_service.extract(ctx, request).await {
        Ok(response) => Json(serde_json::json!({
            "text": response.text,
            "file_id": payload.file_id.clone().unwrap(),
            "filename": payload
                .filename
                .clone()
                .unwrap_or_else(|| payload.file_id.clone().unwrap()),
            "known_type": payload.known_type.unwrap_or_else(|| "unknown".to_string()),
        }))
        .into_response(),
        Err(err) => core_error_to_response(err),
    }
}

async fn legacy_multipart_ingest(
    state: &LegacyCompatState,
    headers: &HeaderMap,
    mut multipart: Multipart,
    source_type: SourceType,
) -> Response {
    let payload = match parse_legacy_upload(&mut multipart).await {
        Ok(payload) => payload,
        Err(resp) => return resp,
    };

    let file_id = match payload.file_id.clone() {
        Some(id) => id,
        None => return bad_request("file_id is required"),
    };

    let content = match payload.content.clone() {
        Some(text) if !text.trim().is_empty() => text,
        _ => return bad_request("file contents are required"),
    };

    let scope = legacy_scope(headers);
    let actor = payload
        .entity_id
        .clone()
        .or_else(|| header_string(headers, "x-actor-id"));
    let ctx = legacy_ctx(headers, &scope, actor.clone());

    let request = IngestRequest {
        scope,
        asset_id: AssetId(file_id.clone()),
        source_type,
        source_uri: None,
        content: Some(content),
        mime_type: payload.known_type.clone(),
    };

    match state.ingest_service.ingest(ctx, request).await {
        Ok(response) => Json(serde_json::json!({
            "status": "success",
            "message": "legacy embed completed",
            "file_id": response.asset_id.0,
            "filename": payload.filename.unwrap_or_else(|| file_id.clone()),
            "known_type": payload.known_type.unwrap_or_else(|| "unknown".to_string()),
        }))
        .into_response(),
        Err(err) => core_error_to_response(err),
    }
}

async fn parse_legacy_upload(multipart: &mut Multipart) -> Result<LegacyUploadPayload, Response> {
    let mut payload = LegacyUploadPayload {
        file_id: None,
        entity_id: None,
        filename: None,
        known_type: None,
        content: None,
    };

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| bad_request("malformed multipart payload"))?
    {
        match field.name() {
            Some("file_id") => {
                let text = field
                    .text()
                    .await
                    .map_err(|_| bad_request("invalid file_id field"))?;
                payload.file_id = Some(text);
            }
            Some("entity_id") => {
                let text = field
                    .text()
                    .await
                    .map_err(|_| bad_request("invalid entity_id field"))?;
                payload.entity_id = Some(text);
            }
            Some("known_type") => {
                let text = field
                    .text()
                    .await
                    .map_err(|_| bad_request("invalid known_type field"))?;
                payload.known_type = Some(text);
            }
            Some("file") => {
                if payload.content.is_some() {
                    continue;
                }
                if let Some(filename) = field.file_name() {
                    payload.filename = Some(filename.to_string());
                }
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|_| bad_request("invalid file data"))?;
                let text = String::from_utf8(bytes.to_vec())
                    .map_err(|_| bad_request("file must be text"))?;
                payload.content = Some(text);
            }
            _ => {}
        }
    }

    Ok(payload)
}

fn legacy_scope(headers: &HeaderMap) -> Scope {
    Scope {
        tenant_id: TenantId(
            header_string(headers, "x-tenant-id")
                .or_else(|| std::env::var("LEGACY_DEFAULT_TENANT").ok())
                .unwrap_or_else(|| "public".to_string()),
        ),
        namespace: Namespace(
            header_string(headers, "x-namespace")
                .or_else(|| std::env::var("LEGACY_DEFAULT_NAMESPACE").ok())
                .unwrap_or_else(|| "librechat".to_string()),
        ),
    }
}

fn legacy_ctx(headers: &HeaderMap, scope: &Scope, actor_id: Option<String>) -> RequestContext {
    RequestContext {
        tenant_id: scope.tenant_id.clone(),
        actor_id: actor_id.map(ActorId),
        roles: vec![],
        allowed_namespaces: vec![scope.namespace.clone()],
        request_id: header_string(headers, "x-request-id")
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
    }
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "detail": message
        })),
    )
        .into_response()
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

    struct DummyIngest {
        seen_request: Arc<Mutex<Option<IngestRequest>>>,
    }
    struct DummyExtract {
        seen_request: Arc<Mutex<Option<ExtractRequest>>>,
    }
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
            ingest_service: Arc::new(DummyIngest {
                seen_request: Arc::new(Mutex::new(None)),
            }),
            extract_service: Arc::new(DummyExtract {
                seen_request: Arc::new(Mutex::new(None)),
            }),
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
            ingest_service: Arc::new(DummyIngest {
                seen_request: Arc::new(Mutex::new(None)),
            }),
            extract_service: Arc::new(DummyExtract {
                seen_request: Arc::new(Mutex::new(None)),
            }),
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

    #[tokio::test]
    async fn post_embed_calls_ingest_and_returns_metadata() {
        let seen_ingest = Arc::new(Mutex::new(None));
        let state = LegacyCompatState {
            ingest_service: Arc::new(DummyIngest {
                seen_request: seen_ingest.clone(),
            }),
            extract_service: Arc::new(DummyExtract {
                seen_request: Arc::new(Mutex::new(None)),
            }),
            query_service: Arc::new(DummyQuery {
                seen_request: Arc::new(Mutex::new(None)),
                seen_batch_request: Arc::new(Mutex::new(None)),
            }),
        };
        let app = router(state);

        let boundary = "BOUND";
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"file_id\"\r\n\r\nlegacy-1\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"name.txt\"\r\n\
             Content-Type: text/plain\r\n\r\ndata\r\n--{b}--\r\n",
            b = boundary
        );

        let req = Request::builder()
            .method("POST")
            .uri("/embed")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(value["status"], "success");
        assert_eq!(value["file_id"], "legacy-1");
        assert!(seen_ingest.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn post_local_embed_calls_ingest_with_source_type() {
        let seen_ingest = Arc::new(Mutex::new(None));
        let state = LegacyCompatState {
            ingest_service: Arc::new(DummyIngest {
                seen_request: seen_ingest.clone(),
            }),
            extract_service: Arc::new(DummyExtract {
                seen_request: Arc::new(Mutex::new(None)),
            }),
            query_service: Arc::new(DummyQuery {
                seen_request: Arc::new(Mutex::new(None)),
                seen_batch_request: Arc::new(Mutex::new(None)),
            }),
        };
        let app = router(state);

        let body = r#"{
            "file_id":"local-1",
            "content":"payload",
            "known_type":"text/plain",
            "path":"/tmp/local.txt"
        }"#;

        let req = Request::builder()
            .method("POST")
            .uri("/local/embed")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(value["filename"], "/tmp/local.txt");
        assert_eq!(value["known_type"], "text/plain");
        assert!(seen_ingest.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn post_text_returns_extracted_text_and_file_id() {
        let seen_extract = Arc::new(Mutex::new(None));
        let state = LegacyCompatState {
            ingest_service: Arc::new(DummyIngest {
                seen_request: Arc::new(Mutex::new(None)),
            }),
            extract_service: Arc::new(DummyExtract {
                seen_request: seen_extract.clone(),
            }),
            query_service: Arc::new(DummyQuery {
                seen_request: Arc::new(Mutex::new(None)),
                seen_batch_request: Arc::new(Mutex::new(None)),
            }),
        };
        let app = router(state);

        let boundary = "BND";
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"file_id\"\r\n\r\ntext-1\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"txt.txt\"\r\n\
             Content-Type: text/plain\r\n\r\nhello\r\n--{b}--\r\n",
            b = boundary
        );

        let req = Request::builder()
            .method("POST")
            .uri("/text")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(value["text"], "hello");
        assert_eq!(value["file_id"], "text-1");
        assert!(seen_extract.lock().unwrap().is_some());
    }
}
