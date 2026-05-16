//! RAG MCP server — exposes ingestion, extraction, query, delete, and list
//! capabilities as MCP tools over Streamable HTTP transport.
//!
//! The handler delegates every operation to the shared `rag-core` service
//! traits so no business logic lives here.

use std::sync::Arc;

use axum::{http::StatusCode, response::IntoResponse, routing::get, Router};
use rag_core::{
    ActorId, AssetFilter, AssetId, BatchQueryRequest, ChunkRepository, CoreError, ExtractRequest,
    ExtractService, IngestRequest, IngestService, Namespace, QueryRequest, QueryService,
    RequestContext as RagCtx, Scope, SourceType, TenantId,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::{
        streamable_http_server::{
            session::local::LocalSessionManager, tower::StreamableHttpService,
        },
        StreamableHttpServerConfig,
    },
    ErrorData, Json, RoleServer, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─── response wrapper ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RagResponse {
    #[schemars(schema_with = "any_value_schema")]
    pub result: Value,
}

fn any_value_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["object", "array", "string", "number", "boolean", "null"]
    })
}

// ─── tool input types ────────────────────────────────────────────────────────

/// Ingest text content into the RAG vector store.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IngestAssetParams {
    /// Unique identifier for this asset (equivalent to `file_id` in legacy API).
    pub asset_id: String,
    /// Raw text content to chunk, embed, and index. Mutually exclusive with
    /// `source_uri`; if both are provided, `content` takes precedence.
    #[serde(default)]
    pub content: Option<String>,
    /// Source type: `text`, `upload`, `local_file`, `website`, `pdf`, `image`,
    /// `code`, or `ide_buffer`. Defaults to `text`.
    #[serde(default = "default_source_type")]
    pub source_type: String,
    /// Local file path or HTTP/S URL to read and extract text from. Required
    /// when `content` is not provided. Supports plain text, HTML, and PDF.
    #[serde(default)]
    pub source_uri: Option<String>,
    /// Optional MIME type hint (e.g. `text/plain`, `application/pdf`).
    #[serde(default)]
    pub mime_type: Option<String>,
    /// Tenant scope. Defaults to `public`.
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    /// Namespace scope. Defaults to `default`.
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Optional actor / user identifier.
    #[serde(default)]
    pub actor_id: Option<String>,
}

fn default_source_type() -> String {
    "text".to_string()
}
fn default_tenant() -> String {
    "public".to_string()
}
fn default_namespace() -> String {
    "default".to_string()
}

/// Extract text from content without storing vectors.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExtractAssetTextParams {
    /// Raw text content to extract from. Mutually exclusive with `source_uri`;
    /// if both are provided, `content` takes precedence.
    #[serde(default)]
    pub content: Option<String>,
    /// Source type hint. Defaults to `text`.
    #[serde(default = "default_source_type")]
    pub source_type: String,
    /// Local file path or HTTP/S URL to read and extract text from. Required
    /// when `content` is not provided.
    #[serde(default)]
    pub source_uri: Option<String>,
    /// Tenant scope. Defaults to `public`.
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    /// Namespace scope. Defaults to `default`.
    #[serde(default = "default_namespace")]
    pub namespace: String,
}

/// Semantic search over a single asset.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryAssetParams {
    /// The search query text.
    pub query: String,
    /// Asset ID to restrict the search to.
    pub asset_id: String,
    /// Number of results to return (1–20). Defaults to 4.
    #[serde(default = "default_k")]
    pub k: usize,
    /// Tenant scope. Defaults to `public`.
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    /// Namespace scope. Defaults to `default`.
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Optional actor / user identifier.
    #[serde(default)]
    pub actor_id: Option<String>,
}

fn default_k() -> usize {
    4
}

/// Semantic search across multiple assets.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QueryAssetsParams {
    /// The search query text.
    pub query: String,
    /// Asset IDs to search across. Must not be empty.
    pub asset_ids: Vec<String>,
    /// Number of results to return (1–20). Defaults to 4.
    #[serde(default = "default_k")]
    pub k: usize,
    /// Tenant scope. Defaults to `public`.
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    /// Namespace scope. Defaults to `default`.
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Optional actor / user identifier.
    #[serde(default)]
    pub actor_id: Option<String>,
}

/// Delete all chunks for one or more assets.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteAssetsParams {
    /// Asset IDs to delete. Must not be empty.
    pub asset_ids: Vec<String>,
    /// Tenant scope. Defaults to `public`.
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    /// Namespace scope. Defaults to `default`.
    #[serde(default = "default_namespace")]
    pub namespace: String,
}

