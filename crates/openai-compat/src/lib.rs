use async_trait::async_trait;
use rag_core::{CoreError, DocumentUnderstandingClient, EmbeddingClient};

pub struct OpenAiCompatClient;

#[async_trait]
impl EmbeddingClient for OpenAiCompatClient {
    async fn embed_texts(
        &self,
        _model: &str,
        _inputs: &[String],
    ) -> Result<Vec<Vec<f32>>, CoreError> {
        Err(CoreError::NotImplemented(
            "openai-compatible embeddings implementation pending".to_string(),
        ))
    }

    async fn embed_query(&self, _model: &str, _input: &str) -> Result<Vec<f32>, CoreError> {
        Err(CoreError::NotImplemented(
            "openai-compatible query embedding implementation pending".to_string(),
        ))
    }
}

#[async_trait]
impl DocumentUnderstandingClient for OpenAiCompatClient {
    async fn describe_image(&self, _uri: &str) -> Result<String, CoreError> {
        Err(CoreError::NotImplemented(
            "openai-compatible image understanding implementation pending".to_string(),
        ))
    }
}
