use std::{collections::BTreeMap, fmt, path::PathBuf, str::FromStr, time::Duration};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

use crate::execution_prefs::ExecutionSelection;
use crate::provider::{
    CapabilityFamily, ConfiguredModelDefinition, GeminiStreamMode, OpenAiApiMode, OpenAiBackend,
    OpenAiStreamMode, ProviderHttpClientConfig, ProviderRequestRetryConfig, ProviderRuntimeConfig,
    ProviderStreamReplayConfig,
    auth::{AuthData, CredentialIssuer},
};

use super::ConfigError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Default,
    File,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedConfig {
    pub default: DefaultConfig,
    pub tracing: TracingConfig,
    pub telemetry: agena_otel::TelemetryConfig,
    pub ui: UiConfig,
    pub runtime: RuntimeConfig,
    pub session: SessionConfig,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::PermissionConfig::is_empty"
    )]
    pub permission: crate::agent::PermissionConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, AgentConfig>,
    pub plugins: PluginConfig,
    pub plugin_storage: PluginStorageConfig,
    #[serde(skip_serializing)]
    pub memory: MemoryConfig,
    #[serde(skip_serializing)]
    pub mcp: McpConfig,
    #[serde(skip_serializing)]
    pub lsp: LspConfig,
    #[serde(skip_serializing)]
    pub web: WebToolsConfig,
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

    pub fn plugin_storage(&self) -> std::sync::Arc<dyn crate::plugins::storage::PluginStorage> {
        std::sync::Arc::new(crate::plugins::storage::FilePluginStorage::new(
            self.plugin_storage.root_path.clone(),
        ))
    }

    pub fn plugin_secret_store(
        &self,
    ) -> std::sync::Arc<dyn crate::plugins::storage::PluginSecretStore> {
        let root = self.plugin_storage.root_path.clone();
        match self.plugin_storage.secrets_backend {
            PluginSecretsBackend::Auto => {
                std::sync::Arc::new(crate::plugins::storage::PluginKeyringSecretStore::system(
                    root,
                    self.plugin_storage.fallback_to_file,
                ))
            }
            PluginSecretsBackend::Keyring => std::sync::Arc::new(
                crate::plugins::storage::PluginKeyringSecretStore::system(root, false),
            ),
            PluginSecretsBackend::File => std::sync::Arc::new(
                crate::plugins::storage::PluginKeyringSecretStore::system(root, true),
            ),
        }
    }
}

pub type DefaultConfig = ExecutionSelection;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TracingConfig {
    pub filter: String,
    pub database: String,
    pub adapter: String,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            filter: "info".to_string(),
            database: "error".to_string(),
            adapter: "off".to_string(),
        }
    }
}

