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
    ProviderAdapterOverlay, ProviderApiSubtype, ProviderAuthMode, ProviderAuthOverlay,
    ProviderDefaultsOverlay, ProviderGitlabApiAccessOverlay, ProviderModelOverlay, ProviderOverlay,
    ProviderProtocolPathsOverlay, ProviderSecretSourceOverlay, ProviderToolsOverlay,
    provider_model_overlay_from_catalog_definition, provider_model_overlay_from_definition,
};
pub use overrides::ConfigOverride;
pub use types::{
    AgenaToolTransport, AgenaToolsConfig, AgentConfig, AmazonBedrockProviderOptions,
    AnthropicProviderOptions, AppliedLayer, BedrockSigv4AuthConfig, BrowserHarnessConfig,
    CLINE_API_BASE_URL, CLINE_API_OPENAI_PROTOCOL_PATH, ConfigOutputFormat, ConfigResolution,
    ConfigResolutionMeta, ConfigSource, EditorHarnessConfig, GeminiProviderOptions,
    GitlabProviderOptions, HarnessViewportConfig, HarnessesConfig,
    HostedCodeExecutionContainerConfig, HttpProviderAdapterConfig, OllamaProviderOptions,
    OpenAiChatCompletionsProviderOptions, OpenAiRealtimeProviderOptions,
    OpenAiResponsesBackendConfig, OpenAiResponsesProviderOptions, PluginConfig,
    ProviderAdapterDefinition, ProviderApiAuthConfig, ProviderAuthConfig,
    ProviderCapabilityFamilyConfig, ProviderClientVersionSettings, ProviderCredentialAuthConfig,
    ProviderDefaultsConfig, ProviderGitlabApiAccessConfig, ProviderGitlabAuthConfig,
    ProviderGitlabCredentialAuthConfig, ProviderHostedCodeExecutionConfig,
    ProviderHostedFileSearchConfig, ProviderHostedImageGenerationConfig, ProviderHostedToolConfigs,
    ProviderHostedUrlContextConfig, ProviderHostedWebSearchConfig, ProviderHttpConfig,
    ProviderHttpCredentialAuthConfig, ProviderInlineCredentialAuthConfig,
    ProviderModelDiscoveryConfig, ProviderProtocolPathsConfig,
    ProviderSapAiCoreCredentialAuthConfig, ProviderSecretSourceConfig, ProviderToolBinding,
    ProviderToolConnectorConfig, ProviderToolFreshness, ProviderToolHarnessBindings,
    ProviderToolHarnessKind, ProviderToolHarnessRef, ProviderToolKind, ProviderToolRoute,
    ProviderToolRoutesConfig, ProviderToolUserLocationConfig, ProviderToolsConfig,
    RequestRetryConfig, ResolvedConfig, ResolvedProviderAdapterConfig, ResolvedProviderConfig,
    ResolvedProviderModelConfig, RuntimeConfig, RuntimeGcConfig, RuntimeModelCatalogConfig,
    RuntimeProvidersConfig, RuntimeReloadConfig, RuntimeSessionConfig, SessionCacheConfig,
    SessionCompactionConfig, SessionConfig, ShellHarnessConfig, SimpleHttpProviderOptions,
    StreamReplayConfig, StreamTransportMode, TracingConfig, TuiColorSchemeConfig,
    TuiGraphicsModeConfig, TuiUiConfig, UiConfig, cline_api_protocol_paths,
};

pub use adapter_models::{
    HTTP_ADAPTER_MODEL_LIST_ADAPTER_IDS, ProviderAdapterModelsListing, ProviderAdapterModelsTarget,
    draft_bedrock_sigv4_provider_adapter_models_target,
    draft_cline_api_provider_adapter_models_target,
    draft_credential_provider_adapter_models_target, draft_gitlab_provider_adapter_models_target,
    draft_none_provider_adapter_models_target, draft_provider_adapter_models_target,
    list_provider_adapter_models_with_config, saved_provider_adapter_models_target,
};
pub use credential_store::{
    ProviderAuthTargetError, ProviderConfigCredentialStore, ProviderDeviceAuthTarget,
    ProviderOAuthTarget, provider_auth_data, provider_gitlab_instance_url,
    provider_has_gitlab_adapter, provider_supports_api_key_write, provider_supports_copilot_device,
    provider_supports_openai_oauth, resolve_provider_device_auth_target,
    resolve_provider_oauth_target,
};
pub use edit::{
    ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsEditResponse,
    ConfigSettingsGetInput, ConfigSettingsListInput, ConfigSettingsListItem,
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
    RawTuiUiConfig, RawUiConfig,
};
pub use registry::ProviderAdapterModelsResult;
pub use registry::list_provider_adapter_models;
