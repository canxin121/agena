pub mod auth;
mod credential;

mod amazon_bedrock;
mod anthropic;
mod capabilities;
mod capability_overrides;
mod cloudflare_ai_gateway;
mod codex;
mod copilot;
mod core;
mod gemini;
mod gitlab;
mod google_vertex;
mod model_metadata;
mod openai;
mod openai_compatible;
mod prompt_cache;
mod registry;
mod runtime;
mod sse;
mod types;
mod utils;

pub use crate::model::{
    CapabilitySupport, Model, ModelCapabilities, ModelFamily, ModelId, ModelInputModality,
    ModelLifecycle, ModelMetadata, ModelRef, ModelTokenLimits, ProviderId,
};
pub use amazon_bedrock::AmazonBedrockProvider;
pub use anthropic::AnthropicProvider;
pub use capabilities::{CapabilityFamily, CapabilityRegistry, default_capability_registry};
pub use capability_overrides::{
    CapabilityOverrideMatchMode, CapabilityOverrideProvider, ModelCapabilityPatch,
    ProviderCapabilityOverrideRule,
};
pub use cloudflare_ai_gateway::CloudflareAiGatewayProvider;
pub use codex::CodexProvider;
pub use copilot::{CopilotProvider, CopilotProviderOptions};
pub use core::{ModelProvider, StreamResumePolicy};
pub use credential::{
    AuthRefreshStrategy, AuthSecretSelector, ManagedCredential, SapAiCoreServiceKey,
    parse_sap_ai_core_service_key, should_retry_credential,
};
pub use gemini::GeminiProvider;
pub use gitlab::{GitlabProvider, GitlabProviderConfig};
pub use google_vertex::GoogleVertexProvider;
pub use model_metadata::{ModelMetadataRegistry, default_model_metadata_registry};
pub use openai::{OpenAiApiMode, OpenAiProvider, OpenAiStreamMode};
pub use openai_compatible::{OpenAiCompatibleProvider, OpenAiCompatibleStreamMode};
pub use registry::{NamedProvider, ProviderAliasRegistration, ProviderRegistry};
pub use runtime::{
    ProviderHttpClientConfig, ProviderRequestRetryConfig, ProviderRuntimeConfig,
    ProviderStreamReplayConfig,
};
pub(crate) use utils::{
    PRUNED_TOOL_RESULT_PLACEHOLDER, ProjectedSessionPart, project_session_parts,
    project_session_text_lossy,
};
pub type ProviderModel = Model;
pub use types::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    CompletionToolCall, CompletionUsage,
};
