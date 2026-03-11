use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rag_core::{ExtractService, IngestService, QueryService};

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
        .route("/query", post(not_implemented))
        .route("/query_multiple", post(not_implemented))
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
