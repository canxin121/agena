use std::{collections::BTreeMap, fmt, path::PathBuf, str::FromStr, time::Duration};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

use crate::provider::{
    ConfiguredModelDefinition, OpenAiApiMode, OpenAiCompatibleStreamMode, OpenAiStreamMode,
    ProviderHttpClientConfig, ProviderRequestRetryConfig, ProviderRuntimeConfig,
    ProviderStreamReplayConfig,
    auth::{ConfiguredAuthStore, FileAuthStore, KeyringAuthStore},
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
            ConfigOutputFormat::Toml => Ok(toml::to_string_pretty(self)?),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedConfig {
    pub tracing: TracingConfig,
    pub telemetry: agena_otel::TelemetryConfig,
    pub auth: AuthConfig,
    pub ui: UiConfig,
    pub runtime: RuntimeConfig,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::PermissionConfig::is_empty"
    )]
    pub permission: crate::agent::PermissionConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, AgentConfig>,
    pub plugins: PluginConfig,
    pub plugin_storage: PluginStorageConfig,
    pub memory: MemoryConfig,
    pub mcp: McpConfig,
    pub lsp: LspConfig,
    pub web: WebToolsConfig,
    pub providers: BTreeMap<String, ResolvedProviderConfig>,
    #[serde(default, skip_serializing_if = "crate::hooks::HooksConfig::is_empty")]
    pub hooks: crate::hooks::HooksConfig,
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

    pub fn auth_store(&self) -> ConfiguredAuthStore {
        let file = FileAuthStore::new(self.auth.store_path.clone());
        match self.auth.store_backend {
            AuthStoreBackend::File => ConfiguredAuthStore::File(file),
            AuthStoreBackend::Auto => {
                ConfiguredAuthStore::Keyring(KeyringAuthStore::system(file, true))
            }
            AuthStoreBackend::Keyring => {
                ConfiguredAuthStore::Keyring(KeyringAuthStore::system(file, false))
            }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TracingConfig {
    pub filter: String,
    pub database_level: String,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            filter: "info".to_string(),
            database_level: "error".to_string(),
        }
    }
}

impl TracingConfig {
    pub fn env_filter(&self) -> Result<EnvFilter, tracing_subscriber::filter::ParseError> {
        let mut filter = EnvFilter::try_new(self.filter.as_str())?;
        for target in ["sqlx", "sea_orm", "sea_orm_migration"] {
            filter = filter.add_directive(format!("{target}={}", self.database_level).parse()?);
        }
        Ok(filter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthConfig {
    pub store_path: PathBuf,
    pub store_backend: AuthStoreBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthStoreBackend {
    #[default]
    Auto,
    File,
    Keyring,
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
    pub reload: RuntimeReloadConfig,
    pub janitor: RuntimeJanitorConfig,
    pub session_cache: SessionCacheConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentConfig {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "crate::agent::AgentMode::is_primary")]
    pub mode: crate::agent::AgentMode,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<crate::agent::AgentTemperature>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::AgentPermissionConfig::is_empty"
    )]
    pub permission: crate::agent::AgentPermissionConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

pub use agena_plugin_host::PluginsConfig as PluginConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct MemoryConfig {
    pub project_instructions: ProjectInstructionsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, ConfiguredModelDefinition>,
    #[serde(flatten)]
    pub definition: ProviderDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderDefinition {
    Ollama(OllamaProviderOptions),
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
pub struct OllamaProviderOptions {
    pub base_url: String,
    pub default_model: String,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct HttpProviderConfig<T> {
    pub base_url: String,
    pub default_model: String,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub extra_headers: BTreeMap<String, String>,
    pub options: T,
}

impl<T: fmt::Debug> fmt::Debug for HttpProviderConfig<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpProviderConfig")
            .field("base_url", &self.base_url)
            .field("default_model", &self.default_model)
            .field("api_key", &redacted(self.api_key.as_deref()))
            .field("api_key_env", &self.api_key_env)
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

#[derive(Clone, PartialEq, Eq, Serialize)]
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

impl fmt::Debug for GitlabProviderOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitlabProviderOptions")
            .field("instance_url", &self.instance_url)
            .field("ai_gateway_url", &self.ai_gateway_url)
            .field("default_model", &self.default_model)
            .field("auth_provider_id", &self.auth_provider_id)
            .field("api_key", &redacted(self.api_key.as_deref()))
            .field("api_key_env", &self.api_key_env)
            .field("ai_gateway_headers", &self.ai_gateway_headers)
            .field("feature_flags", &self.feature_flags)
            .finish()
    }
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

#[derive(Clone, PartialEq, Eq, Serialize)]
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

impl fmt::Debug for BedrockAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bearer {
                api_key,
                api_key_env,
            } => f
                .debug_struct("Bearer")
                .field("api_key", &redacted(api_key.as_deref()))
                .field("api_key_env", api_key_env)
                .finish(),
            Self::Sigv4 {
                profile,
                access_key_id,
                secret_access_key,
                session_token,
            } => f
                .debug_struct("Sigv4")
                .field("profile", profile)
                .field("access_key_id", &redacted(access_key_id.as_deref()))
                .field("secret_access_key", &redacted(secret_access_key.as_deref()))
                .field("session_token", &redacted(session_token.as_deref()))
                .finish(),
        }
    }
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

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CloudflareAiGatewayProviderOptions {
    pub base_url: String,
    pub default_model: String,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
}

impl fmt::Debug for CloudflareAiGatewayProviderOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudflareAiGatewayProviderOptions")
            .field("base_url", &self.base_url)
            .field("default_model", &self.default_model)
            .field("api_key", &redacted(self.api_key.as_deref()))
            .field("api_key_env", &self.api_key_env)
            .finish()
    }
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

// ─────────────────────────── MCP ────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
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
#[serde(tag = "transport", rename_all = "snake_case")]
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
    /// Connect to an HTTP-based MCP server.  `mode = "sse"` uses the
    /// legacy long-lived GET /sse channel; `mode = "streamable_http"` uses
    /// the current spec where every POST may stream frames in its
    /// response body.
    Http {
        url: String,
        #[serde(default = "default_http_mode")]
        mode: McpHttpMode,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<McpHttpAuthConfig>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

fn default_http_mode() -> McpHttpMode {
    McpHttpMode::StreamableHttp
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpHttpMode {
    Sse,
    StreamableHttp,
}

// ─────────────────────────── Web tools ──────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WebToolsConfig {
    pub fetch_enabled: bool,
    pub search: WebSearchConfig,
}

#[derive(Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WebSearchConfig {
    pub backend: WebSearchBackendKind,
    /// Reads from `tavily_api_key`, `exa_api_key`, `brave_api_key` —
    /// when missing, the tool falls back to the corresponding env var.
    pub tavily_api_key: Option<String>,
    pub exa_api_key: Option<String>,
    pub brave_api_key: Option<String>,
}

impl fmt::Debug for WebSearchConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSearchConfig")
            .field("backend", &self.backend)
            .field("tavily_api_key", &redacted(self.tavily_api_key.as_deref()))
            .field("exa_api_key", &redacted(self.exa_api_key.as_deref()))
            .field("brave_api_key", &redacted(self.brave_api_key.as_deref()))
            .finish()
    }
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            backend: WebSearchBackendKind::DuckDuckGoHtml,
            tavily_api_key: None,
            exa_api_key: None,
            brave_api_key: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchBackendKind {
    Tavily,
    Exa,
    Brave,
    DuckDuckGoHtml,
}

