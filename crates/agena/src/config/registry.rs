use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use crate::{
    error::AppError,
    model::{AdapterId, ProviderId},
    model_catalog::ModelCatalogSnapshot,
    plugin::{PluginHost, PluginHostBuildConfig},
    provider::{
        AmazonBedrockAdapter, AnthropicAdapter, AnthropicAdapterOptions, AnthropicProfile,
        AuthRefreshStrategy, AuthSecretSelector, CatalogedModelsProvider, GeminiAdapter,
        GeminiAdapterOptions, GitlabProvider, GitlabProviderConfig, ModelCapabilities, ModelId,
        ModelMetadata, ModelRuntime, ModelSpeedMode, ModelThinkingMode, MultiAdapterProvider,
        OllamaAdapter, OpenAiAdapter, OpenAiAdapterOptions, PromptCacheShape, ProviderModelRoute,
        ProviderRegistry, StreamResumePolicy, auth::AuthData, parse_sap_ai_core_service_key,
    },
};

use super::raw::parse_adapter_model_ref;
use super::{
    ConfigEnvironment, ConfigError, HttpProviderAdapterConfig, ProcessEnvironment,
    ProviderAdapterDefinition, ProviderApiAuthConfig, ProviderAuthConfig,
    ProviderCredentialAuthConfig, ProviderModelDiscoveryConfig, ProviderProtocolPathsConfig,
    ResolvedConfig, ResolvedProviderAdapterConfig, ResolvedProviderConfig,
    cline_api_protocol_paths,
};
const LIST_MODELS_DEFAULT_MODEL_ID: &str = "__list_models__";
static CLINE_API_PROTOCOL_PATHS: LazyLock<ProviderProtocolPathsConfig> =
    LazyLock::new(cline_api_protocol_paths);

#[derive(Debug, Clone)]
pub struct ProviderAdapterModelsResult {
    pub adapter_id: String,
    pub enabled: bool,
    pub resolved_base_url: Option<String>,
    pub models: Vec<crate::provider::ProviderModel>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitlabRoutedBackend {
    OpenAi,
    Anthropic,
}

impl GitlabRoutedBackend {
    fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    fn matches_model(self, model: &ModelId) -> bool {
        let mapped = GitlabProvider::mapped_model(model.as_ref());
        GitlabProvider::use_openai_backend(mapped.as_ref()) == matches!(self, Self::OpenAi)
    }
}

#[derive(Clone)]
struct GitlabRoutedAdapter {
    inner: Arc<GitlabProvider>,
    backend: GitlabRoutedBackend,
    default_model: ModelId,
}

impl GitlabRoutedAdapter {
    fn supports_model(&self, model: &ModelId) -> bool {
        self.backend.matches_model(model)
    }

    fn backend_mismatch_error(&self, model: &ModelId) -> AppError {
        AppError::Config(format!(
            "gitlab auth routed adapter `{}` does not support model `{}`",
            self.backend.label(),
            model
        ))
    }
}

#[async_trait::async_trait]
impl ModelRuntime for GitlabRoutedAdapter {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    fn capability_family(&self) -> Option<crate::provider::CapabilityFamily> {
        self.inner.capability_family()
    }

    fn model_capabilities(&self, model: &ModelId) -> ModelCapabilities {
        if self.supports_model(model) {
            self.inner.model_capabilities(model)
        } else {
            ModelCapabilities::default()
        }
    }

    fn model_metadata(&self, model: &ModelId) -> ModelMetadata {
        if self.supports_model(model) {
            self.inner.model_metadata(model)
        } else {
            ModelMetadata::default()
        }
    }

    fn model_thinking_modes(&self, model: &ModelId) -> BTreeMap<String, ModelThinkingMode> {
        if self.supports_model(model) {
            self.inner.model_thinking_modes(model)
        } else {
            BTreeMap::new()
        }
    }

    fn model_speed_modes(&self, model: &ModelId) -> BTreeMap<String, ModelSpeedMode> {
        if self.supports_model(model) {
            self.inner.model_speed_modes(model)
        } else {
            BTreeMap::new()
        }
    }

    fn stream_resume_policy(&self) -> StreamResumePolicy {
        self.inner.stream_resume_policy()
    }

    fn supports_prompt_continuation(&self, model: &ModelId) -> bool {
        self.supports_model(model) && self.inner.supports_prompt_continuation(model)
    }

    fn prompt_cache_shape(&self, model: &ModelId) -> Option<PromptCacheShape> {
        self.supports_model(model)
            .then(|| self.inner.prompt_cache_shape(model))
            .flatten()
    }

    async fn list_models(&self) -> Result<Vec<crate::provider::Model>, AppError> {
        Ok(self
            .inner
            .list_models()
            .await?
            .into_iter()
            .filter(|model| self.supports_model(&model.id))
            .collect())
    }

    async fn complete(
        &self,
        request: crate::provider::CompletionRequest,
    ) -> Result<crate::provider::CompletionResponse, AppError> {
        if !self.supports_model(&request.model) {
            return Err(self.backend_mismatch_error(&request.model));
        }
        self.inner.complete(request).await
    }

    async fn complete_stream(
        &self,
        request: crate::provider::CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_core::Stream<
                        Item = Result<crate::provider::CompletionStreamEvent, AppError>,
                    > + Send,
            >,
        >,
        AppError,
    > {
        if !self.supports_model(&request.model) {
            return Err(self.backend_mismatch_error(&request.model));
        }
        self.inner.complete_stream(request).await
    }
}

mod auth_resolution;
mod model_listing;
mod plugin_host;
mod provider_registry;

pub(crate) use self::auth_resolution::*;
pub use self::model_listing::list_provider_adapter_models;
pub(crate) use self::model_listing::*;
pub(crate) use self::provider_registry::*;
