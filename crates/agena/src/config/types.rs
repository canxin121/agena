use std::{collections::BTreeMap, fmt, path::PathBuf, str::FromStr, time::Duration};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::{
    permission::{PermissionMode, PermissionPolicy},
    provider::{
        OpenAiApiMode, OpenAiCompatibleStreamMode, OpenAiStreamMode, ProviderCapabilityOverrideRule,
        ProviderHttpClientConfig, ProviderRequestRetryConfig, ProviderRuntimeConfig,
        ProviderStreamReplayConfig, ThinkingRequest, auth::FileAuthStore,
    },
};

use super::ConfigError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Default,
    File,
    Mode,
    Environment,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppliedLayer {
    pub source: ConfigSource,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigResolutionMeta {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub active_mode: Option<ConfigModeName>,
    pub active_mode_source: Option<ConfigSource>,
    pub applied_layers: Vec<AppliedLayer>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigResolution {
    pub config: ResolvedConfig,
    pub meta: ConfigResolutionMeta,
}

impl ConfigResolution {
    pub fn render(&self, format: ConfigOutputFormat) -> Result<String, ConfigError> {
        match format {
            ConfigOutputFormat::Json => Ok(serde_json::to_string_pretty(self)?),
            ConfigOutputFormat::Toml => Ok(toml::to_string_pretty(self)?),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ConfigModeName(String);

impl ConfigModeName {
    pub fn new(value: impl Into<String>) -> Result<Self, ConfigError> {
        value.into().try_into()
    }
}

impl TryFrom<String> for ConfigModeName {
    type Error = ConfigError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(ConfigError::InvalidModeName);
        }
        Ok(Self(trimmed.to_owned()))
    }
}

impl AsRef<str> for ConfigModeName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for ConfigModeName {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl fmt::Display for ConfigModeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedConfig {
    pub tracing: TracingConfig,
    pub auth: AuthConfig,
    pub ui: UiConfig,
    pub runtime: RuntimeConfig,
    pub permission: PermissionConfig,
    pub plugins: PluginConfig,
    pub providers: BTreeMap<String, ResolvedProviderConfig>,
}

impl ResolvedConfig {
    pub fn provider_http_client_config(&self) -> ProviderHttpClientConfig {
        ProviderHttpClientConfig {
            timeout: Duration::from_secs(self.runtime.provider_http.timeout_secs),
            connect_timeout: Duration::from_secs(self.runtime.provider_http.connect_timeout_secs),
        }
    }

    pub fn provider_runtime_config(&self) -> ProviderRuntimeConfig {
        ProviderRuntimeConfig {
            request_retry: ProviderRequestRetryConfig {
                max_retries: self.runtime.request_retry.max_retries,
                base_delay: Duration::from_millis(self.runtime.request_retry.base_delay_ms),
                max_delay: Duration::from_millis(self.runtime.request_retry.max_delay_ms),
            },
            stream_replay: ProviderStreamReplayConfig {
                max_retries_after_output: self.runtime.stream_replay.max_retries_after_output,
                max_tracked_events: self.runtime.stream_replay.max_tracked_events,
            },
        }
    }

    pub fn permission_policy(&self) -> PermissionPolicy {
        PermissionPolicy::new(self.permission.default_read, self.permission.default_write)
            .with_external_directory_default(self.permission.default_external_directory)
    }

    pub fn auth_store(&self) -> FileAuthStore {
        FileAuthStore::new(self.auth.store_path.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TracingConfig {
    pub filter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthConfig {
    pub store_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UiConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeConfig {
    pub provider_http: ProviderHttpConfig,
    pub request_retry: RequestRetryConfig,
    pub stream_replay: StreamReplayConfig,
    pub reload: RuntimeReloadConfig,
    pub janitor: RuntimeJanitorConfig,
    pub session_cache: SessionCacheConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderHttpConfig {
    pub timeout_secs: u64,
    pub connect_timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestRetryConfig {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamReplayConfig {
    pub max_retries_after_output: u32,
    pub max_tracked_events: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeReloadConfig {
    pub enabled: bool,
    pub poll_interval_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeJanitorConfig {
    pub enabled: bool,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionCacheConfig {
    pub max_sessions: usize,
    pub ttl_secs: u64,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermissionConfig {
    pub default_read: PermissionMode,
    pub default_write: PermissionMode,
    pub default_external_directory: PermissionMode,
}

pub use agena_plugin_host::PluginsConfig as PluginConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProviderConfig {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_overrides: Vec<ProviderCapabilityOverrideRule>,
    #[serde(flatten)]
    pub definition: ProviderDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderDefinition {
    Alias(ProviderAliasConfig),
    OpenAi(HttpProviderConfig<OpenAiProviderOptions>),
    OpenAiCompatible(HttpProviderConfig<OpenAiCompatibleProviderOptions>),
    SapAiCore(HttpProviderConfig<OpenAiCompatibleProviderOptions>),
    Anthropic(HttpProviderConfig<AnthropicProviderOptions>),
    Gemini(HttpProviderConfig<SimpleHttpProviderOptions>),
    Codex(CodexProviderOptions),
    Gitlab(GitlabProviderOptions),
    Copilot(CopilotProviderOptions),
    AmazonBedrock(AmazonBedrockProviderOptions),
    GoogleVertex(GoogleVertexProviderOptions),
    CloudflareAiGateway(CloudflareAiGatewayProviderOptions),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderAliasConfig {
    pub target_provider_id: String,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HttpProviderConfig<T> {
    pub base_url: String,
    pub default_model: String,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
    pub default_thinking: Option<ThinkingRequest>,
    pub options: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiProviderOptions {
    pub api_mode: OpenAiApiModeConfig,
    pub stream_mode: StreamTransportMode,
    pub realtime_ws_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiCompatibleProviderOptions {
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    pub stream_mode: StreamTransportMode,
    pub realtime_ws_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnthropicProviderOptions {
    pub auth_header: String,
    pub auth_scheme: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SimpleHttpProviderOptions;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodexProviderOptions {
    pub default_model: String,
    pub auth_provider_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitlabProviderOptions {
    pub instance_url: String,
    pub ai_gateway_url: String,
    pub default_model: String,
    pub auth_provider_id: String,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub ai_gateway_headers: BTreeMap<String, String>,
    pub feature_flags: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CopilotProviderOptions {
    pub default_model: String,
    pub base_url: String,
    pub models_url: Option<String>,
    pub auth_provider_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AmazonBedrockProviderOptions {
    pub base_url: String,
    pub default_model: String,
    pub region: String,
    pub auth: BedrockAuthConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum BedrockAuthConfig {
    Bearer {
        api_key: Option<String>,
        api_key_env: Option<String>,
    },
    Sigv4 {
        profile: Option<String>,
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        session_token: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GoogleVertexProviderOptions {
    pub base_url: String,
    pub default_model: String,
    pub auth: GoogleVertexAuthConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum GoogleVertexAuthConfig {
    StaticToken {
        access_token: Option<String>,
        access_token_env: Option<String>,
    },
    Adc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CloudflareAiGatewayProviderOptions {
    pub base_url: String,
    pub default_model: String,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ConfigOutputFormat {
    Json,
    Toml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamTransportMode {
    Sse,
    RealtimeWebSocket,
}

impl FromStr for StreamTransportMode {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "sse" => Ok(Self::Sse),
            "realtime_websocket" => Ok(Self::RealtimeWebSocket),
            _ => Err(ConfigError::InvalidOverride(format!(
                "unknown stream mode `{value}`"
            ))),
        }
    }
}

impl From<StreamTransportMode> for OpenAiStreamMode {
    fn from(value: StreamTransportMode) -> Self {
        match value {
            StreamTransportMode::Sse => Self::Sse,
            StreamTransportMode::RealtimeWebSocket => Self::RealtimeWebSocket,
        }
    }
}

impl From<StreamTransportMode> for OpenAiCompatibleStreamMode {
    fn from(value: StreamTransportMode) -> Self {
        match value {
            StreamTransportMode::Sse => Self::Sse,
            StreamTransportMode::RealtimeWebSocket => Self::RealtimeWebSocket,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiApiModeConfig {
    Responses,
    Chat,
    Auto,
}

impl FromStr for OpenAiApiModeConfig {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "responses" => Ok(Self::Responses),
            "chat" => Ok(Self::Chat),
            "auto" => Ok(Self::Auto),
            _ => Err(ConfigError::InvalidOverride(format!(
                "unknown openai api mode `{value}`"
            ))),
        }
    }
}

impl From<OpenAiApiModeConfig> for OpenAiApiMode {
    fn from(value: OpenAiApiModeConfig) -> Self {
        match value {
            OpenAiApiModeConfig::Responses => Self::Responses,
            OpenAiApiModeConfig::Chat => Self::Chat,
            OpenAiApiModeConfig::Auto => Self::Auto,
        }
    }
}
