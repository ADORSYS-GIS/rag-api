use std::sync::Arc;

use anyhow::Result;
use rag_app_runtime::build_container;
use rag_gateway::RemappingExtractService;
use rag_mcp_api::{build_router, RagMcpHandler};
use rmcp::ServiceExt as _;

#[tokio::main]
async fn main() -> Result<()> {
    // Always log to stderr — stdout is used by the stdio MCP transport.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,mcp_gateway_rs=debug".to_string()),
        )
        .json()
        .init();

    let host_root = std::env::var("GATEWAY_HOST_ROOT").unwrap_or_default();
    let transport = std::env::var("GATEWAY_TRANSPORT").unwrap_or_else(|_| "http".to_string());

    let container = build_container()?;

    let extract = if host_root.is_empty() {
        container.extract_service
    } else {
        tracing::info!(%host_root, "path remapping enabled");
        Arc::new(RemappingExtractService::new(
            container.extract_service,
            &host_root,
        ))
    };

    match transport.as_str() {
        "stdio" => {
            tracing::info!("starting mcp-gateway (stdio transport)");
            let handler = RagMcpHandler::new(
                container.ingest_service,
                extract,
                container.query_service,
                container.repository,
            );
            let running = handler.serve(rmcp::transport::io::stdio()).await?;
            running.waiting().await?;
        }
        _ => {
            let bind_addr =
                std::env::var("GATEWAY_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8090".to_string());
            tracing::info!(%bind_addr, "starting mcp-gateway (http transport)");

            let app = build_router(
                container.ingest_service,
                extract,
                container.query_service,
                container.repository,
            );

            let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
            axum::serve(listener, app).await?;
        }
    }

    Ok(())
}
