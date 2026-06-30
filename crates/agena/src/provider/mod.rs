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
pub use anthropic::{AnthropicAdapter, AnthropicProfile};
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
pub use gemini::{GeminiAdapter, GeminiStreamMode};
pub use gitlab::{GitlabProvider, GitlabProviderConfig};
pub use model_metadata::{ModelMetadataRegistry, default_model_metadata_registry};
pub use model_modes::{ModelModeRegistry, default_model_mode_registry};
pub use multi_adapter::{MultiAdapterProvider, ProviderModelRoute, ProviderModelRouteKey};
pub use ollama::OllamaAdapter;
pub use openai::{OpenAiAdapter, OpenAiApiMode, OpenAiBackend, OpenAiProfile, OpenAiStreamMode};
pub use prompt_cache_shape::{PromptCacheShape, PromptCacheShapeChange, PromptCacheShapeDiff};
pub use registry::{NamedProvider, ProviderRegistry};
pub use runtime::{
    CLAUDE_CODE_API_USER_AGENT, CLAUDE_CODE_USER_AGENT, CLAUDE_CODE_VERSION,
    CLAUDE_USER_WEB_FETCH_USER_AGENT, CODEX_MCP_CLIENT_NAME, CODEX_ORIGINATOR,
    CODEX_PACKAGE_VERSION, CODEX_USER_AGENT, GEMINI_CLI_USER_AGENT_PREFIX, GEMINI_CLI_VERSION,
    ProviderHttpClientConfig, ProviderRequestRetryConfig, ProviderRuntimeConfig,
    ProviderStreamReplayConfig, claude_code_api_user_agent, claude_code_user_agent,
    claude_user_web_fetch_user_agent, codex_user_agent, gemini_cli_user_agent,
};
pub(crate) use wire_message::{
    WirePart as ProjectedSessionPart, project as project_session_parts,
    project_text_lossy as project_session_text_lossy,
};
pub type ProviderModel = Model;
pub use types::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    CompletionToolCall, CompletionUsage, ReasoningEffort, ResponseFormat, ThinkingDisplay,
    ThinkingRequest,
};
