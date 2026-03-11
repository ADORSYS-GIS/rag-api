use std::sync::Arc;

use axum::{
    extract::State,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use rag_core::{ExtractService, IngestService, QueryService};
use serde::Serialize;

#[derive(Clone)]
pub struct McpApiState {
    pub ingest_service: Arc<dyn IngestService>,
    pub extract_service: Arc<dyn ExtractService>,
    pub query_service: Arc<dyn QueryService>,
}

#[derive(Debug, Serialize)]
struct McpInfo {
    transport: &'static str,
    endpoint: &'static str,
    status: &'static str,
}

pub fn router(state: McpApiState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/mcp", get(mcp_get).post(mcp_post))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    Json(serde_json::json!({"status":"ok","service":"rag-mcp.rs"}))
}

async fn mcp_get(State(_state): State<McpApiState>) -> impl IntoResponse {
    Json(McpInfo {
        transport: "streamable-http",
        endpoint: "/mcp",
        status: "scaffolded",
    })
}

async fn mcp_post(State(_state): State<McpApiState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "error": "MCP methods not implemented yet",
        "endpoint": "/mcp"
    }))
}
