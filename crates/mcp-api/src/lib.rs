use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Json as ExtractJson, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use rag_core::{ExtractService, IngestService, QueryService};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone)]
pub struct McpApiState {
    pub ingest_service: Arc<dyn IngestService>,
    pub extract_service: Arc<dyn ExtractService>,
    pub query_service: Arc<dyn QueryService>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
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
    Json(json!({
        "jsonrpc": "2.0",
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": "local-rag-mcp",
                "version": "0.1.0"
            },
            "instructions": "Local development MCP endpoint for rag-api. Tool methods are scaffolded."
        }
    }))
}

async fn mcp_post(
    State(_state): State<McpApiState>,
    ExtractJson(payload): ExtractJson<JsonRpcRequest>,
) -> impl IntoResponse {
    if payload.jsonrpc.as_deref() != Some("2.0") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "jsonrpc": "2.0",
                "id": payload.id,
                "error": {
                    "code": -32600,
                    "message": "Invalid Request"
                }
            })),
        )
            .into_response();
    }

    match payload.method.as_str() {
        "initialize" => Json(json!({
            "jsonrpc": "2.0",
            "id": payload.id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": {
                    "name": "local-rag-mcp",
                    "version": "0.1.0"
                },
                "instructions": "Local development MCP endpoint for rag-api. Tool methods are scaffolded."
            }
        }))
        .into_response(),
        "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
        "ping" => Json(json!({
            "jsonrpc": "2.0",
            "id": payload.id,
            "result": {}
        }))
        .into_response(),
        "tools/list" => Json(json!({
            "jsonrpc": "2.0",
            "id": payload.id,
            "result": {
                "tools": []
            }
        }))
        .into_response(),
        "resources/list" => Json(json!({
            "jsonrpc": "2.0",
            "id": payload.id,
            "result": {
                "resources": []
            }
        }))
        .into_response(),
        "prompts/list" => Json(json!({
            "jsonrpc": "2.0",
            "id": payload.id,
            "result": {
                "prompts": []
            }
        }))
        .into_response(),
        _ => Json(json!({
            "jsonrpc": "2.0",
            "id": payload.id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {}", payload.method)
            }
        }))
        .into_response(),
    }
}
