pub mod auth;
mod credential;

mod amazon_bedrock;
mod anthropic;
mod capabilities;
mod cataloged_models;
mod chat_wire;
mod configured_models;
mod copilot_models;
mod core;
mod gemini;
mod gitlab;
mod model_metadata;
mod model_modes;
mod multi_adapter;
mod ollama;
mod openai;
mod prompt_cache;
mod prompt_cache_shape;
mod protocol_ids;
mod registry;
mod runtime;
mod sse;
mod tool_stream;
mod types;
mod utils;
mod wire_message;

pub use crate::model::{
    AdapterId, CapabilitySupport, Model, ModelCapabilities, ModelId, ModelInputModality,
    ModelLifecycle, ModelMetadata, ModelRef, ModelSpeedMode, ModelSpeedModeRequestOverride,
    ModelThinkingMode, ModelTokenLimits, ProviderId,
};
pub use amazon_bedrock::AmazonBedrockAdapter;
pub use anthropic::{AnthropicAdapter, AnthropicAdapterOptions, AnthropicProfile};
pub use capabilities::{CapabilityFamily, CapabilityRegistry, default_capability_registry};
pub use cataloged_models::CatalogedModelsProvider;
pub use configured_models::{
    CapabilitySelectionPatch, CapabilitySelectionPatchBody, ConfiguredModelDefinition,
    ConfiguredModelSpeedMode, ConfiguredModelThinkingMode, ConfiguredModelsProvider,
    ModelCapabilityFeature, ModelCapabilityPatch,
};
pub use core::{ModelRuntime, StreamResumePolicy};
pub use credential::{
    AuthRefreshStrategy, AuthSecretSelector, ManagedCredential, SapAiCoreServiceKey,
    parse_sap_ai_core_service_key, should_retry_credential,
};
pub use gemini::{GeminiAdapter, GeminiAdapterOptions, GeminiStreamMode};
pub use gitlab::{GitlabProvider, GitlabProviderConfig};
pub(crate) use gitlab::{
    default_ai_gateway_headers as default_gitlab_ai_gateway_headers,
    default_feature_flags as default_gitlab_feature_flags,
};
pub use model_metadata::{ModelMetadataRegistry, default_model_metadata_registry};
pub use model_modes::{ModelModeRegistry, default_model_mode_registry};
pub use multi_adapter::{MultiAdapterProvider, ProviderModelRoute, ProviderModelRouteKey};
pub use ollama::OllamaAdapter;
pub use openai::{
    OpenAiAdapter, OpenAiAdapterOptions, OpenAiApiMode, OpenAiBackend, OpenAiProfile,
    OpenAiStreamMode,
};
pub use prompt_cache_shape::{PromptCacheShape, PromptCacheShapeChange, PromptCacheShapeDiff};
pub use registry::{NamedProvider, ProviderRegistry};
pub use runtime::{
    CODEX_MCP_CLIENT_NAME, CODEX_ORIGINATOR, ProviderClientVersions, ProviderHttpClientConfig,
    ProviderRequestRetryConfig, ProviderRuntimeConfig, ProviderStreamReplayConfig,
    apply_provider_client_version_settings, claude_code_api_user_agent, claude_code_user_agent,
    claude_user_web_fetch_user_agent, codex_package_version, codex_user_agent,
    gemini_cli_user_agent, install_provider_client_versions, provider_client_versions,
    resolve_provider_client_versions,
};
pub(crate) use wire_message::{
    WirePart as ProjectedSessionPart, project as project_session_parts,
    project_operation_output as project_session_tool_result_output,
    project_text_lossy as project_session_text_lossy,
};
pub type ProviderModel = Model;
pub use types::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    CompletionToolCall, CompletionUsage, ReasoningEffort, ResponseFormat,
    ResponsesApiRequestMetadata, ThinkingDisplay, ThinkingRequest,
};
