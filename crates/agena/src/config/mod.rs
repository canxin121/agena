mod adapter_models;
mod credential_store;
mod edit;
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
    ConfigSource, DefaultConfig, GitlabProviderOptions, HttpProviderAdapterConfig, LspConfig,
    LspServerConfig, McpConfig, McpHttpAuthConfig, McpHttpMode, McpServerConfig, MemoryConfig,
    OllamaProviderOptions, OpenAiApiModeConfig, OpenAiBackendConfig, OpenAiProviderOptions,
    PluginConfig, ProjectInstructionsConfig, ProviderAdapterDefinition, ProviderApiAuthConfig,
    ProviderAuthConfig, ProviderCapabilityFamilyConfig, ProviderCredentialAuthConfig,
    ProviderGoogleAdcAuthConfig, ProviderHttpConfig, ProviderModelDiscoveryConfig,
    ProviderSapAiCoreAuthConfig, RequestRetryConfig, ResolvedConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, ResolvedProviderModelConfig, RuntimeConfig, RuntimeJanitorConfig,
    RuntimeModelCatalogConfig, RuntimeReloadConfig, SessionCacheConfig,
    SharedGatewayEndpointLayout, SimpleHttpProviderOptions, StreamReplayConfig,
    StreamTransportMode, TracingConfig, UiConfig, WebSearchBackend, WebSearchBackendKind,
    WebSearchConfig, WebToolsConfig,
};

pub use adapter_models::{
    HTTP_ADAPTER_MODEL_LIST_ADAPTER_IDS, ProviderAdapterModelsTarget,
    draft_provider_adapter_models_target, list_provider_adapter_models_for_target,
    saved_provider_adapter_models_target,
};
pub use credential_store::{
    ProviderConfigCredentialStore, provider_auth_data, provider_gitlab_instance_url,
    provider_has_gitlab_adapter, provider_supports_api_key_write, provider_supports_atomgit_oauth,
    provider_supports_copilot_device, provider_supports_openai_oauth,
};
pub use edit::{
    ConfigSettingsDeleteInput, ConfigSettingsEditResponse, ConfigSettingsGetInput,
    ConfigSettingsListEntry, ConfigSettingsListInput, ConfigSettingsListResponse,
    ConfigSettingsPatchInput, ConfigSettingsReadResponse, ConfigSettingsReloadResponse,
    ConfigSettingsSetInput, ConfigSettingsSource, ConfigSettingsValidateInput,
    ConfigSettingsValidateResponse, delete_file_setting, delete_file_setting_with_env,
    format_settings_path, get_json_path, list_file_settings, list_json_path, parse_settings_path,
    patch_file_settings, patch_file_settings_with_env, read_file_setting, set_file_setting,
    set_file_setting_with_env, validate_file_settings, validate_file_settings_with_env,
};
pub(crate) use error::parse_numeric;
pub(crate) use raw::{
    RawConfig, RawConfigFile, RawDefaultConfig, RawProviderHttpConfig, RawRequestRetryConfig,
    RawRuntimeConfig, RawRuntimeModelCatalogConfig, RawStreamReplayConfig, RawTracingConfig,
    RawUiConfig,
};
pub use registry::ProviderAdapterModelsResult;
pub use registry::list_provider_adapter_models;
