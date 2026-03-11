use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rag_core::{
    ExtractService, IngestService, QueryService,
};
use serde::Serialize;

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
        .route("/v1/query", post(not_implemented))
        .route("/v1/query:batch", post(not_implemented))
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