/// List all indexed assets in a scope.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListAssetsParams {
    /// Tenant scope. Defaults to `public`.
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    /// Namespace scope. Defaults to `default`.
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// Optional source type filter.
    #[serde(default)]
    pub source_type: Option<String>,
}

// ─── handler ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct RagMcpHandler {
    tool_router: ToolRouter<Self>,
    ingest: Arc<dyn IngestService>,
    extract: Arc<dyn ExtractService>,
    query: Arc<dyn QueryService>,
    repository: Arc<dyn ChunkRepository>,
}

impl std::fmt::Debug for RagMcpHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RagMcpHandler")
            .field("tools", &self.tool_router.list_all().len())
            .finish()
    }
}

impl RagMcpHandler {
    pub fn new(
        ingest: Arc<dyn IngestService>,
        extract: Arc<dyn ExtractService>,
        query: Arc<dyn QueryService>,
        repository: Arc<dyn ChunkRepository>,
    ) -> Self {
        Self {
            tool_router: Self::tool_router(),
            ingest,
            extract,
            query,
            repository,
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RagMcpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "RAG MCP server — ingest, extract, query, delete, and list assets \
                 backed by a Qdrant vector database.",
        )
    }
}

// ─── tool implementations ─────────────────────────────────────────────────────

#[tool_router(router = tool_router)]
impl RagMcpHandler {
    /// Ingest text content into the RAG vector store.
    ///
    /// The content is chunked, embedded via an OpenAI-compatible provider, and
    /// stored in Qdrant. Returns the asset_id and the number of chunks written.
    #[tool(
        name = "ingest_asset",
        description = "Chunk, embed, and index text content into the RAG vector store. \
                       Returns the asset_id and the number of chunks written."
    )]
    async fn ingest_asset_tool(
        &self,
        _context: RequestContext<RoleServer>,
        Parameters(params): Parameters<IngestAssetParams>,
    ) -> std::result::Result<Json<RagResponse>, ErrorData> {
        let source_type = parse_source_type(&params.source_type)?;
        let source_uri = params.source_uri;

        // Resolve text: use pre-extracted content, or pull it from source_uri.
        let content = match params.content {
            Some(c) => c,
            None => {
                let uri = source_uri.as_deref().ok_or_else(|| {
                    ErrorData::invalid_params(
                        "either content or source_uri must be provided",
                        None,
                    )
                })?;
                let ectx = make_rag_ctx(&params.tenant_id, &params.namespace, params.actor_id.as_deref());
                let extract_req = ExtractRequest {
                    scope: make_scope(&params.tenant_id, &params.namespace),
                    source_type: source_type.clone(),
                    source_uri: Some(uri.to_string()),
                    content: None,
                };
                self.extract
                    .extract(ectx, extract_req)
                    .await
                    .map_err(core_to_mcp_error)?
                    .text
            }
        };

        let scope = make_scope(&params.tenant_id, &params.namespace);
        let ctx = make_rag_ctx(&params.tenant_id, &params.namespace, params.actor_id.as_deref());

        let request = IngestRequest {
            scope,
            asset_id: AssetId(params.asset_id),
            source_type,
            source_uri,
            content: Some(content),
            mime_type: params.mime_type,
        };

        let response = self
            .ingest
            .ingest(ctx, request)
            .await
            .map_err(core_to_mcp_error)?;

        to_json(serde_json::json!({
            "asset_id": response.asset_id.0,
            "chunks_written": response.chunks_written,
        }))
    }

    /// Extract text from content without storing vectors.
    ///
    /// Useful for previewing what text would be indexed before committing to
    /// ingestion.
    #[tool(
        name = "extract_asset_text",
        description = "Extract plain text from content without writing to the vector store. \
                       Returns the extracted text string."
    )]
    async fn extract_asset_text_tool(
        &self,
        _context: RequestContext<RoleServer>,
        Parameters(params): Parameters<ExtractAssetTextParams>,
    ) -> std::result::Result<Json<RagResponse>, ErrorData> {
        let source_type = parse_source_type(&params.source_type)?;
        let scope = make_scope(&params.tenant_id, &params.namespace);
        let ctx = make_rag_ctx(&params.tenant_id, &params.namespace, None);

        if params.content.is_none() && params.source_uri.is_none() {
            return Err(ErrorData::invalid_params(
                "either content or source_uri must be provided",
                None,
            ));
        }

        let request = ExtractRequest {
            scope,
            source_type,
            source_uri: params.source_uri,
            content: params.content,
        };

        let response = self
            .extract
            .extract(ctx, request)
            .await
            .map_err(core_to_mcp_error)?;

        to_json(serde_json::json!({ "text": response.text }))
    }

    /// Semantic search over a single asset.
    ///
    /// Embeds the query and returns the top-k most similar chunks from the
    /// specified asset.
    #[tool(
        name = "query_asset",
        description = "Perform semantic similarity search over a single indexed asset. \
                       Returns scored text chunks ordered by relevance."
    )]
    async fn query_asset_tool(
        &self,
        _context: RequestContext<RoleServer>,
        Parameters(params): Parameters<QueryAssetParams>,
    ) -> std::result::Result<Json<RagResponse>, ErrorData> {
        if params.query.trim().is_empty() {
            return Err(ErrorData::invalid_params("query cannot be empty", None));
        }

        let scope = make_scope(&params.tenant_id, &params.namespace);
        let ctx = make_rag_ctx(
            &params.tenant_id,
            &params.namespace,
            params.actor_id.as_deref(),
        );

        let request = QueryRequest {
            scope,
            query: params.query,
            asset_id: Some(AssetId(params.asset_id)),
            k: params.k,
        };

        let response = self
            .query
            .query(ctx, request)
            .await
            .map_err(core_to_mcp_error)?;

        to_json(serde_json::json!({ "matches": response.matches }))
    }

    /// Semantic search across multiple assets.
    ///
    /// Embeds the query and returns the top-k most similar chunks from all
    /// specified assets combined.
    #[tool(
        name = "query_assets",
        description = "Perform semantic similarity search across multiple indexed assets. \
                       Returns scored text chunks ordered by relevance."
    )]
    async fn query_assets_tool(
        &self,
        _context: RequestContext<RoleServer>,
        Parameters(params): Parameters<QueryAssetsParams>,
    ) -> std::result::Result<Json<RagResponse>, ErrorData> {
        if params.query.trim().is_empty() {
            return Err(ErrorData::invalid_params("query cannot be empty", None));
        }
        if params.asset_ids.is_empty() {
            return Err(ErrorData::invalid_params("asset_ids cannot be empty", None));
        }

        let scope = make_scope(&params.tenant_id, &params.namespace);
        let ctx = make_rag_ctx(
            &params.tenant_id,
            &params.namespace,
            params.actor_id.as_deref(),
        );

        let request = BatchQueryRequest {
            scope,
            query: params.query,
            asset_ids: params.asset_ids.into_iter().map(AssetId).collect(),
            k: params.k,
        };

        let response = self
            .query
            .query_batch(ctx, request)
            .await
            .map_err(core_to_mcp_error)?;

        to_json(serde_json::json!({ "matches": response.matches }))
    }

    /// Delete all chunks for one or more assets.
    ///
    /// Removes every indexed chunk belonging to the given asset IDs from the
    /// vector store. Returns the total number of points deleted per asset.
    #[tool(
        name = "delete_assets",
        description = "Delete all indexed chunks for one or more assets from the vector store. \
                       Returns per-asset deletion counts."
    )]
    async fn delete_assets_tool(
        &self,
        _context: RequestContext<RoleServer>,
        Parameters(params): Parameters<DeleteAssetsParams>,
    ) -> std::result::Result<Json<RagResponse>, ErrorData> {
        if params.asset_ids.is_empty() {
            return Err(ErrorData::invalid_params("asset_ids cannot be empty", None));
        }

        let scope = make_scope(&params.tenant_id, &params.namespace);
        let mut results = Vec::with_capacity(params.asset_ids.len());

        for id in &params.asset_ids {
            let asset_id = AssetId(id.clone());
            let summary = self
                .repository
                .delete_asset(&scope, &asset_id)
                .await
                .map_err(core_to_mcp_error)?;
            results.push(serde_json::json!({
                "asset_id": id,
                "points_deleted": summary.points_deleted,
            }));
        }

        to_json(serde_json::json!({ "deleted": results }))
    }

    /// List all indexed assets in a scope.
    ///
    /// Returns asset IDs and their chunk counts for the given tenant and
    /// namespace. Optionally filtered by source type.
    #[tool(
        name = "list_assets",
        description = "List all assets indexed in the vector store for a given scope. \
                       Returns asset IDs and chunk counts."
    )]
    async fn list_assets_tool(
        &self,
        _context: RequestContext<RoleServer>,
        Parameters(params): Parameters<ListAssetsParams>,
    ) -> std::result::Result<Json<RagResponse>, ErrorData> {
        let scope = make_scope(&params.tenant_id, &params.namespace);

        let source_type_filter = params
            .source_type
            .as_deref()
            .map(parse_source_type)
            .transpose()?;

        let filter = AssetFilter {
            source_type: source_type_filter,
        };

        let assets = self
            .repository
            .list_assets(&scope, filter)
            .await
            .map_err(core_to_mcp_error)?;

        let items: Vec<Value> = assets
            .into_iter()
            .map(|a| {
                serde_json::json!({
                    "asset_id": a.asset_id.0,
                    "chunk_count": a.chunk_count,
                })
            })
            .collect();

        to_json(serde_json::json!({ "assets": items }))
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn parse_source_type(raw: &str) -> std::result::Result<SourceType, ErrorData> {
    SourceType::parse(raw).ok_or_else(|| {
        ErrorData::invalid_params(
            format!(
                "unknown source_type '{raw}'; valid values: \
                 text, upload, local_file, website, pdf, image, code, ide_buffer"
            ),
            None,
        )
    })
}

fn make_scope(tenant_id: &str, namespace: &str) -> Scope {
    Scope {
        tenant_id: TenantId(tenant_id.to_string()),
        namespace: Namespace(namespace.to_string()),
    }
}

fn make_rag_ctx(tenant_id: &str, namespace: &str, actor_id: Option<&str>) -> RagCtx {
    RagCtx {
        tenant_id: TenantId(tenant_id.to_string()),
        actor_id: actor_id.map(|a| ActorId(a.to_string())),
        roles: vec![],
        allowed_namespaces: vec![Namespace(namespace.to_string())],
        request_id: uuid::Uuid::now_v7().to_string(),
    }
}

fn core_to_mcp_error(err: CoreError) -> ErrorData {
    match err {
        CoreError::Validation(msg) => ErrorData::invalid_params(msg, None),
        CoreError::Unauthorized => ErrorData::invalid_params("unauthorized", None),
        CoreError::Forbidden => ErrorData::invalid_params("forbidden", None),
        CoreError::NotFound => ErrorData::resource_not_found("not found", None),
        CoreError::NotImplemented(msg) => ErrorData::internal_error(msg, None),
        CoreError::Provider(msg) | CoreError::Storage(msg) | CoreError::Lock(msg) => {
            ErrorData::internal_error(msg, None)
        }
    }
}

fn to_json(value: Value) -> std::result::Result<Json<RagResponse>, ErrorData> {
    Ok(Json(RagResponse { result: value }))
}

// ─── public server builder ────────────────────────────────────────────────────

/// Build the Axum router that serves the MCP endpoint plus health probes.
///
/// Mount this on a `TcpListener` in the binary.
pub fn build_router(
    ingest: Arc<dyn IngestService>,
    extract: Arc<dyn ExtractService>,
    query: Arc<dyn QueryService>,
    repository: Arc<dyn ChunkRepository>,
) -> Router {
    let handler = RagMcpHandler::new(ingest, extract, query, repository);

    let mcp_service: StreamableHttpService<RagMcpHandler, LocalSessionManager> =
        StreamableHttpService::new(
            {
                let handler = handler.clone();
                move || Ok(handler.clone())
            },
            Default::default(),
            StreamableHttpServerConfig::default(),
        );

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .nest_service("/mcp", mcp_service)
}

async fn healthz() -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "status": "ok",
            "service": "rag-mcp"
        })),
    )
}

