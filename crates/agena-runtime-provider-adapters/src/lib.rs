//! Vendor-specific provider implementations.

pub use agena_runtime_provider::{
    ProviderClientIdentity, ProviderError, RUNTIME_CODEX_ORIGINATOR, runtime_codex_user_agent,
};

pub mod config_support;
pub mod provider;

pub use provider::{
    AmazonBedrockAdapter, AnthropicAdapter, AnthropicAdapterOptions, GeminiAdapter,
    GeminiAdapterOptions, GitlabProvider, MultiAdapterProvider, OllamaAdapter,
    OpenAiChatCompletionsAdapter, OpenAiChatCompletionsAdapterOptions, OpenAiRealtimeAdapter,
    OpenAiRealtimeAdapterOptions, OpenAiResponsesAdapter, OpenAiResponsesAdapterOptions,
};
