use std::sync::Arc;

use agena_provider::ModelCatalogDocument;
use futures_util::StreamExt;
use thiserror::Error;

use crate::{
    ModelCatalogConfiguredPublicSource, ModelCatalogPublicSource,
    ModelCatalogRemoteDocumentFetcher, ModelCatalogRemoteSource,
};
use crate::{default_public_model_catalog_sources, public_model_catalog_sources_enabled};

#[derive(Debug, Error)]
/// Error from the model catalog HTTP source.
pub enum ModelCatalogHttpError {
    #[error("model catalog source error: {0}")]
    Source(String),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

const MAX_MODEL_CATALOG_SOURCE_BYTES: usize = 8 * 1024 * 1024;

pub fn build_model_catalog_public_source(
    user_agent: impl Into<String>,
    sources: Vec<ModelCatalogRemoteSource>,
) -> Result<Arc<dyn ModelCatalogPublicSource>, ModelCatalogHttpError> {
    let user_agent = user_agent.into();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(user_agent)
        .build()?;
    Ok(Arc::new(ModelCatalogConfiguredPublicSource::new(
        Arc::new(HttpModelCatalogDocumentFetcher { client }),
        sources,
    )))
}

/// Builds the enabled default public-source adapter. Runtime owns both the
/// executable source list and the environment enablement policy; composition
/// supplies only process-specific HTTP identity.
pub fn build_default_public_model_catalog_source(
    user_agent: impl Into<String>,
) -> Result<Arc<dyn ModelCatalogPublicSource>, ModelCatalogHttpError> {
    let sources = public_model_catalog_sources_enabled()
        .then(default_public_model_catalog_sources)
        .unwrap_or_default();
    build_model_catalog_public_source(user_agent, sources)
}

struct HttpModelCatalogDocumentFetcher {
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl ModelCatalogRemoteDocumentFetcher for HttpModelCatalogDocumentFetcher {
    async fn fetch_document(
        &self,
        source: &ModelCatalogRemoteSource,
    ) -> Result<ModelCatalogDocument, String> {
        let mut last_error = None;

        for url in &source.urls {
            match self.fetch_and_parse_source_document(url).await {
                Ok(document) => return Ok(document),
                Err(error) => last_error = Some(format!("{url}: {error}")),
            }
        }

        Err(ModelCatalogHttpError::Source(format!(
            "all source URLs failed for {}: {}",
            source.name,
            last_error.unwrap_or_else(|| "no URLs configured".to_owned())
        ))
        .to_string())
    }
}

impl HttpModelCatalogDocumentFetcher {
    async fn fetch_and_parse_source_document(
        &self,
        url: &str,
    ) -> Result<ModelCatalogDocument, ModelCatalogHttpError> {
        let response = self.client.get(url).send().await?.error_for_status()?;
        let body = response_text_bounded(response).await?;
        let document: ModelCatalogDocument = serde_json::from_str(body.as_str())?;
        if document.models.is_empty() {
            return Err(ModelCatalogHttpError::Source(
                "catalog document contains no models".to_owned(),
            ));
        }
        Ok(document)
    }
}

async fn response_text_bounded(
    response: reqwest::Response,
) -> Result<String, ModelCatalogHttpError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MODEL_CATALOG_SOURCE_BYTES as u64)
    {
        return Err(ModelCatalogHttpError::Source(
            "model catalog source exceeds the 8 MiB limit".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > MAX_MODEL_CATALOG_SOURCE_BYTES {
            return Err(ModelCatalogHttpError::Source(
                "model catalog source exceeds the 8 MiB limit".to_owned(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| {
        ModelCatalogHttpError::Source("model catalog source is not UTF-8 text".to_owned())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agena_provider::ModelCatalogDocument;

    use crate::{
        ModelCatalogConfiguredPublicSource, ModelCatalogPublicSource,
        ModelCatalogRemoteDocumentFetcher, ModelCatalogRemoteSource, ModelCatalogRemoteSourceKind,
        default_public_model_catalog_sources, public_model_catalog_sources_enabled,
    };

    #[test]
    fn default_sources_point_at_the_github_catalog() {
        let sources = default_public_model_catalog_sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].kind, ModelCatalogRemoteSourceKind::GithubCatalog);
        assert!(
            sources[0]
                .urls
                .iter()
                .any(|url| url.contains("raw.githubusercontent.com/canxin121"))
        );
    }

    #[test]
    fn public_source_toggle_defaults_to_enabled() {
        if std::env::var_os("AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES").is_none() {
            assert!(public_model_catalog_sources_enabled());
        }
    }

    struct FixtureFetcher;

    #[async_trait::async_trait]
    impl ModelCatalogRemoteDocumentFetcher for FixtureFetcher {
        async fn fetch_document(
            &self,
            source: &ModelCatalogRemoteSource,
        ) -> Result<ModelCatalogDocument, String> {
            if source.name == "unavailable" {
                return Err("offline".to_owned());
            }
            let document: ModelCatalogDocument =
                serde_json::from_str(r#"{"models":{"gpt-4o":{"origin":"OpenAI"}}}"#)
                    .expect("fixture catalog");
            Ok(document)
        }
    }

    #[tokio::test]
    async fn configured_public_source_composes_the_concrete_fetcher() {
        let source = ModelCatalogConfiguredPublicSource::new(
            Arc::new(FixtureFetcher),
            vec![ModelCatalogRemoteSource::new(
                "available",
                ModelCatalogRemoteSourceKind::GithubCatalog,
                ["https://example.invalid/available".to_owned()],
            )],
        );
        let result = source.fetch().await;
        assert_eq!(result.succeeded, 1);
        assert!(result.models.contains_key("gpt-4o"));
    }
}