/// Resolved variant used at runtime — bundles each backend with the
/// credentials it actually needs.
#[derive(Clone, PartialEq, Eq)]
pub enum WebSearchBackend {
    Tavily { api_key: String },
    Exa { api_key: String },
    Brave { api_key: String },
    DuckDuckGoHtml,
}

impl fmt::Debug for WebSearchBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tavily { api_key } => f
                .debug_struct("Tavily")
                .field("api_key", &redacted(Some(api_key)))
                .finish(),
            Self::Exa { api_key } => f
                .debug_struct("Exa")
                .field("api_key", &redacted(Some(api_key)))
                .finish(),
            Self::Brave { api_key } => f
                .debug_struct("Brave")
                .field("api_key", &redacted(Some(api_key)))
                .finish(),
            Self::DuckDuckGoHtml => f.debug_struct("DuckDuckGoHtml").finish(),
        }
    }
}

impl WebSearchBackend {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Tavily { .. } => "tavily",
            Self::Exa { .. } => "exa",
            Self::Brave { .. } => "brave",
            Self::DuckDuckGoHtml => "duckduckgo_html",
        }
    }
}

impl WebSearchConfig {
    /// Materialize the resolved backend, reading API keys from config or
    /// falling back to the conventional env var.
    pub fn resolve(&self) -> WebSearchBackend {
        fn pick(cfg: &Option<String>, env_key: &str) -> String {
            cfg.clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| std::env::var(env_key).unwrap_or_default())
        }
        match self.backend {
            WebSearchBackendKind::Tavily => WebSearchBackend::Tavily {
                api_key: pick(&self.tavily_api_key, "TAVILY_API_KEY"),
            },
            WebSearchBackendKind::Exa => WebSearchBackend::Exa {
                api_key: pick(&self.exa_api_key, "EXA_API_KEY"),
            },
            WebSearchBackendKind::Brave => WebSearchBackend::Brave {
                api_key: pick(&self.brave_api_key, "BRAVE_API_KEY"),
            },
            WebSearchBackendKind::DuckDuckGoHtml => WebSearchBackend::DuckDuckGoHtml,
        }
    }
}
