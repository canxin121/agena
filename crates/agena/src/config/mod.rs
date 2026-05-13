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
    AgentConfig, AmazonBedrockProviderOptions, AnthropicProviderOptions, AppliedLayer, AuthConfig,
    AuthStoreBackend, BedrockSigv4AuthConfig, CloudflareAiGatewayProviderOptions,
    ConfigOutputFormat, ConfigResolution, ConfigResolutionMeta, ConfigSource,
    CopilotProviderOptions, GitlabProviderOptions, GoogleVertexProviderOptions,
    HttpProviderAdapterConfig, LspConfig, LspServerConfig, McpConfig, McpHttpAuthConfig,
    McpHttpMode, McpServerConfig, MemoryConfig, OllamaProviderOptions, OpenAiApiModeConfig,
    OpenAiBackendConfig, OpenAiCompatibleProviderOptions, OpenAiProviderOptions, PluginConfig,
    ProjectInstructionsConfig, ProviderAuthConfig, ProviderHttpConfig, ProviderSapAiCoreAuthConfig,
    ProviderSecretAuthConfig, ProviderAdapterDefinition, RequestRetryConfig, ResolvedConfig,
    ResolvedProviderAdapterConfig, ResolvedProviderConfig, ResolvedProviderModelConfig,
    RuntimeConfig, RuntimeJanitorConfig, RuntimeReloadConfig, SessionCacheConfig,
    SimpleHttpProviderOptions, StreamReplayConfig, StreamTransportMode, TracingConfig, UiConfig,
    WebSearchBackend, WebSearchBackendKind, WebSearchConfig, WebToolsConfig,
};

pub(crate) use error::parse_numeric;
pub(crate) use raw::{
    RawAuthConfig, RawConfig, RawConfigFile, RawProviderHttpConfig, RawRequestRetryConfig,
    RawRuntimeConfig, RawStreamReplayConfig, RawTracingConfig, RawUiConfig,
};