async fn readyz() -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "status": "ready",
            "service": "rag-mcp"
        })),
    )
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use rag_core::{
        AssetFilter, AssetId, AssetSummary, BatchQueryRequest, BatchQueryResponse, ChunkRecord,
        CoreError, DeleteSummary, ExtractRequest, ExtractResponse, ExtractService, IngestRequest,
        IngestResponse, IngestService, QueryRequest, QueryResponse, QueryService,
        RequestContext as RagCtx, Scope, ScoredChunk, SearchRequest, SourceType, UpsertSummary,
    };

    use super::RagMcpHandler;

    struct NoopIngest;
    struct NoopExtract;
    struct NoopQuery;
    struct NoopRepo;

    #[async_trait]
    impl IngestService for NoopIngest {
        async fn ingest(
            &self,
            _ctx: RagCtx,
            request: IngestRequest,
        ) -> Result<IngestResponse, CoreError> {
            Ok(IngestResponse {
                asset_id: request.asset_id,
                chunks_written: 0,
            })
        }
    }

    #[async_trait]
    impl ExtractService for NoopExtract {
        async fn extract(
            &self,
            _ctx: RagCtx,
            request: ExtractRequest,
        ) -> Result<ExtractResponse, CoreError> {
            Ok(ExtractResponse {
                text: request.content.unwrap_or_default(),
            })
        }
    }

    #[async_trait]
    impl QueryService for NoopQuery {
        async fn query(
            &self,
            _ctx: RagCtx,
            request: QueryRequest,
        ) -> Result<QueryResponse, CoreError> {
            let chunk = ChunkRecord {
                tenant_id: request.scope.tenant_id,
                namespace: request.scope.namespace,
                asset_id: request.asset_id.unwrap_or_else(|| AssetId("a".into())),
                actor_id: None,
                source_type: SourceType::Text,
                source_uri: None,
                digest: "d".into(),
                chunk_index: 0,
                page: None,
                path: None,
                language: None,
                mime_type: None,
                title: None,
                text: "hello".into(),
                embedding: vec![],
                tags: vec![],
                created_at: chrono::Utc::now(),
            };
            Ok(QueryResponse {
                matches: vec![ScoredChunk { chunk, score: 0.9 }],
            })
        }

        async fn query_batch(
            &self,
            _ctx: RagCtx,
            request: BatchQueryRequest,
        ) -> Result<BatchQueryResponse, CoreError> {
            let chunk = ChunkRecord {
                tenant_id: request.scope.tenant_id,
                namespace: request.scope.namespace,
                asset_id: request.asset_ids[0].clone(),
                actor_id: None,
                source_type: SourceType::Text,
                source_uri: None,
                digest: "d-batch".into(),
                chunk_index: 0,
                page: None,
                path: None,
                language: None,
                mime_type: None,
                title: None,
                text: "hello-batch".into(),
                embedding: vec![],
                tags: vec![],
                created_at: chrono::Utc::now(),
            };
            Ok(BatchQueryResponse {
                matches: vec![ScoredChunk { chunk, score: 0.8 }],
            })
        }
    }

    #[async_trait]
    impl rag_core::ChunkRepository for NoopRepo {
        async fn upsert_chunks(
            &self,
            _scope: &Scope,
            _chunks: Vec<ChunkRecord>,
        ) -> Result<UpsertSummary, CoreError> {
            Ok(UpsertSummary { points_written: 0 })
        }

        async fn delete_asset(
            &self,
            _scope: &Scope,
            _asset_id: &AssetId,
        ) -> Result<DeleteSummary, CoreError> {
            Ok(DeleteSummary { points_deleted: 3 })
        }

        async fn get_asset_chunks(
            &self,
            _scope: &Scope,
            _asset_id: &AssetId,
        ) -> Result<Vec<ChunkRecord>, CoreError> {
            Ok(vec![])
        }

        async fn list_assets(
            &self,
            _scope: &Scope,
            _filter: AssetFilter,
        ) -> Result<Vec<AssetSummary>, CoreError> {
            Ok(vec![AssetSummary {
                asset_id: AssetId("asset-1".into()),
                chunk_count: 5,
            }])
        }

        async fn search(
            &self,
            _scope: &Scope,
            _request: SearchRequest,
        ) -> Result<Vec<ScoredChunk>, CoreError> {
            Ok(vec![])
        }
    }

    fn make_handler() -> RagMcpHandler {
        RagMcpHandler::new(
            Arc::new(NoopIngest),
            Arc::new(NoopExtract),
            Arc::new(NoopQuery),
            Arc::new(NoopRepo),
        )
    }

    #[test]
    fn handler_registers_expected_tools() {
        let handler = make_handler();
        let mut names: Vec<String> = handler
            .tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        names.sort();

        let mut expected = vec![
            "ingest_asset",
            "extract_asset_text",
            "query_asset",
            "query_assets",
            "delete_assets",
            "list_assets",
        ];
        expected.sort();

        assert_eq!(names, expected);
    }

    #[test]
    fn tool_output_schemas_are_objects_not_booleans() {
        let handler = make_handler();
        for tool in handler.tool_router.list_all() {
            if let Some(output) = &tool.output_schema {
                let result_schema = output
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .and_then(|p| p.get("result"));
                if let Some(schema) = result_schema {
                    assert!(
                        schema.is_object(),
                        "tool '{}' result schema must be an object, not a boolean",
                        tool.name
                    );
                }
            }
        }
    }
}
