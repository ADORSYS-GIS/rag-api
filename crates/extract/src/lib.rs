use std::time::Duration;

use async_trait::async_trait;
use rag_core::{CoreError, ExtractRequest, ExtractResponse, ExtractService, RequestContext};

const MAX_SOURCE_BYTES: usize = 5 * 1024 * 1024; // 5 MiB

/// Extraction service that resolves a `source_uri` (local file or HTTP/S URL)
/// into plain text. Pre-extracted `content` is passed through unchanged for
/// backward compatibility with callers that already hold the text.
pub struct SourceExtractService {
    http: reqwest::Client,
}

impl SourceExtractService {
    pub fn new() -> Result<Self, CoreError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("rag-mcp/1.0")
            .build()
            .map_err(|e| CoreError::Provider(format!("failed to build HTTP client: {e}")))?;
        Ok(Self { http })
    }
}

#[async_trait]
impl ExtractService for SourceExtractService {
    async fn extract(
        &self,
        _ctx: RequestContext,
        request: ExtractRequest,
    ) -> Result<ExtractResponse, CoreError> {
        // Passthrough — caller already has text
        if let Some(content) = request.content {
            if content.trim().is_empty() {
                return Err(CoreError::Validation("content cannot be empty".to_string()));
            }
            return Ok(ExtractResponse { text: content });
        }

        let uri = request.source_uri.as_deref().ok_or_else(|| {
            CoreError::Validation("either content or source_uri must be provided".to_string())
        })?;

        tracing::debug!(uri, "extracting from source");

        let (raw, detected_mime) = fetch_source(&self.http, uri).await?;

        // Size guard (belt-and-suspenders; fetch_source also checks for files)
        if raw.len() > MAX_SOURCE_BYTES {
            return Err(CoreError::Validation(format!(
                "source exceeds 5 MiB limit ({} bytes)",
                raw.len()
            )));
        }

        let mime = detected_mime.as_deref().unwrap_or("text/plain");
        let text = to_text(raw, mime).await?;

        if text.trim().is_empty() {
            return Err(CoreError::Validation("extracted text is empty".to_string()));
        }

        Ok(ExtractResponse { text })
    }
}

async fn fetch_source(
    http: &reqwest::Client,
    uri: &str,
) -> Result<(Vec<u8>, Option<String>), CoreError> {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        fetch_url(http, uri).await
    } else {
        read_file(uri).await
    }
}

async fn fetch_url(
    http: &reqwest::Client,
    url: &str,
) -> Result<(Vec<u8>, Option<String>), CoreError> {
    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| CoreError::Provider(format!("HTTP request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(CoreError::Provider(format!(
            "HTTP {} fetching {url}",
            resp.status()
        )));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_string());

    // Bail early if Content-Length already exceeds the limit
    if let Some(len) = resp.content_length() {
        if len > MAX_SOURCE_BYTES as u64 {
            return Err(CoreError::Validation(format!(
                "source exceeds 5 MiB limit ({len} bytes)"
            )));
        }
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| CoreError::Provider(format!("failed to read response body: {e}")))?;

    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(CoreError::Validation(format!(
            "source exceeds 5 MiB limit ({} bytes)",
            bytes.len()
        )));
    }

    Ok((bytes.to_vec(), content_type))
}

async fn read_file(path: &str) -> Result<(Vec<u8>, Option<String>), CoreError> {
    let path = path.strip_prefix("file://").unwrap_or(path);

    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| CoreError::Provider(format!("cannot access '{path}': {e}")))?;

    if meta.len() > MAX_SOURCE_BYTES as u64 {
        return Err(CoreError::Validation(format!(
            "source exceeds 5 MiB limit ({} bytes)",
            meta.len()
        )));
    }

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| CoreError::Provider(format!("cannot read '{path}': {e}")))?;

    let mime = mime_guess::from_path(path).first().map(|m| m.to_string());

    Ok((bytes, mime))
}

async fn to_text(raw: Vec<u8>, mime: &str) -> Result<String, CoreError> {
    let base = mime.split(';').next().unwrap_or(mime).trim();

    if base == "application/pdf" {
        return tokio::task::spawn_blocking(move || {
            pdf_extract::extract_text_from_mem(&raw)
                .map_err(|e| CoreError::Provider(format!("PDF extraction failed: {e}")))
        })
        .await
        .map_err(|e| CoreError::Provider(format!("PDF task panicked: {e}")))?;
    }

    if base == "text/html" {
        let html = String::from_utf8_lossy(&raw).into_owned();
        let doc = scraper::Html::parse_document(&html);
        let sel = scraper::Selector::parse("body").unwrap();
        let text = doc
            .select(&sel)
            .next()
            .map(|b| b.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_else(|| doc.root_element().text().collect::<Vec<_>>().join(" "));
        return Ok(text.split_whitespace().collect::<Vec<_>>().join(" "));
    }

    // Everything else: UTF-8 text (plain text, markdown, code, JSON, CSV, …)
    String::from_utf8(raw)
        .map_err(|_| CoreError::Provider("file content is not valid UTF-8".to_string()))
}
