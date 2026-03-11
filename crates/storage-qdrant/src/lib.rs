use async_trait::async_trait;
use rag_core::{
    AssetFilter, AssetId, AssetSummary, ChunkRecord, ChunkRepository, CoreError, DeleteSummary,
    Scope, ScoredChunk, SearchRequest, UpsertSummary,
};

pub struct QdrantChunkRepository;

#[async_trait]
impl ChunkRepository for QdrantChunkRepository {
    async fn upsert_chunks(
        &self,
        _scope: &Scope,
        chunks: Vec<ChunkRecord>,
    ) -> Result<UpsertSummary, CoreError> {
        Ok(UpsertSummary {
            points_written: chunks.len(),
        })
    }

    async fn delete_asset(
        &self,
        _scope: &Scope,
        _asset_id: &AssetId,
    ) -> Result<DeleteSummary, CoreError> {
        Err(CoreError::NotImplemented(
            "qdrant delete implementation pending".to_string(),
        ))
    }

    async fn get_asset_chunks(
        &self,
        _scope: &Scope,
        _asset_id: &AssetId,
    ) -> Result<Vec<ChunkRecord>, CoreError> {
        Err(CoreError::NotImplemented(
            "qdrant get chunks implementation pending".to_string(),
        ))
    }

    async fn list_assets(
        &self,
        _scope: &Scope,
        _filter: AssetFilter,
    ) -> Result<Vec<AssetSummary>, CoreError> {
        Err(CoreError::NotImplemented(
            "qdrant list assets implementation pending".to_string(),
        ))
    }

    async fn search(
        &self,
        _scope: &Scope,
        _request: SearchRequest,
    ) -> Result<Vec<ScoredChunk>, CoreError> {
        Err(CoreError::NotImplemented(
            "qdrant search implementation pending".to_string(),
        ))
    }
}
