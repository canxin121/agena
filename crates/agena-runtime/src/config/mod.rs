mod adapter_models;
mod credential_store;
mod edit;
mod loader;
mod overrides;
mod raw;
mod registry;

pub(crate) use registry::build_provider_registry_from_inputs;

pub(crate) use agena_runtime::ConfigError;
pub(crate) use agena_runtime::LoadConfigRequest;
pub(crate) use agena_runtime::{
    AgentConfig, AmazonBedrockProviderOptions, AnthropicProviderOptions, ConfigResolution,
    GeminiProviderOptions, HarnessViewportConfig, HarnessesConfig, HttpProviderAdapterConfig,
    OllamaProviderOptions, OpenAiChatCompletionsProviderOptions, OpenAiRealtimeProviderOptions,
    OpenAiResponsesProviderOptions, ProviderAdapterDefinition, ProviderApiAuthConfig,
    ProviderAuthConfig, ProviderClientVersionSettings, ProviderDefaultsConfig,
    ProviderGitlabAuthConfig, ResolvedConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, RuntimeConfig, RuntimeProvidersConfig, SessionCompactionConfig,
    SessionConfig, TuiColorSchemeConfig, TuiGraphicsModeConfig, TuiUiConfig, UiConfig,
};
pub(crate) use loader::{ConfigEnvironment, ConfigLoader, ProcessEnvironment};
pub(crate) use overrides::apply_config_override;

pub(crate) use adapter_models::{
    ProviderAdapterModelsTarget, draft_bedrock_sigv4_provider_adapter_models_target,
    draft_cline_api_provider_adapter_models_target,
    draft_credential_provider_adapter_models_target, draft_gitlab_provider_adapter_models_target,
    draft_none_provider_adapter_models_target, draft_provider_adapter_models_target,
    list_provider_adapter_models_with_providers, saved_provider_adapter_models_target,
};
pub(crate) use credential_store::{
    ProviderAuthTargetError, ProviderConfigCredentialStore, ProviderDeviceAuthTarget,
    ProviderOAuthTarget, provider_auth_data, provider_gitlab_instance_url,
    provider_supports_api_key_write, resolve_provider_device_auth_target,
    resolve_provider_oauth_target,
};
pub(crate) use edit::{
    ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsGetInput,
    ConfigSettingsLayer, ConfigSettingsListInput, ConfigSettingsListResponse,
    ConfigSettingsPatchInput, ConfigSettingsPathInput, ConfigSettingsReadResponse,
    ConfigSettingsSetInput, ConfigSettingsSource, ConfigSettingsValidateResponse,
    delete_layered_file_setting, list_file_settings, list_json_path, parse_settings_path,
    patch_layered_file_settings, read_file_setting, set_layered_file_setting,
    validate_layered_file_settings,
};
pub(crate) use raw::{
    RawConfig, RawConfigFile, RawTracingConfig, RawTuiUiConfig, RawUiConfig, validate_config_text,
};
pub(crate) use registry::ProviderAdapterModelsResult;
pub(crate) use registry::list_provider_adapter_models;
