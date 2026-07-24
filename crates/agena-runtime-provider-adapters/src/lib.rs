//! Vendor-specific provider implementations.

pub use agena_runtime_provider::{
    ProviderError, RUNTIME_CODEX_ORIGINATOR, claude_code_api_user_agent, codex_package_version,
    codex_user_agent, gemini_cli_user_agent, runtime_codex_user_agent,
};

pub mod config_support;
pub mod provider;

pub use provider::{
    AmazonBedrockAdapter, AnthropicAdapter, AnthropicAdapterOptions, GeminiAdapter,
    GeminiAdapterOptions, GitlabProvider, MultiAdapterProvider, OllamaAdapter,
    OpenAiChatCompletionsAdapter, OpenAiChatCompletionsAdapterOptions, OpenAiRealtimeAdapter,
    OpenAiRealtimeAdapterOptions, OpenAiResponsesAdapter, OpenAiResponsesAdapterOptions,
};
