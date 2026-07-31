use agena_domain::*;
use agena_domain::{ModelCapabilities, ModelMetadata};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, LazyLock},
};

use crate::{
    ProviderError,
    provider::{
        AmazonBedrockAdapter, AnthropicAdapter, AnthropicAdapterOptions, CatalogedModelsProvider,
        GeminiAdapter, GeminiAdapterOptions, GitlabProvider, ManagedCredential, ModelId,
        ModelRuntime, ModelSpeedMode, ModelThinkingMode, MultiAdapterProvider, OllamaAdapter,
        OpenAiChatCompletionsAdapter, OpenAiChatCompletionsAdapterOptions, OpenAiRealtimeAdapter,
        OpenAiRealtimeAdapterOptions, OpenAiResponsesAdapter, OpenAiResponsesAdapterOptions,
        ProviderModelRoute, ProviderRegistry, parse_sap_ai_core_service_key,
    },
};
use agena_provider::CompletionRequest;
use agena_provider::{
    AuthData, ModelCatalogSnapshot, PromptCacheShape, ProviderProtocolPathsConfig,
    StreamResumePolicy, cline_api_protocol_paths,
};

use agena_runtime_config::{
    ConfigEnvironment, ConfigError, HttpProviderAdapterConfig, ProviderAdapterDefinition,
    ProviderApiAuthConfig, ProviderAuthConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig,
};

use agena_runtime_config::config::raw::parse_adapter_model_ref;
const LIST_MODELS_DEFAULT_MODEL_ID: &str = "__list_models__";
static CLINE_API_PROTOCOL_PATHS: LazyLock<ProviderProtocolPathsConfig> =
    LazyLock::new(cline_api_protocol_paths);

#[derive(Debug, Clone)]
pub struct ProviderAdapterModelsResult {
    pub adapter_id: String,
    pub enabled: bool,
    pub resolved_base_url: Option<String>,
    pub models: Vec<Model>,
    pub failure: Option<agena_failure::Failure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitlabRoutedBackend {
    OpenAiResponses,
    OpenAiChatCompletions,
    Anthropic,
}

impl GitlabRoutedBackend {
    fn label(self) -> &'static str {
        match self {
            Self::OpenAiResponses => "openai_responses",
            Self::OpenAiChatCompletions => "openai_chat_completions",
            Self::Anthropic => "anthropic",
        }
    }

    fn matches_model(self, model: &ModelId) -> bool {
        let mapped = GitlabProvider::mapped_model(model.as_ref());
        match self {
            Self::OpenAiResponses => {
                GitlabProvider::use_openai_backend(mapped.as_ref())
                    && GitlabProvider::use_responses_api(mapped.as_ref())
            }
            Self::OpenAiChatCompletions => {
                GitlabProvider::use_openai_backend(mapped.as_ref())
                    && !GitlabProvider::use_responses_api(mapped.as_ref())
            }
            Self::Anthropic => !GitlabProvider::use_openai_backend(mapped.as_ref()),
        }
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

    fn backend_mismatch_error(&self, model: &ModelId) -> ProviderError {
        ProviderError::Config(format!(
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

    fn capability_family(&self) -> Option<agena_provider::CapabilityFamily> {
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

    fn model_thinking_modes(&self, model: &ModelId) -> Vec<ModelThinkingMode> {
        if self.supports_model(model) {
            self.inner.model_thinking_modes(model)
        } else {
            Vec::new()
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

    async fn list_models(&self) -> Result<Vec<crate::provider::Model>, ProviderError> {
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
        request: CompletionRequest,
    ) -> Result<agena_provider::CompletionResponse, ProviderError> {
        if !self.supports_model(&request.model) {
            return Err(self.backend_mismatch_error(&request.model));
        }
        self.inner.complete(request).await
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_core::Stream<
                        Item = Result<agena_provider::CompletionStreamEvent, ProviderError>,
                    > + Send,
            >,
        >,
        ProviderError,
    > {
        if !self.supports_model(&request.model) {
            return Err(self.backend_mismatch_error(&request.model));
        }
        self.inner.complete_stream(request).await
    }
}

mod auth_resolution;
mod model_listing;
mod provider_registry;

pub(crate) use self::auth_resolution::*;
pub use self::model_listing::list_provider_adapter_models;
pub(crate) use self::model_listing::*;
pub use self::provider_registry::build_provider_registry_from_configs;
pub(crate) use self::provider_registry::*;
