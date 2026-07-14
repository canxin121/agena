use super::{
    BTreeMap, Deserialize, EnvFilter, PathBuf, ProviderAuthConfig, ProviderDefaultsConfig,
    ResolvedProviderAdapterConfig, ResolvedProviderModelConfig, Serialize,
};

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
        for target in ["sqlx", "sea_orm"] {
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
    pub tui: TuiUiConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TuiColorSchemeConfig {
    #[default]
    Auto,
    Dark,
    Light,
}

impl std::str::FromStr for TuiColorSchemeConfig {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "dark" => Ok(Self::Dark),
            "light" => Ok(Self::Light),
            _ => Err(format!(
                "ui.tui.color_scheme expects one of auto,dark,light, got `{value}`"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct TuiUiConfig {
    pub color_scheme: TuiColorSchemeConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeConfig {
    pub providers: RuntimeProvidersConfig,
    pub model_catalog: RuntimeModelCatalogConfig,
    pub reload: RuntimeReloadConfig,
    pub session: RuntimeSessionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeProvidersConfig {
    pub client_versions: ProviderClientVersionSettings,
    pub http: ProviderHttpConfig,
    pub retry: RequestRetryConfig,
    pub stream_replay: StreamReplayConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderClientVersionSettings {
    pub codex: String,
    pub claude: String,
    pub gemini: String,
}

impl Default for ProviderClientVersionSettings {
    fn default() -> Self {
        Self {
            codex: crate::provider::DEFAULT_CODEX_CLIENT_VERSION.to_owned(),
            claude: crate::provider::DEFAULT_CLAUDE_CLIENT_VERSION.to_owned(),
            gemini: crate::provider::DEFAULT_GEMINI_CLIENT_VERSION.to_owned(),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RuntimeSessionConfig {
    pub cache: SessionCacheConfig,
    pub gc: RuntimeGcConfig,
    pub max_concurrent_tools: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RuntimeGcConfig {
    pub enabled: bool,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SessionCacheConfig {
    pub max_sessions: usize,
    pub ttl_secs: u64,
    pub max_bytes: usize,
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
        skip_serializing_if = "crate::agents::AgentSelectionConfig::is_empty"
    )]
    pub defaults: crate::agents::AgentSelectionConfig,
    #[serde(
        default,
        skip_serializing_if = "crate::agents::AgentToolsConfig::is_empty"
    )]
    pub tools: crate::agents::AgentToolsConfig,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

pub use agena_plugin_host::PluginsConfig as PluginConfig;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProviderConfig {
    pub enabled: bool,
    pub defaults: ProviderDefaultsConfig,
    pub auth: ProviderAuthConfig,
    pub adapters: BTreeMap<String, ResolvedProviderAdapterConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, ResolvedProviderModelConfig>,
}
