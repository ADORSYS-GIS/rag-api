use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use qdrant_client::qdrant::{
    Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, DeletePointsBuilder,
    Distance, FieldType, PointStruct, ScoredPoint, ScrollPointsBuilder, SearchPointsBuilder,
    VectorParamsBuilder, vectors_config,
};
use qdrant_client::{Payload, Qdrant};
use rag_core::{
    ActorId, AssetFilter, AssetId, AssetSummary, ChunkRecord, ChunkRepository, CoreError,
    DeleteSummary, Namespace, Scope, ScoredChunk, SearchRequest, SourceType, TenantId,
    UpsertSummary,
};
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;
use tracing::{info, error, debug};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct QdrantRepositoryConfig {
    pub url: String,
    pub api_key: Option<String>,
    pub collection_name: String,
    pub vector_size: u64,
}

impl Default for QdrantRepositoryConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:6334".to_string(),
            api_key: None,
            collection_name: "chunks_te3_small".to_string(),
            vector_size: 1536,
        }
    }
}

pub struct QdrantChunkRepository {
    client: Qdrant,
    config: QdrantRepositoryConfig,
    init_once: OnceCell<()>,
}

impl QdrantChunkRepository {
    pub fn new(config: QdrantRepositoryConfig) -> Result<Self, CoreError> {
        let mut builder = Qdrant::from_url(&config.url);
        if let Some(api_key) = config.api_key.clone().filter(|k| !k.is_empty()) {
            builder = builder.api_key(api_key);
        }

        let client = builder
            .build()
            .map_err(|e| CoreError::Storage(format!("qdrant client init failed: {e}")))?;

        Ok(Self {
            client,
            config,
            init_once: OnceCell::new(),
        })
    }

    pub async fn ensure_schema(&self) -> Result<(), CoreError> {
        self.init_once
            .get_or_try_init(|| async {
                self.ensure_collection().await?;
                self.ensure_indexes().await?;
                Ok::<(), CoreError>(())
            })
            .await
            .map(|_| ())
    }

    async fn ensure_collection(&self) -> Result<(), CoreError> {
        let exists = self
            .client
            .collection_exists(&self.config.collection_name)
            .await
            .map_err(|e| CoreError::Storage(format!("collection_exists failed: {e}")))?;

        if exists {
            self.validate_existing_collection_vector_size().await?;
            return Ok(());
        }

        self.client
            .create_collection(
                CreateCollectionBuilder::new(&self.config.collection_name).vectors_config(
                    VectorParamsBuilder::new(self.config.vector_size, Distance::Cosine),
                ),
            )
            .await
            .map_err(|e| CoreError::Storage(format!("create_collection failed: {e}")))?;

        Ok(())
    }

    async fn validate_existing_collection_vector_size(&self) -> Result<(), CoreError> {
        let info = self
            .client
            .collection_info(&self.config.collection_name)
            .await
            .map_err(|e| CoreError::Storage(format!("collection_info failed: {e}")))?;

        let Some(result) = info.result else {
            return Err(CoreError::Storage(
                "collection_info returned no result".to_string(),
            ));
        };

        let Some(config) = result.config else {
            return Err(CoreError::Storage(
                "collection has no config payload".to_string(),
            ));
        };

        let Some(params) = config.params else {
            return Err(CoreError::Storage(
                "collection has no params payload".to_string(),
            ));
        };

        let Some(vectors_config) = params.vectors_config else {
            return Err(CoreError::Storage(
                "collection has no vectors_config".to_string(),
            ));
        };

        let actual_size = match vectors_config.config {
            Some(vectors_config::Config::Params(vector)) => Some(vector.size),
            Some(vectors_config::Config::ParamsMap(vectors)) => {
                vectors.map.values().next().map(|vector| vector.size)
            }
            None => None,
        };

        let Some(actual_size) = actual_size else {
            return Err(CoreError::Storage(
                "unable to determine collection vector size".to_string(),
            ));
        };

        if actual_size != self.config.vector_size {
            return Err(CoreError::Validation(format!(
                "qdrant collection '{}' vector size mismatch: expected {}, found {}",
                self.config.collection_name, self.config.vector_size, actual_size
            )));
        }

        Ok(())
    }

