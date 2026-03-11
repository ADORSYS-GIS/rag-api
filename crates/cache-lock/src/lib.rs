use async_trait::async_trait;
use rag_core::{AssetId, AssetLockManager, CoreError, Namespace, QueryCache, TenantId};

pub struct RedisLockManager;

#[async_trait]
impl AssetLockManager for RedisLockManager {
    async fn acquire_asset_lock(
        &self,
        _tenant_id: &TenantId,
        _namespace: &Namespace,
        _asset_id: &AssetId,
    ) -> Result<(), CoreError> {
        Err(CoreError::NotImplemented(
            "redis lock implementation pending".to_string(),
        ))
    }
}

pub struct RedisQueryCache;

#[async_trait]
impl QueryCache for RedisQueryCache {
    async fn get_query_embedding(&self, _key: &str) -> Result<Option<Vec<f32>>, CoreError> {
        Err(CoreError::NotImplemented(
            "redis query cache implementation pending".to_string(),
        ))
    }

    async fn put_query_embedding(
        &self,
        _key: &str,
        _vector: Vec<f32>,
        _ttl_secs: u64,
    ) -> Result<(), CoreError> {
        Err(CoreError::NotImplemented(
            "redis query cache implementation pending".to_string(),
        ))
    }
}
