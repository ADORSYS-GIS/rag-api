use anyhow::Result;
use rag_app_runtime::build_container;
use rag_mcp_api::{McpApiState, router};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,rag_mcp_rs=debug".to_string()),
        )
        .json()
        .init();

    let container = build_container()?;
    let state = McpApiState {
        ingest_service: container.ingest_service,
        extract_service: container.extract_service,
        query_service: container.query_service,
    };

    let app = router(state);
    let bind_addr =
        std::env::var("RAG_MCP_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8081".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!(%bind_addr, "starting rag-mcp.rs");
    axum::serve(listener, app).await?;

    Ok(())
}
