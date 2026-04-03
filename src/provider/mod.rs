pub mod auth;

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
mod openai;
mod openai_compatible;
mod registry;
mod runtime;
mod sse;
mod types;
mod utils;

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
pub use gemini::GeminiProvider;
pub use gitlab::{GitlabProvider, GitlabProviderConfig};
pub use google_vertex::GoogleVertexProvider;
pub use openai::{OpenAiApiMode, OpenAiProvider, OpenAiStreamMode};
pub use openai_compatible::{OpenAiCompatibleProvider, OpenAiCompatibleStreamMode};
pub use registry::{NamedProvider, ProviderAliasRegistration, ProviderRegistry};
pub use runtime::{
    ProviderHttpClientConfig, ProviderRequestRetryConfig, ProviderRuntimeConfig,
    ProviderStreamReplayConfig,
};
pub use types::{
    CapabilitySupport, CompletionFinishReason, CompletionRequest, CompletionResponse,
    CompletionStreamEvent, CompletionToolCall, CompletionUsage, ModelCapabilities,
    ModelInputModality, ProviderModel,
};