impl TracingConfig {
    pub fn env_filter(&self) -> Result<EnvFilter, tracing_subscriber::filter::ParseError> {
        let mut filter = EnvFilter::try_new(self.filter.as_str())?;
        for target in ["sqlx", "sea_orm", "sea_orm_migration"] {
            filter = filter.add_directive(format!("{target}={}", self.database).parse()?);
        }
        filter = filter.add_directive(format!("agena::adapter={}", self.adapter).parse()?);
        Ok(filter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginStorageConfig {
    pub root_path: PathBuf,
    pub secrets_backend: PluginSecretsBackend,
    pub fallback_to_file: bool,
}

impl Default for PluginStorageConfig {
    fn default() -> Self {
        Self {
            root_path: crate::plugins::storage::default_storage_root(),
            secrets_backend: PluginSecretsBackend::Auto,
            fallback_to_file: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginSecretsBackend {
    #[default]
    Auto,
    Keyring,
    File,
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
    pub model_catalog: RuntimeModelCatalogConfig,
    pub reload: RuntimeReloadConfig,
    pub janitor: RuntimeJanitorConfig,
    pub session_cache: SessionCacheConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SessionConfig {
    pub compaction: SessionCompactionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionCompactionConfig {
    pub auto: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserved_tokens: Option<u32>,
}

impl Default for SessionCompactionConfig {
    fn default() -> Self {
        Self {
            auto: true,
            reserved_tokens: None,
        }
    }
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
pub struct RuntimeModelCatalogConfig {
    pub cache_max_age_secs: u64,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::PermissionConfig::is_empty"
    )]
    pub permission: crate::agent::PermissionConfig,
    #[serde(
        default,
        rename = "default",
        skip_serializing_if = "crate::agents::AgentDefaultModelConfig::is_empty"
    )]
    pub default: crate::agents::AgentDefaultModelConfig,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

pub use agena_plugin_host::PluginsConfig as PluginConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
#[derive(Default)]
pub struct MemoryConfig {
    pub project_instructions: ProjectInstructionsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectInstructionsConfig {
    pub enabled: bool,
    pub include_global: bool,
}

impl Default for ProjectInstructionsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            include_global: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProviderConfig {
    pub enabled: bool,
    pub default_adapter: String,
    pub default_model: String,
    pub auth: ProviderAuthConfig,
    pub adapters: BTreeMap<String, ResolvedProviderAdapterConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, ResolvedProviderModelConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ProviderAuthConfig {
    None,
    Api(ProviderApiAuthConfig),
    #[serde(rename = "gitlab_api")]
    Gitlab(ProviderGitlabAuthConfig),
    Credential(ProviderCredentialAuthConfig),
    BedrockSigv4(BedrockSigv4AuthConfig),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderProtocolPathsConfig {
    pub openai: String,
    pub anthropic: String,
    pub gemini: String,
}

impl Default for ProviderProtocolPathsConfig {
    fn default() -> Self {
        Self {
            openai: "/v1".to_owned(),
            anthropic: "/v1".to_owned(),
            gemini: "/v1beta".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderModelDiscoveryConfig {
    #[default]
    Live,
    ConfiguredOnly,
}

#[derive(Clone, PartialEq, Eq, Serialize, Default)]
pub struct ProviderApiAuthConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub protocol_paths: ProviderProtocolPathsConfig,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
}

impl fmt::Debug for ProviderApiAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderApiAuthConfig")
            .field("base_url", &self.base_url)
            .field("protocol_paths", &self.protocol_paths)
            .field("api_key", &redacted(self.api_key.as_deref()))
            .field("api_key_env", &self.api_key_env)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Default)]
pub struct ProviderGitlabAuthConfig {
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<AuthData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_gateway_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai_gateway_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub feature_flags: BTreeMap<String, bool>,
}

impl fmt::Debug for ProviderGitlabAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderGitlabAuthConfig")
            .field("api_key", &redacted(self.api_key.as_deref()))
            .field("api_key_env", &self.api_key_env)
            .field(
                "credential",
                &self
                    .credential
                    .as_ref()
                    .map(|credential| credential_debug_kind(credential)),
            )
            .field("instance_url", &self.instance_url)
            .field("ai_gateway_url", &self.ai_gateway_url)
            .field("ai_gateway_headers", &self.ai_gateway_headers)
            .field("feature_flags", &self.feature_flags)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct ProviderCredentialAuthConfig {
    pub issuer: CredentialIssuer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<AuthData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub protocol_paths: ProviderProtocolPathsConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_key_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_gateway_url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ai_gateway_headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub feature_flags: BTreeMap<String, bool>,
}

impl fmt::Debug for ProviderCredentialAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderCredentialAuthConfig")
            .field("issuer", &self.issuer)
            .field(
                "credential",
                &self
                    .credential
                    .as_ref()
                    .map(|credential| credential_debug_kind(credential)),
            )
            .field("base_url", &self.base_url)
            .field("protocol_paths", &self.protocol_paths)
            .field("service_key_env", &self.service_key_env)
            .field("instance_url", &self.instance_url)
            .field("ai_gateway_url", &self.ai_gateway_url)
            .field("ai_gateway_headers", &self.ai_gateway_headers)
            .field("feature_flags", &self.feature_flags)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct BedrockSigv4AuthConfig {
    pub base_url: String,
    pub region: String,
    pub profile: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
}

impl fmt::Debug for BedrockSigv4AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BedrockSigv4AuthConfig")
            .field("base_url", &self.base_url)
            .field("region", &self.region)
            .field("profile", &self.profile)
            .field("access_key_id", &redacted(self.access_key_id.as_deref()))
            .field(
                "secret_access_key",
                &redacted(self.secret_access_key.as_deref()),
            )
            .field("session_token", &redacted(self.session_token.as_deref()))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProviderAdapterConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "is_default")]
    pub model_discovery: ProviderModelDiscoveryConfig,
    #[serde(flatten)]
    pub definition: ProviderAdapterDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderAdapterDefinition {
    Ollama(OllamaProviderOptions),
    OpenAi(HttpProviderAdapterConfig<OpenAiProviderOptions>),
    Anthropic(HttpProviderAdapterConfig<AnthropicProviderOptions>),
    Gemini(HttpProviderAdapterConfig<GeminiProviderOptions>),
    Gitlab(GitlabProviderOptions),
    AmazonBedrock(AmazonBedrockProviderOptions),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolvedProviderModelConfig {
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub definition: ConfiguredModelDefinition,
}

impl Default for ResolvedProviderModelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            definition: ConfiguredModelDefinition::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaProviderOptions {
    pub base_url: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct HttpProviderAdapterConfig<T> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
    pub options: T,
}

impl<T: fmt::Debug> fmt::Debug for HttpProviderAdapterConfig<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpProviderAdapterConfig")
            .field("user_agent", &self.user_agent)
            .field("extra_headers", &self.extra_headers)
            .field("options", &self.options)
            .finish()
    }
}

fn redacted(value: Option<&str>) -> &'static str {
    match value {
        Some(s) if !s.is_empty() => "***redacted***",
        _ => "<none>",
    }
}

fn credential_debug_kind(value: &AuthData) -> &'static str {
    match value {
        AuthData::Api { .. } => "api",
        AuthData::OAuth { .. } => "oauth",
        AuthData::WellKnown { .. } => "well_known",
    }
}

fn is_default<T>(value: &T) -> bool
where
    T: Default + PartialEq,
{
    value == &T::default()
}

fn is_true(value: &bool) -> bool {
    *value
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiProviderOptions {
    pub backend: OpenAiBackendConfig,
    pub api_mode: OpenAiApiModeConfig,
    #[serde(skip_serializing)]
    pub api_mode_explicit: bool,
    pub stream_mode: StreamTransportMode,
    pub realtime_ws_url: Option<String>,
    pub models_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_family: Option<ProviderCapabilityFamilyConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnthropicProviderOptions {
    pub models_url: Option<String>,
    pub messages_url: Option<String>,
    pub auth_header: String,
    pub auth_scheme: Option<String>,
    pub extra_beta_header: Option<String>,
    pub eager_input_streaming: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GeminiProviderOptions {
    pub auth_header: Option<String>,
    pub auth_scheme: Option<String>,
    pub stream_mode: StreamTransportMode,
    pub realtime_ws_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SimpleHttpProviderOptions {
    pub auth_header: Option<String>,
    pub auth_scheme: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct GitlabProviderOptions {
    pub instance_url: Option<String>,
    pub ai_gateway_url: Option<String>,
    pub ai_gateway_headers: BTreeMap<String, String>,
    pub feature_flags: BTreeMap<String, bool>,
}

impl fmt::Debug for GitlabProviderOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitlabProviderOptions")
            .field("instance_url", &self.instance_url)
            .field("ai_gateway_url", &self.ai_gateway_url)
            .field("ai_gateway_headers", &self.ai_gateway_headers)
            .field("feature_flags", &self.feature_flags)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct AmazonBedrockProviderOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapabilityFamilyConfig {
    #[serde(rename = "openai")]
    OpenAi,
    Anthropic,
    Gemini,
    #[serde(rename = "bedrock")]
    Bedrock,
    Gitlab,
}

impl From<ProviderCapabilityFamilyConfig> for CapabilityFamily {
    fn from(value: ProviderCapabilityFamilyConfig) -> Self {
        match value {
            ProviderCapabilityFamilyConfig::OpenAi => CapabilityFamily::OpenAi,
            ProviderCapabilityFamilyConfig::Anthropic => CapabilityFamily::Anthropic,
            ProviderCapabilityFamilyConfig::Gemini => CapabilityFamily::Gemini,
            ProviderCapabilityFamilyConfig::Bedrock => CapabilityFamily::Bedrock,
            ProviderCapabilityFamilyConfig::Gitlab => CapabilityFamily::Gitlab,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ConfigOutputFormat {
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamTransportMode {
    Sse,
    #[serde(rename = "realtime_websocket")]
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

impl From<StreamTransportMode> for GeminiStreamMode {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiBackendConfig {
    #[default]
    Api,
    ChatgptCodex,
}

impl FromStr for OpenAiBackendConfig {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "api" => Ok(Self::Api),
            "chatgpt_codex" => Ok(Self::ChatgptCodex),
            _ => Err(ConfigError::InvalidOverride(format!(
                "unknown openai backend `{value}`"
            ))),
        }
    }
}

impl From<OpenAiBackendConfig> for OpenAiBackend {
    fn from(value: OpenAiBackendConfig) -> Self {
        match value {
            OpenAiBackendConfig::Api => Self::Api,
            OpenAiBackendConfig::ChatgptCodex => Self::ChatgptCodex,
        }
    }
}

// ─────────────────────────── MCP ────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpConfig {
    /// Map of `<server_name> -> <transport spec>`.
    pub servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LspConfig {
    /// Map of `<server_name> -> <server spec>`. Each entry is spawned on
    /// demand by [`agena_lsp::LspRegistry`] when an LSP-using tool first
    /// touches a matching file.
    pub servers: BTreeMap<String, LspServerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct LspServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// File extensions (without the leading `.`) routed to this server.
    /// Empty matches everything.
    #[serde(default)]
    pub file_extensions: Vec<String>,
    /// Marker filenames whose presence identifies the project root.
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initialization_options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpServerConfig {
    /// Spawn a child process and exchange newline-delimited JSON over
    /// its stdin/stdout (the typical MCP server style).
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        cwd: Option<PathBuf>,
    },
    /// Connect to a streamable HTTP MCP server.
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<McpHttpAuthConfig>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpHttpAuthConfig {
    /// Static `Authorization: Bearer <token>`.
    Bearer { token: String },
    /// Read the bearer token from the named env var at connect time.
    BearerFromEnv { env: String },
    /// Resolve via the runtime's MCP token store.
    BearerFromStore,
    /// Free-form header map.
    Custom { headers: BTreeMap<String, String> },
}

// ─────────────────────────── Web tools ──────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebToolsConfig {
    pub fetch_enabled: bool,
    pub search: WebSearchConfig,
}

#[derive(Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebSearchConfig {
    /// Reads from `api_key` and falls back to `BRAVE_API_KEY`.
    pub api_key: Option<String>,
}

impl fmt::Debug for WebSearchConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSearchConfig")
            .field("api_key", &redacted(self.api_key.as_deref()))
            .finish()
    }
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self { api_key: None }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct WebSearchBackend {
    pub api_key: String,
}

impl fmt::Debug for WebSearchBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSearchBackend")
            .field("api_key", &redacted(Some(self.api_key.as_str())))
            .finish()
    }
}

impl WebSearchBackend {
    pub fn name(&self) -> &'static str {
        "brave"
    }
}

impl WebSearchConfig {
    /// Materialize the resolved backend, reading the API key from config or
    /// falling back to `BRAVE_API_KEY`.
    pub fn resolve(&self) -> WebSearchBackend {
        let api_key = self
            .api_key
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| std::env::var("BRAVE_API_KEY").unwrap_or_default());
        WebSearchBackend { api_key }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{McpServerConfig, WebSearchConfig};

    #[test]
    fn mcp_http_config_rejects_removed_mode_field() {
        let err = serde_json::from_value::<McpServerConfig>(json!({
            "transport": "http",
            "url": "https://example.com/mcp",
            "mode": "sse"
        }))
        .expect_err("removed MCP http mode should be rejected");
        assert!(err.to_string().contains("unknown field `mode`"));
    }

    #[test]
    fn web_search_config_rejects_removed_backend_field() {
        let err = serde_json::from_value::<WebSearchConfig>(json!({
            "backend": "exa",
            "api_key": "test"
        }))
        .expect_err("removed web search backend should be rejected");
        assert!(err.to_string().contains("unknown field `backend`"));
    }
}
