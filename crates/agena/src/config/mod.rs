mod error;
mod loader;
mod overrides;
mod provider_presets;
mod raw;
mod registry;
mod types;

#[cfg(test)]
mod tests;

pub use error::ConfigError;
pub use loader::{ConfigEnvironment, ConfigLoader, LoadConfigRequest, ProcessEnvironment};
pub use overrides::ConfigOverride;
pub use types::{
    AmazonBedrockProviderOptions, AnthropicProviderOptions, AppliedLayer, AuthConfig,
    BedrockAuthConfig, CloudflareAiGatewayProviderOptions, CodexProviderOptions, ConfigModeName,
    ConfigOutputFormat, ConfigResolution, ConfigResolutionMeta, ConfigSource,
    CopilotProviderOptions, GitlabProviderOptions, GoogleVertexAuthConfig,
    GoogleVertexProviderOptions, HttpProviderConfig, McpConfig, McpHttpMode, McpServerConfig,
    OpenAiApiModeConfig,
    OpenAiCompatibleProviderOptions, OpenAiProviderOptions, PermissionConfig, PluginConfig,
    ProviderAliasConfig, ProviderDefinition, ProviderHttpConfig, RequestRetryConfig,
    ResolvedConfig, ResolvedProviderConfig, RuntimeConfig, RuntimeJanitorConfig,
    RuntimeReloadConfig, SessionCacheConfig, SimpleHttpProviderOptions, StreamReplayConfig,
    StreamTransportMode, TracingConfig, UiConfig,
};

pub(crate) use error::{parse_numeric, parse_permission_mode};
pub(crate) use raw::{
    RawAuthConfig, RawConfig, RawConfigFile, RawPermissionConfig, RawProviderHttpConfig,
    RawRequestRetryConfig, RawRuntimeConfig, RawStreamReplayConfig, RawTracingConfig, RawUiConfig,
};
