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
    Project,
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
    pub project_config_path: PathBuf,
    pub project_config_found: bool,
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
    #[serde(skip_serializing)]
    pub default_selection: ExecutionSelection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
    pub tracing: TracingConfig,
    pub ui: UiConfig,
    pub desktop: DesktopConfig,
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
    #[serde(default, skip_serializing_if = "HarnessesConfig::is_empty")]
    pub harnesses: HarnessesConfig,
    pub providers: BTreeMap<String, ResolvedProviderConfig>,
}

impl ResolvedConfig {
    pub fn provider_http_client_config(&self) -> ProviderHttpClientConfig {
        ProviderHttpClientConfig {
            timeout: Duration::from_secs(self.runtime.providers.http.timeout_secs),
            connect_timeout: Duration::from_secs(self.runtime.providers.http.connect_timeout_secs),
        }
    }

    pub fn provider_runtime_config(&self) -> ProviderRuntimeConfig {
        ProviderRuntimeConfig {
            request_retry: ProviderRequestRetryConfig {
                max_retries: self.runtime.providers.retry.max_retries,
                base_delay: Duration::from_millis(self.runtime.providers.retry.base_delay_ms),
                max_delay: Duration::from_millis(self.runtime.providers.retry.max_delay_ms),
            },
            stream_replay: ProviderStreamReplayConfig {
                max_retries_after_output: self
                    .runtime
                    .providers
                    .stream_replay
                    .max_retries_after_output,
                max_tracked_events: self.runtime.providers.stream_replay.max_tracked_events,
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

    pub fn provider_model_native_tool_bindings(
        &self,
        provider_id: &str,
    ) -> Option<BTreeMap<String, Vec<ProviderNativeToolBinding>>> {
        self.providers.get(provider_id).map(|provider| {
            provider
                .models
                .iter()
                .filter_map(|(route, model)| {
                    let bindings = model.native_tool_bindings();
                    (!bindings.is_empty()).then(|| (route.clone(), bindings))
                })
                .collect()
        })
    }
}

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopConfig {
    pub autostart_on_boot: bool,
    pub backend: DesktopBackendConfig,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            autostart_on_boot: true,
            backend: DesktopBackendConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesktopBackendConfig {
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cors_origins: Vec<String>,
    pub cors_allow_all: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_log_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_cookie_samesite: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_url: Option<String>,
}

impl Default for DesktopBackendConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3210,
            ui_dir: None,
            cors_origins: Vec::new(),
            cors_allow_all: false,
            backend_log_level: None,
            ui_password: Some(String::new()),
            ui_cookie_samesite: None,
            workspace_root: None,
            database_path: None,
            database_url: None,
        }
    }
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
    pub http: ProviderHttpConfig,
    pub retry: RequestRetryConfig,
    pub stream_replay: StreamReplayConfig,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNativeToolKind {
    WebSearch,
    FileSearch,
    CodeExecution,
    ImageGeneration,
    Computer,
    Bash,
    TextEditor,
    UrlContext,
    RemoteMcp,
}

impl ProviderNativeToolKind {
    pub const ALL: [Self; 9] = [
        Self::WebSearch,
        Self::FileSearch,
        Self::CodeExecution,
        Self::ImageGeneration,
        Self::Computer,
        Self::Bash,
        Self::TextEditor,
        Self::UrlContext,
        Self::RemoteMcp,
    ];

    pub const fn config_key(self) -> &'static str {
        match self {
            Self::WebSearch => "web_search",
            Self::FileSearch => "file_search",
            Self::CodeExecution => "code_execution",
            Self::ImageGeneration => "image_generation",
            Self::Computer => "computer",
            Self::Bash => "bash",
            Self::TextEditor => "text_editor",
            Self::UrlContext => "url_context",
            Self::RemoteMcp => "remote_mcp",
        }
    }

    pub const fn supports_route(self, route: ProviderNativeToolRoute) -> bool {
        match self {
            Self::WebSearch => matches!(
                route,
                ProviderNativeToolRoute::Disabled
                    | ProviderNativeToolRoute::Plugin
                    | ProviderNativeToolRoute::ProviderHosted
            ),
            Self::FileSearch | Self::CodeExecution | Self::ImageGeneration | Self::UrlContext => {
                matches!(
                    route,
                    ProviderNativeToolRoute::Disabled | ProviderNativeToolRoute::ProviderHosted
                )
            }
            Self::Computer | Self::Bash | Self::TextEditor => matches!(
                route,
                ProviderNativeToolRoute::Disabled | ProviderNativeToolRoute::ProviderHarness
            ),
            Self::RemoteMcp => matches!(
                route,
                ProviderNativeToolRoute::Disabled | ProviderNativeToolRoute::ProviderConnector
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderNativeToolRoute {
    Disabled,
    Plugin,
    ProviderHosted,
    ProviderHarness,
    ProviderConnector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolRoutesConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_search: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_execution: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_generation: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bash: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_editor: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url_context: Option<ProviderNativeToolRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_mcp: Option<ProviderNativeToolRoute>,
}

impl ProviderNativeToolRoutesConfig {
    pub const fn is_empty(&self) -> bool {
        self.web_search.is_none()
            && self.file_search.is_none()
            && self.code_execution.is_none()
            && self.image_generation.is_none()
            && self.computer.is_none()
            && self.bash.is_none()
            && self.text_editor.is_none()
            && self.url_context.is_none()
            && self.remote_mcp.is_none()
    }

    pub const fn route_for(&self, tool: ProviderNativeToolKind) -> Option<ProviderNativeToolRoute> {
        match tool {
            ProviderNativeToolKind::WebSearch => self.web_search,
            ProviderNativeToolKind::FileSearch => self.file_search,
            ProviderNativeToolKind::CodeExecution => self.code_execution,
            ProviderNativeToolKind::ImageGeneration => self.image_generation,
            ProviderNativeToolKind::Computer => self.computer,
            ProviderNativeToolKind::Bash => self.bash,
            ProviderNativeToolKind::TextEditor => self.text_editor,
            ProviderNativeToolKind::UrlContext => self.url_context,
            ProviderNativeToolKind::RemoteMcp => self.remote_mcp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NativeToolUserLocationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

impl NativeToolUserLocationConfig {
    pub const fn is_empty(&self) -> bool {
        self.country.is_none()
            && self.region.is_none()
            && self.city.is_none()
            && self.timezone.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeToolFreshness {
    Auto,
    Cached,
    Live,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedWebSearchConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_domains: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freshness: Option<NativeToolFreshness>,
    #[serde(
        default,
        skip_serializing_if = "NativeToolUserLocationConfig::is_empty"
    )]
    pub user_location: NativeToolUserLocationConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedWebSearchConfig {
    pub fn is_empty(&self) -> bool {
        self.allowed_domains.is_empty()
            && self.blocked_domains.is_empty()
            && self.freshness.is_none()
            && self.user_location.is_empty()
            && self.max_results.is_none()
            && self.search_context_size.is_none()
            && self.provider_options.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedFileSearchConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vector_store_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_results: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedFileSearchConfig {
    pub fn is_empty(&self) -> bool {
        self.vector_store_ids.is_empty()
            && self.max_results.is_none()
            && self.include_results.is_none()
            && self.provider_options.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HostedCodeExecutionContainerConfig {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_ids: Vec<String>,
}

impl HostedCodeExecutionContainerConfig {
    pub fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.id.is_none()
            && self.memory_limit.is_none()
            && self.file_ids.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedCodeExecutionConfig {
    #[serde(
        default,
        skip_serializing_if = "HostedCodeExecutionContainerConfig::is_empty"
    )]
    pub container: HostedCodeExecutionContainerConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedCodeExecutionConfig {
    pub fn is_empty(&self) -> bool {
        self.container.is_empty() && self.provider_options.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedImageGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moderation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedImageGenerationConfig {
    pub fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.size.is_none()
            && self.quality.is_none()
            && self.moderation.is_none()
            && self.provider_options.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedUrlContextConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_urls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
}

impl ProviderHostedUrlContextConfig {
    pub fn is_empty(&self) -> bool {
        self.max_urls.is_none() && self.provider_options.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderHostedToolConfigs {
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedWebSearchConfig::is_empty"
    )]
    pub web_search: ProviderHostedWebSearchConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedFileSearchConfig::is_empty"
    )]
    pub file_search: ProviderHostedFileSearchConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedCodeExecutionConfig::is_empty"
    )]
    pub code_execution: ProviderHostedCodeExecutionConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedImageGenerationConfig::is_empty"
    )]
    pub image_generation: ProviderHostedImageGenerationConfig,
    #[serde(
        default,
        skip_serializing_if = "ProviderHostedUrlContextConfig::is_empty"
    )]
    pub url_context: ProviderHostedUrlContextConfig,
}

impl ProviderHostedToolConfigs {
    pub fn is_empty(&self) -> bool {
        self.web_search.is_empty()
            && self.file_search.is_empty()
            && self.code_execution.is_empty()
            && self.image_generation.is_empty()
            && self.url_context.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeToolHarnessKind {
    Browser,
    Shell,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderNativeHarnessRef {
    pub kind: NativeToolHarnessKind,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeHarnessBindings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer: Option<ProviderNativeHarnessRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bash: Option<ProviderNativeHarnessRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_editor: Option<ProviderNativeHarnessRef>,
}

impl ProviderNativeHarnessBindings {
    pub const fn is_empty(&self) -> bool {
        self.computer.is_none() && self.bash.is_none() && self.text_editor.is_none()
    }

    pub fn binding_for(&self, tool: ProviderNativeToolKind) -> Option<&ProviderNativeHarnessRef> {
        match tool {
            ProviderNativeToolKind::Computer => self.computer.as_ref(),
            ProviderNativeToolKind::Bash => self.bash.as_ref(),
            ProviderNativeToolKind::TextEditor => self.text_editor.as_ref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeConnectorConfig {
    pub server: String,
    pub require_approval: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_filter: Vec<String>,
}

impl Default for ProviderNativeConnectorConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            require_approval: true,
            tool_filter: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNativeToolsConfig {
    pub enabled: bool,
    #[serde(
        default,
        skip_serializing_if = "ProviderNativeToolRoutesConfig::is_empty"
    )]
    pub routes: ProviderNativeToolRoutesConfig,
    #[serde(default, skip_serializing_if = "ProviderHostedToolConfigs::is_empty")]
    pub hosted: ProviderHostedToolConfigs,
    #[serde(
        default,
        skip_serializing_if = "ProviderNativeHarnessBindings::is_empty"
    )]
    pub harness: ProviderNativeHarnessBindings,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub connectors: BTreeMap<String, ProviderNativeConnectorConfig>,
}

impl ProviderNativeToolsConfig {
    pub fn is_empty(&self) -> bool {
        !self.enabled
            && self.routes.is_empty()
            && self.hosted.is_empty()
            && self.harness.is_empty()
            && self.connectors.is_empty()
    }

    pub fn bindings(&self) -> Vec<ProviderNativeToolBinding> {
        if !self.enabled {
            return Vec::new();
        }

        ProviderNativeToolKind::ALL
            .into_iter()
            .filter_map(|tool| {
                let route = self.routes.route_for(tool)?;
                if route == ProviderNativeToolRoute::Disabled {
                    return None;
                }
                if tool == ProviderNativeToolKind::FileSearch
                    && route == ProviderNativeToolRoute::ProviderHosted
                    && self.hosted.file_search.vector_store_ids.is_empty()
                {
                    return None;
                }
                Some(ProviderNativeToolBinding {
                    tool,
                    route,
                    harness: self.harness.binding_for(tool).cloned(),
                    connector_names: if tool == ProviderNativeToolKind::RemoteMcp {
                        self.connectors.keys().cloned().collect()
                    } else {
                        Vec::new()
                    },
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderNativeToolBinding {
    pub tool: ProviderNativeToolKind,
    pub route: ProviderNativeToolRoute,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<ProviderNativeHarnessRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connector_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HarnessViewportConfig {
    pub width: u32,
    pub height: u32,
}

impl HarnessViewportConfig {
    pub const fn is_empty(&self) -> bool {
        self.width == 0 && self.height == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserHarnessConfig {
    pub driver: String,
    pub headless: bool,
    #[serde(default, skip_serializing_if = "HarnessViewportConfig::is_empty")]
    pub viewport: HarnessViewportConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_options: Option<serde_json::Value>,
}

impl Default for BrowserHarnessConfig {
    fn default() -> Self {
        Self {
            driver: "playwright".to_owned(),
            headless: true,
            viewport: HarnessViewportConfig::default(),
            allowed_domains: Vec::new(),
            launch_options: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShellHarnessConfig {
    pub workspace_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl Default for ShellHarnessConfig {
    fn default() -> Self {
        Self {
            workspace_only: true,
            allow_commands: Vec::new(),
            deny_commands: Vec::new(),
            env: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorHarnessConfig {
    pub workspace_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_file_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_extensions: Vec<String>,
}

impl Default for EditorHarnessConfig {
    fn default() -> Self {
        Self {
            workspace_only: true,
            max_file_bytes: None,
            allowed_extensions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct HarnessesConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub browser: BTreeMap<String, BrowserHarnessConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub shell: BTreeMap<String, ShellHarnessConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub editor: BTreeMap<String, EditorHarnessConfig>,
}

impl HarnessesConfig {
    pub fn is_empty(&self) -> bool {
        self.browser.is_empty() && self.shell.is_empty() && self.editor.is_empty()
    }

    pub fn contains(&self, reference: &ProviderNativeHarnessRef) -> bool {
        match reference.kind {
            NativeToolHarnessKind::Browser => self.browser.contains_key(reference.name.as_str()),
            NativeToolHarnessKind::Shell => self.shell.contains_key(reference.name.as_str()),
            NativeToolHarnessKind::Editor => self.editor.contains_key(reference.name.as_str()),
        }
    }
}

pub type ProviderDefaultsConfig = crate::agents::AgentSelectionConfig;

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
    #[serde(default, skip_serializing_if = "ProviderNativeToolsConfig::is_empty")]
    pub native_tools: ProviderNativeToolsConfig,
    #[serde(flatten)]
    pub definition: ConfiguredModelDefinition,
}

impl Default for ResolvedProviderModelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            native_tools: ProviderNativeToolsConfig::default(),
            definition: ConfiguredModelDefinition::default(),
        }
    }
}

impl ResolvedProviderModelConfig {
    pub fn native_tool_bindings(&self) -> Vec<ProviderNativeToolBinding> {
        self.native_tools.bindings()
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
