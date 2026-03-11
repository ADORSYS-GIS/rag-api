use async_trait::async_trait;
use rag_core::{CoreError, DocumentUnderstandingClient, EmbeddingClient};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::time::{Duration, sleep};

#[derive(Debug, Clone)]
pub struct OpenAiCompatConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub timeout_ms: u64,
    pub max_retries: u32,
}

impl Default for OpenAiCompatConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com".to_string(),
            api_key: std::env::var("OPENAI_API_KEY").ok(),
            timeout_ms: 30_000,
            max_retries: 2,
        }
    }
}

pub struct OpenAiCompatClient {
    http: Client,
    config: OpenAiCompatConfig,
}

impl OpenAiCompatClient {
    pub fn new(config: OpenAiCompatConfig) -> Result<Self, CoreError> {
        if config.base_url.trim().is_empty() {
            return Err(CoreError::Validation(
                "OPENAI_BASE_URL cannot be empty".to_string(),
            ));
        }

        if config.timeout_ms == 0 {
            return Err(CoreError::Validation(
                "OPENAI_TIMEOUT_MS must be greater than zero".to_string(),
            ));
        }

        let http = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| CoreError::Provider(format!("failed to build HTTP client: {e}")))?;

        Ok(Self { http, config })
    }

    fn embeddings_endpoint(&self) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        if base.ends_with("/v1") {
            format!("{base}/embeddings")
        } else {
            format!("{base}/v1/embeddings")
        }
    }

    async fn call_embeddings(
        &self,
        request: EmbeddingsRequest<'_>,
    ) -> Result<EmbeddingsResponse, CoreError> {
        let url = self.embeddings_endpoint();
        let mut attempt: u32 = 0;
        let max_attempts = self.config.max_retries.saturating_add(1);

        loop {
            attempt = attempt.saturating_add(1);

            let mut req = self.http.post(&url).json(&request);
            if let Some(api_key) = self.config.api_key.as_ref().filter(|k| !k.is_empty()) {
                req = req.bearer_auth(api_key);
            }

            let response = match req.send().await {
                Ok(resp) => resp,
                Err(err) if attempt < max_attempts => {
                    tracing::warn!(attempt, max_attempts, error=%err, "embeddings request failed, retrying");
                    sleep(backoff_delay(attempt)).await;
                    continue;
                }
                Err(err) => {
                    return Err(CoreError::Provider(format!(
                        "embeddings request failed: {err}"
                    )));
                }
            };

            let status = response.status();
            if status.is_success() {
                return response.json::<EmbeddingsResponse>().await.map_err(|e| {
                    CoreError::Provider(format!("invalid embeddings response payload: {e}"))
                });
            }

            let body_text = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            let message = parse_error_message(status, &body_text);
            if should_retry_status(status) && attempt < max_attempts {
                tracing::warn!(
                    attempt,
                    max_attempts,
                    status = %status,
                    message,
                    "embeddings upstream returned retryable status"
                );
                sleep(backoff_delay(attempt)).await;
                continue;
            }

            return Err(CoreError::Provider(message));
        }
    }
}

fn should_retry_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn backoff_delay(attempt: u32) -> Duration {
    let multiplier = 1_u64 << attempt.saturating_sub(1).min(6);
    Duration::from_millis(100 * multiplier)
}

fn parse_error_message(status: StatusCode, raw_body: &str) -> String {
    if let Ok(parsed) = serde_json::from_str::<EmbeddingsErrorEnvelope>(raw_body)
        && !parsed.error.message.trim().is_empty()
    {
        return format!(
            "upstream embeddings error ({status}): {}",
            parsed.error.message
        );
    }

    format!("upstream embeddings error ({status}): {raw_body}")
}

#[derive(Debug, Serialize)]
struct EmbeddingsRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingsErrorEnvelope {
    error: EmbeddingsErrorBody,
}

#[derive(Debug, Deserialize)]
struct EmbeddingsErrorBody {
    message: String,
}

#[async_trait]
impl EmbeddingClient for OpenAiCompatClient {
    async fn embed_texts(
        &self,
        model: &str,
        inputs: &[String],
    ) -> Result<Vec<Vec<f32>>, CoreError> {
        if model.trim().is_empty() {
            return Err(CoreError::Validation(
                "embedding model cannot be empty".to_string(),
            ));
        }

        if inputs.is_empty() {
            return Err(CoreError::Validation(
                "embedding input cannot be empty".to_string(),
            ));
        }

        if inputs.iter().any(|item| item.trim().is_empty()) {
            return Err(CoreError::Validation(
                "embedding input contains an empty text item".to_string(),
            ));
        }

        let request = EmbeddingsRequest {
            model,
            input: inputs,
        };
        let response = self.call_embeddings(request).await?;

        if response.data.len() != inputs.len() {
            return Err(CoreError::Provider(format!(
                "embedding response count mismatch: requested {}, got {}",
                inputs.len(),
                response.data.len()
            )));
        }

        let mut vectors = Vec::with_capacity(response.data.len());
        for row in response.data {
            if row.embedding.is_empty() {
                return Err(CoreError::Provider(
                    "embedding response contained an empty vector".to_string(),
                ));
            }
            vectors.push(row.embedding);
        }
        Ok(vectors)
    }

    async fn embed_query(&self, model: &str, input: &str) -> Result<Vec<f32>, CoreError> {
        let mut vectors = self.embed_texts(model, &[input.to_string()]).await?;
        vectors
            .pop()
            .ok_or_else(|| CoreError::Provider("missing embedding vector".to_string()))
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
