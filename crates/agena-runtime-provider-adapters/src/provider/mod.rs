//! Concrete provider adapter implementations (Anthropic, OpenAI, Gemini, ...).

pub use agena_domain::{Model, ModelId, ModelSpeedMode, ModelThinkingMode};
pub use agena_runtime_provider::provider::{
    CatalogedModelsProvider, MultiAdapterProvider, ProjectedSessionPart,
    parse_sap_ai_core_service_key, project_completion_input,
};
pub use agena_runtime_provider::provider::{
    CompletionResponse, ManagedCredential, should_retry_credential,
};
pub use agena_runtime_provider::provider::{
    ModelRuntime, ProviderModelRoute, ProviderRegistry, ProviderRequestHeaderHook,
    catalog_decoration_source, install_request_header_hook, with_request_cancellation,
};

pub(crate) use agena_provider::{
    self as copilot_models, self as prompt_cache, self as protocol_ids, self as tool_stream,
};
pub use agena_runtime_provider::provider::chat_wire::ChatFunctionCallWire;
pub(crate) use agena_runtime_provider::provider::{chat_wire, core, utils, wire_message};
pub(crate) use agena_runtime_provider::provider_sse as sse;

pub mod amazon_bedrock;
pub mod anthropic;
pub mod gemini;
pub mod gitlab;
pub mod ollama;
pub mod openai;

pub use amazon_bedrock::AmazonBedrockAdapter;
pub use anthropic::{AnthropicAdapter, AnthropicAdapterOptions};
pub use gemini::{GeminiAdapter, GeminiAdapterOptions};
pub use gitlab::GitlabProvider;
pub use gitlab::{
    default_ai_gateway_headers as default_gitlab_ai_gateway_headers,
    default_feature_flags as default_gitlab_feature_flags,
};
pub use ollama::OllamaAdapter;
pub use openai::{
    OpenAiChatCompletionsAdapter, OpenAiChatCompletionsAdapterOptions, OpenAiRealtimeAdapter,
    OpenAiRealtimeAdapterOptions, OpenAiResponsesAdapter, OpenAiResponsesAdapterOptions,
};
