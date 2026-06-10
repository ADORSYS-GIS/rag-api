use std::sync::Arc;

use async_trait::async_trait;
use rag_core::{CoreError, ExtractRequest, ExtractResponse, ExtractService, RequestContext};

/// Wraps any `ExtractService` and rewrites local file paths in `source_uri`
/// by prepending `host_root` before delegating to the inner service.
///
/// When the gateway runs in Docker with `-v /:/host:ro`, setting
/// `GATEWAY_HOST_ROOT=/host` means a client path like `/home/user/doc.pdf`
/// becomes `/host/home/user/doc.pdf` inside the container — no per-path
/// volume declarations needed.
///
/// HTTP/S and `file://` URIs are passed through unchanged.
/// When `host_root` is empty (native / host-side binary), paths are also
/// passed through unchanged.
pub struct RemappingExtractService {
    inner: Arc<dyn ExtractService>,
    host_root: String,
}

impl RemappingExtractService {
    pub fn new(inner: Arc<dyn ExtractService>, host_root: impl Into<String>) -> Self {
        Self {
            inner,
            host_root: host_root.into(),
        }
    }

    fn remap(&self, uri: &str) -> String {
        if self.host_root.is_empty()
            || uri.starts_with("http://")
            || uri.starts_with("https://")
            || uri.starts_with("file://")
        {
            return uri.to_string();
        }
        format!(
            "{}/{}",
            self.host_root.trim_end_matches('/'),
            uri.trim_start_matches('/')
        )
    }
}

#[async_trait]
impl ExtractService for RemappingExtractService {
    async fn extract(
        &self,
        ctx: RequestContext,
        request: ExtractRequest,
    ) -> Result<ExtractResponse, CoreError> {
        let source_uri = request.source_uri.as_deref().map(|u| self.remap(u));
        self.inner
            .extract(
                ctx,
                ExtractRequest {
                    source_uri,
                    ..request
                },
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(host_root: &str) -> RemappingExtractService {
        struct Noop;
        #[async_trait]
        impl ExtractService for Noop {
            async fn extract(
                &self,
                _: RequestContext,
                r: ExtractRequest,
            ) -> Result<ExtractResponse, CoreError> {
                Ok(ExtractResponse {
                    text: r.source_uri.unwrap_or_default(),
                })
            }
        }
        RemappingExtractService::new(Arc::new(Noop), host_root)
    }

    #[tokio::test]
    async fn prepends_host_root_to_absolute_path() {
        let s = svc("/host");
        let req = ExtractRequest {
            scope: rag_core::Scope {
                tenant_id: rag_core::TenantId("t".into()),
                namespace: rag_core::Namespace("n".into()),
            },
            source_type: rag_core::SourceType::LocalFile,
            source_uri: Some("/home/user/doc.pdf".into()),
            content: None,
        };
        let ctx = rag_core::RequestContext {
            tenant_id: rag_core::TenantId("t".into()),
            actor_id: None,
            roles: vec![],
            allowed_namespaces: vec![],
            request_id: "r".into(),
        };
        let res = s.extract(ctx, req).await.unwrap();
        assert_eq!(res.text, "/host/home/user/doc.pdf");
    }

    #[tokio::test]
    async fn leaves_http_uri_unchanged() {
        let s = svc("/host");
        let req = ExtractRequest {
            scope: rag_core::Scope {
                tenant_id: rag_core::TenantId("t".into()),
                namespace: rag_core::Namespace("n".into()),
            },
            source_type: rag_core::SourceType::Website,
            source_uri: Some("https://example.com/page".into()),
            content: None,
        };
        let ctx = rag_core::RequestContext {
            tenant_id: rag_core::TenantId("t".into()),
            actor_id: None,
            roles: vec![],
            allowed_namespaces: vec![],
            request_id: "r".into(),
        };
        let res = s.extract(ctx, req).await.unwrap();
        assert_eq!(res.text, "https://example.com/page");
    }

    #[tokio::test]
    async fn no_op_when_host_root_empty() {
        let s = svc("");
        let req = ExtractRequest {
            scope: rag_core::Scope {
                tenant_id: rag_core::TenantId("t".into()),
                namespace: rag_core::Namespace("n".into()),
            },
            source_type: rag_core::SourceType::LocalFile,
            source_uri: Some("/etc/hostname".into()),
            content: None,
        };
        let ctx = rag_core::RequestContext {
            tenant_id: rag_core::TenantId("t".into()),
            actor_id: None,
            roles: vec![],
            allowed_namespaces: vec![],
            request_id: "r".into(),
        };
        let res = s.extract(ctx, req).await.unwrap();
        assert_eq!(res.text, "/etc/hostname");
    }
}
