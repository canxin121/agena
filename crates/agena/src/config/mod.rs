mod adapter_models;
mod credential_store;
mod edit;
mod error;
mod loader;
mod overlay;
mod overrides;
mod raw;
mod registry;
mod types;

pub use error::ConfigError;
pub use loader::{ConfigEnvironment, ConfigLoader, LoadConfigRequest, ProcessEnvironment};
pub use overlay::{
    ProviderAdapterOverlay, ProviderAuthMode, ProviderAuthOverlay, ProviderDefaultsOverlay,
    ProviderModelOverlay, ProviderNativeToolsOverlay, ProviderOverlay,
    ProviderProtocolPathsOverlay, provider_model_overlay_from_catalog_definition,
    provider_model_overlay_from_definition,
};
pub use overrides::ConfigOverride;
pub use types::{
    AgentConfig, AmazonBedrockProviderOptions, AnthropicProviderOptions, AppliedLayer,
    BedrockSigv4AuthConfig, BrowserHarnessConfig, ConfigOutputFormat, ConfigResolution,
    ConfigResolutionMeta, ConfigSource, CrawlConfig, CrawlFetchEngine, EditorHarnessConfig,
    GeminiProviderOptions, GitlabProviderOptions, HarnessViewportConfig, HarnessesConfig,
    HostedCodeExecutionContainerConfig, HttpProviderAdapterConfig, LspConfig, LspServerConfig,
    McpConfig, McpHttpAuthConfig, McpServerConfig, MemoryConfig, MemoryRetrievalConfig,
    NativeToolFreshness, NativeToolHarnessKind, NativeToolUserLocationConfig,
    OllamaProviderOptions, OpenAiApiModeConfig, OpenAiBackendConfig, OpenAiProviderOptions,
    PluginConfig, ProjectInstructionsConfig, ProviderAdapterDefinition, ProviderApiAuthConfig,
    ProviderAuthConfig, ProviderCapabilityFamilyConfig, ProviderCredentialAuthConfig,
    ProviderDefaultsConfig, ProviderGitlabAuthConfig, ProviderHostedCodeExecutionConfig,
    ProviderHostedFileSearchConfig, ProviderHostedImageGenerationConfig, ProviderHostedToolConfigs,
    ProviderHostedUrlContextConfig, ProviderHostedWebSearchConfig, ProviderHttpConfig,
    ProviderModelDiscoveryConfig, ProviderNativeConnectorConfig, ProviderNativeHarnessBindings,
    ProviderNativeHarnessRef, ProviderNativeToolBinding, ProviderNativeToolKind,
    ProviderNativeToolRoute, ProviderNativeToolRoutesConfig, ProviderNativeToolsConfig,
    ProviderProtocolPathsConfig, RequestRetryConfig, ResolvedConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, ResolvedProviderModelConfig, RuntimeConfig, RuntimeGcConfig,
    RuntimeModelCatalogConfig, RuntimeProvidersConfig, RuntimeReloadConfig, RuntimeSessionConfig,
    SessionCacheConfig, SessionCompactionConfig, SessionConfig, ShellHarnessConfig,
    SimpleHttpProviderOptions, StreamReplayConfig, StreamTransportMode, TracingConfig, UiConfig,
    WebSearchBackend, WebSearchConfig, WebToolsConfig,
};

pub use adapter_models::{
    HTTP_ADAPTER_MODEL_LIST_ADAPTER_IDS, ProviderAdapterModelsListing, ProviderAdapterModelsTarget,
    draft_atomgit_provider_adapter_models_target, draft_gitlab_provider_adapter_models_target,
    draft_provider_adapter_models_target, list_provider_adapter_models_with_config,
    saved_provider_adapter_models_target,
};
pub use credential_store::{
    ProviderAuthTargetError, ProviderConfigCredentialStore, ProviderDeviceAuthTarget,
    ProviderOAuthTarget, provider_auth_data, provider_gitlab_instance_url,
    provider_has_gitlab_adapter, provider_supports_api_key_write, provider_supports_atomgit_oauth,
    provider_supports_copilot_device, provider_supports_openai_oauth,
    resolve_provider_device_auth_target, resolve_provider_oauth_target,
};
pub use edit::{
    ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsEditResponse,
    ConfigSettingsGetInput, ConfigSettingsListEntry, ConfigSettingsListInput,
    ConfigSettingsListResponse, ConfigSettingsPatchInput, ConfigSettingsPathInput,
    ConfigSettingsReadResponse, ConfigSettingsReloadResponse, ConfigSettingsSetInput,
    ConfigSettingsSource, ConfigSettingsValidateInput, ConfigSettingsValidateResponse,
    delete_file_setting, delete_file_setting_with_env, format_settings_path, get_json_path,
    list_file_settings, list_json_path, parse_settings_path, patch_file_settings,
    patch_file_settings_with_env, read_file_setting, set_file_setting, set_file_setting_with_env,
    validate_file_settings, validate_file_settings_with_env,
};
pub(crate) use error::parse_numeric;
pub(crate) use raw::{
    RawConfig, RawConfigFile, RawProviderHttpConfig, RawRequestRetryConfig, RawRuntimeConfig,
    RawRuntimeGcConfig, RawRuntimeModelCatalogConfig, RawRuntimeProvidersConfig,
    RawRuntimeSessionConfig, RawSessionCacheConfig, RawStreamReplayConfig, RawTracingConfig,
    RawUiConfig,
};
pub use registry::ProviderAdapterModelsResult;
pub use registry::list_provider_adapter_models;