    async fn ensure_indexes(&self) -> Result<(), CoreError> {
        for field in [
            "tenant_id",
            "namespace",
            "asset_id",
            "actor_id",
            "source_type",
        ] {
            let result = self
                .client
                .create_field_index(CreateFieldIndexCollectionBuilder::new(
                    &self.config.collection_name,
                    field,
                    FieldType::Keyword,
                ))
                .await;

            if let Err(err) = result {
                let text = err.to_string();
                if !text.to_ascii_lowercase().contains("already") {
                    return Err(CoreError::Storage(format!(
                        "create_field_index failed for '{field}': {text}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn point_id(chunk: &ChunkRecord) -> String {
        let seed = format!(
            "{}:{}:{}:{}:{}",
            chunk.tenant_id.0, chunk.namespace.0, chunk.asset_id.0, chunk.chunk_index, chunk.digest
        );
        Uuid::new_v5(&Uuid::NAMESPACE_OID, seed.as_bytes()).to_string()
    }

    fn chunk_to_payload(chunk: &ChunkRecord) -> Result<Payload, CoreError> {
        let payload = StoredPayload {
            tenant_id: chunk.tenant_id.0.clone(),
            namespace: chunk.namespace.0.clone(),
            asset_id: chunk.asset_id.0.clone(),
            actor_id: chunk.actor_id.as_ref().map(|a| a.0.clone()),
            source_type: chunk.source_type.as_str().to_string(),
            source_uri: chunk.source_uri.clone(),
            digest: chunk.digest.clone(),
            chunk_index: chunk.chunk_index,
            page: chunk.page,
            path: chunk.path.clone(),
            language: chunk.language.clone(),
            mime_type: chunk.mime_type.clone(),
            title: chunk.title.clone(),
            text: chunk.text.clone(),
            tags: chunk.tags.clone(),
            created_at: chunk.created_at.to_rfc3339(),
        };

        let value = serde_json::to_value(payload)
            .map_err(|e| CoreError::Storage(format!("payload serialization failed: {e}")))?;
        Payload::try_from(value)
            .map_err(|e| CoreError::Storage(format!("payload conversion failed: {e}")))
    }

    fn scored_point_to_chunk(point: ScoredPoint) -> Result<ScoredChunk, CoreError> {
        let payload: Payload = point.payload.into();
        let stored: StoredPayload = payload
            .deserialize()
            .map_err(|e| CoreError::Storage(format!("payload deserialize failed: {e}")))?;

        Ok(ScoredChunk {
            chunk: stored.into_chunk_record(),
            score: point.score,
        })
    }

    fn build_scope_filter(
        scope: &Scope,
        single_asset: Option<&AssetId>,
        asset_ids: &[AssetId],
        source_type: Option<&SourceType>,
    ) -> qdrant_client::qdrant::Filter {
        let mut must = vec![
            Condition::matches("tenant_id", scope.tenant_id.0.clone()),
            Condition::matches("namespace", scope.namespace.0.clone()),
        ];

        if let Some(asset_id) = single_asset {
            must.push(Condition::matches("asset_id", asset_id.0.clone()));
        }

        if !asset_ids.is_empty() {
            let should = asset_ids
                .iter()
                .map(|id| Condition::matches("asset_id", id.0.clone()))
                .collect::<Vec<_>>();
            must.push(qdrant_client::qdrant::Filter::should(should).into());
        }

        if let Some(source_type) = source_type {
            must.push(Condition::matches(
                "source_type",
                source_type.as_str().to_string(),
            ));
        }

        qdrant_client::qdrant::Filter::must(must)
    }

    async fn scroll_with_filter(
        &self,
        filter: qdrant_client::qdrant::Filter,
    ) -> Result<Vec<qdrant_client::qdrant::RetrievedPoint>, CoreError> {
        let mut points = Vec::new();
        let mut offset: Option<qdrant_client::qdrant::PointId> = None;

        loop {
            let mut builder = ScrollPointsBuilder::new(&self.config.collection_name)
                .filter(filter.clone())
                .with_payload(true)
                .limit(256);

            if let Some(next_offset) = offset.clone() {
                builder = builder.offset(next_offset);
            }

            let response = self
                .client
                .scroll(builder)
                .await
                .map_err(|e| CoreError::Storage(format!("qdrant scroll failed: {e}")))?;

            if response.result.is_empty() {
                break;
            }

            points.extend(response.result);
            offset = response.next_page_offset;
            if offset.is_none() {
                break;
            }
        }

        Ok(points)
    }
}

#[async_trait]
impl ChunkRepository for QdrantChunkRepository {
    async fn upsert_chunks(
        &self,
        scope: &Scope,
        chunks: Vec<ChunkRecord>,
    ) -> Result<UpsertSummary, CoreError> {
        info!(
            "Qdrant upsert_chunks: collection={}, tenant={}, namespace={}, chunks_count={}",
            self.config.collection_name,
            scope.tenant_id.0,
            scope.namespace.0,
            chunks.len()
        );

        self.ensure_schema().await?;

        if chunks.is_empty() {
            debug!("Qdrant upsert_chunks: no chunks to upsert, returning early");
            return Ok(UpsertSummary { points_written: 0 });
        }

        let mut points = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            if chunk.embedding.is_empty() {
                error!("Qdrant upsert_chunks: chunk.embedding is empty for asset_id={}, chunk_index={}", 
                       chunk.asset_id.0, chunk.chunk_index);
                return Err(CoreError::Validation(
                    "chunk.embedding cannot be empty when upserting to qdrant".to_string(),
                ));
            }

            let payload = Self::chunk_to_payload(&chunk)?;
            let point = PointStruct::new(Self::point_id(&chunk), chunk.embedding.clone(), payload);
            points.push(point);
        }

        let points_written = points.len();

        debug!(
            "Qdrant upsert_chunks: writing {} points to collection={}",
            points_written, self.config.collection_name
        );

        self.client
            .upsert_points(
                qdrant_client::qdrant::UpsertPointsBuilder::new(
                    &self.config.collection_name,
                    points,
                )
                .wait(true),
            )
            .await
            .map_err(|e| {
                error!("Qdrant upsert_chunks: upsert_points failed: {}", e);
                CoreError::Storage(format!("qdrant upsert failed: {e}"))
            })?;

        info!(
            "Qdrant upsert_chunks: successfully wrote {} points to collection={}",
            points_written, self.config.collection_name
        );

        Ok(UpsertSummary { points_written })
    }

    async fn delete_asset(
        &self,
        scope: &Scope,
        asset_id: &AssetId,
    ) -> Result<DeleteSummary, CoreError> {
        self.ensure_schema().await?;

        let filter = Self::build_scope_filter(scope, Some(asset_id), &[], None);
        let existing_count = self.scroll_with_filter(filter.clone()).await?.len();

        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.config.collection_name)
                    .points(filter)
                    .wait(true),
            )
            .await
            .map_err(|e| CoreError::Storage(format!("qdrant delete failed: {e}")))?;

        Ok(DeleteSummary {
            points_deleted: existing_count,
        })
    }

    async fn get_asset_chunks(
        &self,
        scope: &Scope,
        asset_id: &AssetId,
    ) -> Result<Vec<ChunkRecord>, CoreError> {
        self.ensure_schema().await?;

        let filter = Self::build_scope_filter(scope, Some(asset_id), &[], None);
        let points = self.scroll_with_filter(filter).await?;

        let mut chunks = Vec::with_capacity(points.len());
        for point in points {
            let payload: Payload = point.payload.into();
            let stored: StoredPayload = payload
                .deserialize()
                .map_err(|e| CoreError::Storage(format!("payload deserialize failed: {e}")))?;
            chunks.push(stored.into_chunk_record());
        }

        Ok(chunks)
    }

    async fn list_assets(
        &self,
        scope: &Scope,
        filter: AssetFilter,
    ) -> Result<Vec<AssetSummary>, CoreError> {
        self.ensure_schema().await?;

        let scoped_filter = Self::build_scope_filter(scope, None, &[], filter.source_type.as_ref());
        let points = self.scroll_with_filter(scoped_filter).await?;

        let mut counts: HashMap<String, usize> = HashMap::new();
        for point in points {
            let payload: Payload = point.payload.into();
            let stored: StoredPayload = payload
                .deserialize()
                .map_err(|e| CoreError::Storage(format!("payload deserialize failed: {e}")))?;
            *counts.entry(stored.asset_id).or_insert(0) += 1;
        }

        let mut assets = counts
            .into_iter()
            .map(|(asset_id, chunk_count)| AssetSummary {
                asset_id: AssetId(asset_id),
                chunk_count,
            })
            .collect::<Vec<_>>();
        assets.sort_by(|a, b| a.asset_id.0.cmp(&b.asset_id.0));

        Ok(assets)
    }

    async fn search(
        &self,
        scope: &Scope,
        request: SearchRequest,
    ) -> Result<Vec<ScoredChunk>, CoreError> {
        info!(
            "Qdrant search: collection={}, tenant={}, namespace={}, k={}, asset_ids_count={}",
            self.config.collection_name,
            scope.tenant_id.0,
            scope.namespace.0,
            request.k,
            request.asset_ids.len()
        );

        self.ensure_schema().await?;

        if request.query_vector.is_empty() {
            error!("Qdrant search: query_vector is empty");
            return Err(CoreError::Validation(
                "query_vector cannot be empty".to_string(),
            ));
        }

        let filter = Self::build_scope_filter(scope, None, &request.asset_ids, None);
        let response = self
            .client
            .search_points(
                SearchPointsBuilder::new(
                    &self.config.collection_name,
                    request.query_vector,
                    request.k as u64,
                )
                .filter(filter)
                .with_payload(true),
            )
            .await
            .map_err(|e| {
                error!("Qdrant search: search_points failed: {}", e);
                CoreError::Storage(format!("qdrant search failed: {e}"))
            })?;

        let results_count = response.result.len();
        info!(
            "Qdrant search: found {} results in collection={}",
            results_count, self.config.collection_name
        );

        response
            .result
            .into_iter()
            .map(Self::scored_point_to_chunk)
            .collect::<Result<Vec<_>, _>>()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredPayload {
    tenant_id: String,
    namespace: String,
    asset_id: String,
    actor_id: Option<String>,
    source_type: String,
    source_uri: Option<String>,
    digest: String,
    chunk_index: u32,
    page: Option<u32>,
    path: Option<String>,
    language: Option<String>,
    mime_type: Option<String>,
    title: Option<String>,
    text: String,
    tags: Vec<String>,
    created_at: String,
}

impl StoredPayload {
    fn into_chunk_record(self) -> ChunkRecord {
        let created_at = DateTime::parse_from_rfc3339(&self.created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        ChunkRecord {
            tenant_id: TenantId(self.tenant_id),
            namespace: Namespace(self.namespace),
            asset_id: AssetId(self.asset_id),
            actor_id: self.actor_id.map(ActorId),
            source_type: SourceType::parse(&self.source_type).unwrap_or(SourceType::Text),
            source_uri: self.source_uri,
            digest: self.digest,
            chunk_index: self.chunk_index,
            page: self.page,
            path: self.path,
            language: self.language,
            mime_type: self.mime_type,
            title: self.title,
            text: self.text,
            embedding: vec![],
            tags: self.tags,
            created_at,
        }
    }
}
