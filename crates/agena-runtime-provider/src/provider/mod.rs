pub mod auth;
mod credential;

mod amazon_bedrock;
mod anthropic;
mod catalog_decoration;
mod cataloged_models;
mod chat_wire;
mod core;
mod gemini;
mod gitlab;
mod multi_adapter;
mod ollama;
mod openai;
mod prompt_tool_transport;
mod registry;
mod tool_mode;
mod utils;
mod wire_message;

pub(crate) use crate::provider_sse as sse;
use agena_provider::CompletionResponse;
pub(crate) use agena_provider::{
    self as copilot_models, self as prompt_cache, self as protocol_ids, self as tool_stream,
};
pub use utils::{
    ProviderRequestHeaderHook, install_request_header_hook, with_request_cancellation,
};

pub use agena_domain::{Model, ModelId, ModelSpeedMode, ModelThinkingMode};
pub use amazon_bedrock::AmazonBedrockAdapter;
pub use anthropic::{AnthropicAdapter, AnthropicAdapterOptions};
pub use catalog_decoration::catalog_decoration_source;
pub use cataloged_models::CatalogedModelsProvider;
pub use core::ModelRuntime;
pub use credential::{ManagedCredential, parse_sap_ai_core_service_key, should_retry_credential};
pub use gemini::{GeminiAdapter, GeminiAdapterOptions};
pub use gitlab::GitlabProvider;
pub use gitlab::{
    default_ai_gateway_headers as default_gitlab_ai_gateway_headers,
    default_feature_flags as default_gitlab_feature_flags,
};
pub use multi_adapter::MultiAdapterProvider;
pub use multi_adapter::ProviderModelRoute;
pub use ollama::OllamaAdapter;
pub use openai::{
    OpenAiChatCompletionsAdapter, OpenAiChatCompletionsAdapterOptions, OpenAiRealtimeAdapter,
    OpenAiRealtimeAdapterOptions, OpenAiResponsesAdapter, OpenAiResponsesAdapterOptions,
};
pub use registry::ProviderRegistry;
pub use wire_message::{
    WirePart as ProjectedSessionPart, project_completion_input, project_operation_output,
    project_operation_output as project_session_tool_result_output, project_persisted,
    project_persisted as project_session_parts, project_persisted_text_lossy,
    project_persisted_text_lossy as project_session_text_lossy,
};
