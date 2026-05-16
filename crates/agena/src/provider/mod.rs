pub mod auth;
mod credential;

mod amazon_bedrock;
mod anthropic;
mod capabilities;
mod cataloged_models;
mod chat_wire;
mod configured_models;
mod core;
mod gemini;
mod gitlab;
mod model_metadata;
mod multi_adapter;
mod ollama;
mod openai;
mod openai_compatible;
mod prompt_cache;
mod prompt_cache_shape;
mod registry;
mod remote_model_catalog_cache;
mod runtime;
mod sse;
mod types;
mod utils;
mod wire_message;

pub use crate::model::{
    CapabilitySupport, Model, ModelCapabilities, ModelFamily, ModelId, ModelInputModality,
    ModelLifecycle, ModelMetadata, ModelRef, ModelTokenLimits, ModelVariant, ProviderId,
};
pub use amazon_bedrock::AmazonBedrockProvider;
pub use anthropic::{AnthropicProfile, AnthropicProvider};
pub use capabilities::{CapabilityFamily, CapabilityRegistry, default_capability_registry};
pub use cataloged_models::CatalogedModelsProvider;
pub use configured_models::{
    ConfiguredModelDefinition, ConfiguredModelVariant, ConfiguredModelsProvider,
    ModelCapabilityFeature, ModelCapabilityPatch,
};
pub use core::{ModelProvider, StreamResumePolicy};
pub use credential::{
    AuthRefreshStrategy, AuthSecretSelector, ManagedCredential, SapAiCoreServiceKey,
    parse_sap_ai_core_service_key, should_retry_credential,
};
pub use gemini::GeminiProvider;
pub use gitlab::{GitlabProvider, GitlabProviderConfig};
pub use model_metadata::{ModelMetadataRegistry, default_model_metadata_registry};
pub use multi_adapter::{MultiAdapterProvider, ProviderModelRoute};
pub use ollama::OllamaProvider;
pub use openai::{OpenAiApiMode, OpenAiBackend, OpenAiProfile, OpenAiProvider, OpenAiStreamMode};
pub use openai_compatible::{OpenAiCompatibleProvider, OpenAiCompatibleStreamMode};
pub use prompt_cache_shape::{PromptCacheShape, PromptCacheShapeChange, PromptCacheShapeDiff};
pub use registry::{NamedProvider, ProviderRegistry};
pub use runtime::{
    ProviderHttpClientConfig, ProviderRequestRetryConfig, ProviderRuntimeConfig,
    ProviderStreamReplayConfig,
};
pub(crate) use wire_message::{
    PRUNED_TOOL_RESULT_PLACEHOLDER, WirePart as ProjectedSessionPart,
    project as project_session_parts, project_text_lossy as project_session_text_lossy,
};
pub type ProviderModel = Model;
pub use types::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    CompletionToolCall, CompletionUsage, ReasoningEffort, ResponseFormat, ThinkingRequest,
};
