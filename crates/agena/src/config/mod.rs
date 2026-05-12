mod error;
mod loader;
mod overrides;
mod provider_presets;
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
    AuthStoreBackend, BedrockAuthConfig, CloudflareAiGatewayProviderOptions, CodexProviderOptions,
    ConfigOutputFormat, ConfigResolution, ConfigResolutionMeta, ConfigSource,
    CopilotProviderOptions, GitlabProviderOptions, GoogleVertexAuthConfig,
    GoogleVertexProviderOptions, HttpProviderConfig, LspConfig, LspServerConfig, McpConfig,
    McpHttpAuthConfig, McpHttpMode, McpServerConfig, MemoryConfig, OllamaProviderOptions,
    OpenAiApiModeConfig, OpenAiCompatibleProviderOptions, OpenAiProviderOptions, PluginConfig,
    ProjectInstructionsConfig, ProviderDefinition, ProviderHttpConfig, RequestRetryConfig,
    ResolvedConfig, ResolvedProviderConfig, RuntimeConfig, RuntimeJanitorConfig,
    RuntimeReloadConfig, SessionCacheConfig, SimpleHttpProviderOptions, StreamReplayConfig,
    StreamTransportMode, TracingConfig, UiConfig, WebSearchBackend, WebSearchBackendKind,
    WebSearchConfig, WebToolsConfig,
};

pub(crate) use error::parse_numeric;
pub(crate) use raw::{
    RawAuthConfig, RawConfig, RawConfigFile, RawProviderHttpConfig, RawRequestRetryConfig,
    RawRuntimeConfig, RawStreamReplayConfig, RawTracingConfig, RawUiConfig,
};
