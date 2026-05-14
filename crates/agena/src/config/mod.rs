mod credential_store;
mod error;
mod loader;
mod overrides;
mod raw;
mod registry;
mod types;

#[cfg(test)]
mod tests;

pub use agena_otel::TelemetryConfig;
pub use error::ConfigError;
pub use loader::{ConfigEnvironment, ConfigLoader, LoadConfigRequest, ProcessEnvironment};
pub use overrides::ConfigOverride;
pub use types::{
    AgentConfig, AmazonBedrockProviderOptions, AnthropicProviderOptions, AppliedLayer,
    BedrockSigv4AuthConfig, ConfigOutputFormat, ConfigResolution, ConfigResolutionMeta,
    ConfigSource, GitlabProviderOptions, HttpProviderAdapterConfig, LspConfig,
    LspServerConfig, McpConfig, McpHttpAuthConfig, McpHttpMode, McpServerConfig, MemoryConfig,
    OllamaProviderOptions, OpenAiApiModeConfig, OpenAiBackendConfig,
    OpenAiProviderOptions, PluginConfig,
    ProjectInstructionsConfig, ProviderAdapterDefinition, ProviderApiAuthConfig,
    ProviderAuthConfig, ProviderCapabilityFamilyConfig, ProviderCredentialAuthConfig,
    ProviderHttpConfig, ProviderSapAiCoreAuthConfig, RequestRetryConfig, ResolvedConfig,
    ResolvedProviderAdapterConfig, ResolvedProviderConfig, ResolvedProviderModelConfig,
    RuntimeConfig, RuntimeJanitorConfig, RuntimeReloadConfig, SessionCacheConfig,
    SimpleHttpProviderOptions, StreamReplayConfig, StreamTransportMode, TracingConfig, UiConfig,
    WebSearchBackend, WebSearchBackendKind, WebSearchConfig, WebToolsConfig,
};

pub use credential_store::{
    ProviderConfigCredentialStore, provider_auth_data, provider_gitlab_instance_url,
    provider_has_gitlab_adapter, provider_supports_api_key_write, provider_supports_copilot_device,
    provider_supports_openai_oauth,
};
pub(crate) use error::parse_numeric;
pub(crate) use raw::{
    RawConfig, RawConfigFile, RawProviderHttpConfig, RawRequestRetryConfig, RawRuntimeConfig,
    RawStreamReplayConfig, RawTracingConfig, RawUiConfig,
};
