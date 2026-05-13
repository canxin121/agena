use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use toml::Value;

use crate::provider::{
    ConfiguredModelDefinition, ConfiguredModelVariant, ProviderRequestRetryConfig,
    ProviderStreamReplayConfig, auth::FileAuthStore,
};

use super::{
    AgentConfig, AuthConfig, AuthStoreBackend, BedrockSigv4AuthConfig, ConfigEnvironment,
    ConfigError, HttpProviderAdapterConfig, McpConfig, MemoryConfig, OpenAiApiModeConfig,
    PluginConfig, ProjectInstructionsConfig, ProviderAdapterDefinition, ProviderAuthConfig,
    ProviderSapAiCoreAuthConfig, ProviderSecretAuthConfig, ResolvedConfig,
    ResolvedProviderAdapterConfig, ResolvedProviderConfig, ResolvedProviderModelConfig,
    RuntimeConfig, StreamTransportMode, TelemetryConfig, TracingConfig, UiConfig,
    WebToolsConfig,
};

const DEFAULT_LOG_FILTER: &str = "info";
const DEFAULT_DATABASE_LOG_LEVEL: &str = "error";

#[derive(Debug, Clone)]
pub(crate) struct RawConfigFile {
    pub(crate) config: RawConfig,
    pub(crate) found: bool,
}

impl RawConfigFile {
    pub(crate) fn read(path: &Path) -> Result<Self, ConfigError> {
        match fs::read_to_string(path) {
            Ok(text) => {
                reject_unsupported_fields(path, &text)?;
                let config = toml::from_str::<RawConfig>(&text).map_err(|source| {
                    ConfigError::ParseFile {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;
                Ok(Self {
                    config,
                    found: true,
                })
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                config: RawConfig::default(),
                found: false,
            }),
            Err(source) => Err(ConfigError::ReadFile {
                path: path.to_path_buf(),
                source,
            }),
        }
    }
}

fn reject_unsupported_fields(path: &Path, text: &str) -> Result<(), ConfigError> {
    let value = toml::from_str::<Value>(text).map_err(|source| ConfigError::ParseFile {
        path: path.to_path_buf(),
        source,
    })?;
    let Some(table) = value.as_table() else {
        return Ok(());
    };
    if table.contains_key("mode") {
        return Err(ConfigError::UnsupportedModeConfig { field: "mode" });
    }
    if table.contains_key("modes") {
        return Err(ConfigError::UnsupportedModeConfig { field: "modes" });
    }
    for field in ["memory", "mcp", "lsp", "web", "hooks"] {
        if table.contains_key(field) {
            return Err(ConfigError::Validation(format!(
                "`{field}` must be configured as first-party plugin options under `[plugins.list.\"agena.{field}\".options]`"
            )));
        }
    }
    if let Some(providers) = table.get("providers").and_then(Value::as_table) {
        for (provider_id, provider) in providers {
            let Some(provider) = provider.as_table() else {
                continue;
            };
            if provider.contains_key("variants") {
                return Err(ConfigError::Validation(format!(
                    "provider `{provider_id}` variants must be configured under `providers.{provider_id}.models.\"<model-id>\".variants`; provider-level variants are not supported"
                )));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawConfig {
    pub(crate) tracing: Option<RawTracingConfig>,
    pub(crate) telemetry: Option<RawTelemetryConfig>,
    pub(crate) auth: Option<RawAuthConfig>,
    pub(crate) ui: Option<RawUiConfig>,
    pub(crate) runtime: Option<RawRuntimeConfig>,
    pub(crate) permission: Option<crate::agent::PermissionConfig>,
    pub(crate) agents: BTreeMap<String, AgentConfig>,
    pub(crate) plugins: Option<PluginConfig>,
    pub(crate) memory: Option<MemoryConfig>,
    pub(crate) mcp: Option<McpConfig>,
    pub(crate) lsp: Option<crate::config::types::LspConfig>,
    pub(crate) web: Option<WebToolsConfig>,
    pub(crate) providers: BTreeMap<String, RawProviderConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "hooks")]
    pub(crate) hooks: Vec<crate::hooks::HookEntry>,
}

impl RawConfig {
    pub(crate) fn merge_from(&mut self, overlay: Self) {
        merge_option_struct(&mut self.tracing, overlay.tracing);
        merge_option_struct(&mut self.telemetry, overlay.telemetry);
        merge_option_struct(&mut self.auth, overlay.auth);
        merge_option_struct(&mut self.ui, overlay.ui);
        merge_option_struct(&mut self.runtime, overlay.runtime);
        merge_option_struct(&mut self.permission, overlay.permission);
        merge_map(&mut self.agents, overlay.agents);
        merge_option_struct(&mut self.plugins, overlay.plugins);
        merge_option_struct(&mut self.memory, overlay.memory);
        merge_option_struct(&mut self.mcp, overlay.mcp);
        merge_option_struct(&mut self.lsp, overlay.lsp);
        merge_option_struct(&mut self.web, overlay.web);
        merge_map(&mut self.providers, overlay.providers);
        if !overlay.hooks.is_empty() {
            self.hooks = overlay.hooks;
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tracing.is_none()
            && self.telemetry.is_none()
            && self.auth.is_none()
            && self.ui.is_none()
            && self.runtime.is_none()
            && self.permission.is_none()
            && self.agents.is_empty()
            && self.plugins.is_none()
            && self.memory.is_none()
            && self.mcp.is_none()
            && self.lsp.is_none()
            && self.web.is_none()
            && self.providers.is_empty()
            && self.hooks.is_empty()
    }

    pub(crate) fn provider_mut(&mut self, provider_id: &str) -> &mut RawProviderConfig {
        self.providers.entry(provider_id.to_owned()).or_default()
    }

    pub(crate) fn from_env(env: &dyn ConfigEnvironment) -> Result<Self, ConfigError> {
        let mut config = Self::default();

        if let Some(path) = env.var("AGENA_AUTH_FILE") {
            config
                .auth
                .get_or_insert_with(RawAuthConfig::default)
                .store_path = Some(PathBuf::from(path));
        }
        if let Some(filter) = env.var("AGENA_LOG") {
            config
                .tracing
                .get_or_insert_with(RawTracingConfig::default)
                .filter = Some(filter);
        }
        if let Some(level) = env.var("AGENA_DATABASE_LOG") {
            config
                .tracing
                .get_or_insert_with(RawTracingConfig::default)
                .database_level = Some(level);
        }
        if let Some(enabled) = env.var("AGENA_TELEMETRY_ENABLED") {
            config
                .telemetry
                .get_or_insert_with(RawTelemetryConfig::default)
                .enabled = Some(parse_bool("AGENA_TELEMETRY_ENABLED", enabled.as_str())?);
        }
        if let Some(service_name) = env.var("AGENA_OTEL_SERVICE_NAME") {
            config
                .telemetry
                .get_or_insert_with(RawTelemetryConfig::default)
                .service_name = Some(service_name);
        }
        if let Some(endpoint) = env
            .var("AGENA_OTEL_ENDPOINT")
            .or_else(|| env.var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"))
        {
            config
                .telemetry
                .get_or_insert_with(RawTelemetryConfig::default)
                .otlp_endpoint = Some(endpoint);
        }
        if let Some(locale) = env.var("AGENA_LOCALE") {
            config.ui.get_or_insert_with(RawUiConfig::default).locale = Some(locale);
        }
        if let Some(enabled) = env.var("AGENA_PLUGIN_ENABLED") {
            config
                .plugins
                .get_or_insert_with(PluginConfig::default)
                .enabled = parse_bool("AGENA_PLUGIN_ENABLED", enabled.as_str())?;
        }
        // Note: `AGENA_PLUGIN_PATHS` is no longer supported — the new plugin
        // config requires explicit `[plugins.list.<id>]` entries.

        apply_env_number(env, "AGENA_PROVIDER_HTTP_TIMEOUT_SECS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .provider_http
                .get_or_insert_with(RawProviderHttpConfig::default)
                .timeout_secs = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_CONNECT_TIMEOUT_SECS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .provider_http
                .get_or_insert_with(RawProviderHttpConfig::default)
                .connect_timeout_secs = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_REQUEST_MAX_RETRIES", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .request_retry
                .get_or_insert_with(RawRequestRetryConfig::default)
                .max_retries = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_RETRY_BASE_DELAY_MS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .request_retry
                .get_or_insert_with(RawRequestRetryConfig::default)
                .base_delay_ms = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_RETRY_MAX_DELAY_MS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .request_retry
                .get_or_insert_with(RawRequestRetryConfig::default)
                .max_delay_ms = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_STREAM_REPLAY_MAX_RETRIES", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .stream_replay
                .get_or_insert_with(RawStreamReplayConfig::default)
                .max_retries_after_output = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_STREAM_REPLAY_MAX_EVENTS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .stream_replay
                .get_or_insert_with(RawStreamReplayConfig::default)
                .max_tracked_events = Some(value);
        })?;

        for (key, value) in env.vars() {
            let Some(rest) = key.strip_prefix("AGENA_PROVIDER__") else {
                continue;
            };
            let mut parts = rest.split("__");
            let Some(provider_raw) = parts.next() else {
                continue;
            };
            let Some(field) = parts.next() else {
                continue;
            };
            let provider_id = normalize_env_provider_id(provider_raw);
            let provider = config.provider_mut(provider_id.as_str());
            match field {
                "ENABLED" => provider.enabled = Some(parse_bool(key.as_str(), value.as_str())?),
                "KIND" => provider.kind = Some(ProviderKind::from_str(value.as_str())?),
                "BACKEND" => {
                    provider.backend = Some(super::OpenAiBackendConfig::from_str(value.as_str())?)
                }
                "DEFAULT_MODEL" => provider.default_model = Some(value),
                "BASE_URL" => provider.base_url = Some(value),
                "API_KEY" => provider.api_key = Some(value),
                "API_KEY_ENV" => provider.api_key_env = Some(value),
                "AUTH_HEADER" => provider.auth_header = Some(value),
                "AUTH_SCHEME" => provider.auth_scheme = Some(value),
                "STREAM_MODE" => {
                    provider.stream_mode = Some(StreamTransportMode::from_str(value.as_str())?)
                }
                "API_MODE" => {
                    provider.api_mode = Some(OpenAiApiModeConfig::from_str(value.as_str())?)
                }
                "REALTIME_WS_URL" => provider.realtime_ws_url = Some(value),
                "AUTH_PROVIDER_ID" => provider.auth_provider_id = Some(value),
                "INSTANCE_URL" => provider.instance_url = Some(value),
                "AI_GATEWAY_URL" => provider.ai_gateway_url = Some(value),
                "MODELS_URL" => provider.models_url = Some(value),
                "REGION" => provider.region = Some(value),
                "PROFILE" => provider.profile = Some(value),
                "ACCESS_TOKEN" => provider.access_token = Some(value),
                "ACCESS_TOKEN_ENV" => provider.access_token_env = Some(value),
                "ACCESS_KEY_ID" => provider.access_key_id = Some(value),
                "SECRET_ACCESS_KEY" => provider.secret_access_key = Some(value),
                "SESSION_TOKEN" => provider.session_token = Some(value),
                _ => {}
            }
        }

        Ok(config)
    }

    #[allow(dead_code)]
    pub(crate) fn resolve(self) -> Result<ResolvedConfig, ConfigError> {
        self.resolve_with_env(&super::ProcessEnvironment)
    }

    pub(crate) fn resolve_with_env(
        self,
        env: &dyn ConfigEnvironment,
    ) -> Result<ResolvedConfig, ConfigError> {
        let raw_tracing = self.tracing.unwrap_or_default();
        let database_level = raw_tracing
            .database_level
            .unwrap_or_else(|| DEFAULT_DATABASE_LOG_LEVEL.to_owned());
        validate_database_log_level(database_level.as_str())?;
        let tracing = TracingConfig {
            filter: raw_tracing
                .filter
                .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_owned()),
            database_level,
        };
        let raw_telemetry = self.telemetry.unwrap_or_default();
        let telemetry = TelemetryConfig {
            enabled: raw_telemetry.enabled.unwrap_or(false),
            service_name: raw_telemetry
                .service_name
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| TelemetryConfig::default().service_name),
            otlp_endpoint: raw_telemetry
                .otlp_endpoint
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            headers: raw_telemetry.headers,
        };

        let raw_auth = self.auth.unwrap_or_default();
        let auth = AuthConfig {
            store_path: raw_auth
                .store_path
                .unwrap_or_else(FileAuthStore::default_path),
            store_backend: raw_auth.store_backend.unwrap_or_default(),
        };
        let ui = UiConfig {
            locale: self
                .ui
                .and_then(|value| value.locale)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        };

        let runtime = RuntimeConfig::from_raw(self.runtime.unwrap_or_default())?;
        let permission = self.permission.unwrap_or_default();
        let plugins: PluginConfig = self.plugins.unwrap_or_default();
        let plugin_options = PluginRuntimeOptions::from_plugins(&plugins)?;
        let memory: MemoryConfig = plugin_options.memory.or(self.memory).unwrap_or_default();
        let mcp: McpConfig = plugin_options.mcp.or(self.mcp).unwrap_or_default();
        let lsp: crate::config::types::LspConfig =
            plugin_options.lsp.or(self.lsp).unwrap_or_default();
        let web: WebToolsConfig = plugin_options.web.or(self.web).unwrap_or_default();
        let hooks = plugin_options.hooks.unwrap_or(self.hooks);

        let providers = self
            .providers
            .into_iter()
            .map(|(provider_id, raw)| raw.resolve(provider_id, env))
            .collect::<Result<BTreeMap<_, _>, _>>()?;

        validate_permission_config("permission", &permission)?;
        for (agent_name, agent) in &self.agents {
            let effective = agent.permission.effective_with_defaults(&permission);
            validate_permission_config(
                format!("agents.{agent_name}.permission").as_str(),
                &effective,
            )?;
        }

        Ok(ResolvedConfig {
            tracing,
            telemetry,
            auth,
            ui,
            runtime,
            permission,
            agents: self.agents,
            plugins,
            plugin_storage: crate::config::types::PluginStorageConfig::default(),
            memory,
            mcp,
            lsp,
            web,
            providers,
            hooks: crate::hooks::HooksConfig::new(hooks),
        })
    }
}

#[derive(Debug, Default)]
struct PluginRuntimeOptions {
    memory: Option<MemoryConfig>,
    hooks: Option<Vec<crate::hooks::HookEntry>>,
    mcp: Option<McpConfig>,
    lsp: Option<crate::config::types::LspConfig>,
    web: Option<WebToolsConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HooksPluginOptions {
    hooks: Vec<crate::hooks::HookEntry>,
}

impl Default for HooksPluginOptions {
    fn default() -> Self {
        Self { hooks: Vec::new() }
    }
}

impl PluginRuntimeOptions {
    fn from_plugins(plugins: &PluginConfig) -> Result<Self, ConfigError> {
        let mut out = Self::default();
        for (plugin_id, entry) in &plugins.list {
            if matches!(
                plugin_id.as_str(),
                "agena.memory" | "agena.hooks" | "agena.mcp" | "agena.lsp" | "agena.web"
            ) {
                ensure_first_party_static_plugin(plugin_id, entry)?;
            }
            let options = entry.options();
            if options.is_null() {
                continue;
            }
            match plugin_id.as_str() {
                "agena.memory" => {
                    out.memory = Some(parse_plugin_options(plugin_id, options.clone())?);
                }
                "agena.hooks" => {
                    let parsed: HooksPluginOptions =
                        parse_plugin_options(plugin_id, options.clone())?;
                    out.hooks = Some(parsed.hooks);
                }
                "agena.mcp" => {
                    out.mcp = Some(parse_plugin_options(plugin_id, options.clone())?);
                }
                "agena.lsp" => {
                    out.lsp = Some(parse_plugin_options(plugin_id, options.clone())?);
                }
                "agena.web" => {
                    out.web = Some(parse_plugin_options(plugin_id, options.clone())?);
                }
                _ => {}
            }
        }
        Ok(out)
    }
}

fn ensure_first_party_static_plugin(
    plugin_id: &str,
    entry: &agena_plugin_host::PluginEntry,
) -> Result<(), ConfigError> {
    if matches!(entry, agena_plugin_host::PluginEntry::Static { .. }) {
        Ok(())
    } else {
        Err(ConfigError::Validation(format!(
            "plugins.list.{plugin_id} is registered by the runtime and must use `kind = \"static\"`"
        )))
    }
}

fn parse_plugin_options<T>(plugin_id: &str, value: serde_json::Value) -> Result<T, ConfigError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(value)
        .map_err(|err| ConfigError::Validation(format!("plugins.list.{plugin_id}.options: {err}")))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[allow(dead_code)]
pub(crate) struct RawPluginConfig;

impl Merge for RawPluginConfig {
    fn merge_from(&mut self, _overlay: Self) {}
}

impl Merge for PluginConfig {
    fn merge_from(&mut self, overlay: Self) {
        // Overlay completely replaces nested plugin entries; otherwise we'd
        // need entry-level merge logic. List entries from a more-specific
        // mode override the parent.
        self.enabled = overlay.enabled;
        if !overlay.list.is_empty() {
            self.list = overlay.list;
        }
        self.timeouts = overlay.timeouts;
    }
}

impl Merge for McpConfig {
    fn merge_from(&mut self, overlay: Self) {
        // A more-specific layer fully replaces server entries with the same
        // name; entries it doesn't mention pass through unchanged.
        for (name, server) in overlay.servers {
            self.servers.insert(name, server);
        }
    }
}

impl Merge for crate::config::types::LspConfig {
    fn merge_from(&mut self, overlay: Self) {
        for (name, server) in overlay.servers {
            self.servers.insert(name, server);
        }
    }
}

impl Merge for WebToolsConfig {
    fn merge_from(&mut self, overlay: Self) {
        // Whole-struct replace.  WebToolsConfig is small and there's no
        // sensible per-field overlay semantics to preserve.
        *self = overlay;
    }
}

impl Merge for MemoryConfig {
    fn merge_from(&mut self, overlay: Self) {
        self.project_instructions
            .merge_from(overlay.project_instructions);
    }
}

impl Merge for ProjectInstructionsConfig {
    fn merge_from(&mut self, overlay: Self) {
        *self = overlay;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawTracingConfig {
    pub(crate) filter: Option<String>,
    pub(crate) database_level: Option<String>,
}

impl Merge for RawTracingConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.filter, overlay.filter);
        merge_option(&mut self.database_level, overlay.database_level);
    }
}

fn validate_database_log_level(value: &str) -> Result<(), ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "error" | "warn" | "info" | "debug" | "trace" => Ok(()),
        _ => Err(ConfigError::Validation(format!(
            "tracing.database_level expects one of off,error,warn,info,debug,trace, got `{value}`"
        ))),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawTelemetryConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) service_name: Option<String>,
    pub(crate) otlp_endpoint: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

impl Merge for RawTelemetryConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.enabled, overlay.enabled);
        merge_option(&mut self.service_name, overlay.service_name);
        merge_option(&mut self.otlp_endpoint, overlay.otlp_endpoint);
        self.headers.extend(overlay.headers);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawAuthConfig {
    pub(crate) store_path: Option<PathBuf>,
    pub(crate) store_backend: Option<AuthStoreBackend>,
}

impl Merge for RawAuthConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.store_path, overlay.store_path);
        merge_option(&mut self.store_backend, overlay.store_backend);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawUiConfig {
    pub(crate) locale: Option<String>,
}

impl Merge for RawUiConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.locale, overlay.locale);
    }
}

impl Merge for crate::config::types::AgentConfig {
    fn merge_from(&mut self, overlay: Self) {
        if !overlay.description.is_empty() {
            self.description = overlay.description;
        }
        if !overlay.prompt.is_empty() {
            self.prompt = overlay.prompt;
        }
        self.mode = overlay.mode;
        if overlay.hidden {
            self.hidden = true;
        }
        merge_option(&mut self.color, overlay.color);
        merge_option(&mut self.temperature, overlay.temperature);
        merge_option(&mut self.max_output_tokens, overlay.max_output_tokens);
        merge_option(&mut self.steps, overlay.steps);
        if !overlay.allowed_tools.is_empty() {
            self.allowed_tools = overlay.allowed_tools;
        }
        self.permission.merge_from(overlay.permission);
        merge_option(&mut self.model, overlay.model);
        if !overlay.aliases.is_empty() {
            self.aliases = overlay.aliases;
        }
        if overlay.disabled {
            self.disabled = true;
        }
    }
}

impl Merge for crate::agent::PermissionConfig {
    fn merge_from(&mut self, overlay: Self) {
        self.merge_from(overlay);
    }
}

impl Merge for crate::permission::PermissionMode {
    fn merge_from(&mut self, overlay: Self) {
        *self = overlay;
    }
}

impl Merge for crate::agent::AgentTemperature {
    fn merge_from(&mut self, overlay: Self) {
        *self = overlay;
    }
}

impl Merge for crate::agent::AgentPermissionConfig {
    fn merge_from(&mut self, overlay: Self) {
        self.merge_from(overlay);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawRuntimeConfig {
    pub(crate) provider_http: Option<RawProviderHttpConfig>,
    pub(crate) request_retry: Option<RawRequestRetryConfig>,
    pub(crate) stream_replay: Option<RawStreamReplayConfig>,
    pub(crate) reload: Option<RawRuntimeReloadConfig>,
    pub(crate) janitor: Option<RawRuntimeJanitorConfig>,
    pub(crate) session_cache: Option<RawSessionCacheConfig>,
    pub(crate) default_agent: Option<String>,
}

impl Merge for RawRuntimeConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option_struct(&mut self.provider_http, overlay.provider_http);
        merge_option_struct(&mut self.request_retry, overlay.request_retry);
        merge_option_struct(&mut self.stream_replay, overlay.stream_replay);
        merge_option_struct(&mut self.reload, overlay.reload);
        merge_option_struct(&mut self.janitor, overlay.janitor);
        merge_option_struct(&mut self.session_cache, overlay.session_cache);
        merge_option(&mut self.default_agent, overlay.default_agent);
    }
}

impl RuntimeConfig {
    pub(crate) fn from_raw(raw: RawRuntimeConfig) -> Result<Self, ConfigError> {
        let provider_http = raw.provider_http.unwrap_or_default();
        let request_retry = raw.request_retry.unwrap_or_default();
        let stream_replay = raw.stream_replay.unwrap_or_default();
        let reload = raw.reload.unwrap_or_default();
        let janitor = raw.janitor.unwrap_or_default();
        let session_cache = raw.session_cache.unwrap_or_default();

        let timeout_secs = provider_http.timeout_secs.unwrap_or(120);
        let connect_timeout_secs = provider_http.connect_timeout_secs.unwrap_or(15);
        if timeout_secs == 0 || connect_timeout_secs == 0 {
            return Err(ConfigError::Validation(
                "runtime.provider_http timeout values must be greater than 0".to_owned(),
            ));
        }

        let base_delay_ms = request_retry.base_delay_ms.unwrap_or(250);
        let max_delay_ms = request_retry
            .max_delay_ms
            .unwrap_or(2_000)
            .max(base_delay_ms);
        let reload_poll_interval_secs = reload.poll_interval_secs.unwrap_or(2);
        let janitor_interval_secs = janitor.interval_secs.unwrap_or(30);
        let session_cache_ttl_secs = session_cache.ttl_secs.unwrap_or(15 * 60);
        let session_cache_max_sessions = session_cache.max_sessions.unwrap_or(128);
        let session_cache_max_bytes = session_cache.max_bytes.unwrap_or(64 * 1024 * 1024);

        if reload_poll_interval_secs == 0 {
            return Err(ConfigError::Validation(
                "runtime.reload.poll_interval_secs must be greater than 0".to_owned(),
            ));
        }
        if janitor_interval_secs == 0 {
            return Err(ConfigError::Validation(
                "runtime.janitor.interval_secs must be greater than 0".to_owned(),
            ));
        }
        if session_cache_ttl_secs == 0 {
            return Err(ConfigError::Validation(
                "runtime.session_cache.ttl_secs must be greater than 0".to_owned(),
            ));
        }
        if session_cache_max_sessions == 0 {
            return Err(ConfigError::Validation(
                "runtime.session_cache.max_sessions must be greater than 0".to_owned(),
            ));
        }
        if session_cache_max_bytes == 0 {
            return Err(ConfigError::Validation(
                "runtime.session_cache.max_bytes must be greater than 0".to_owned(),
            ));
        }

        Ok(Self {
            provider_http: super::ProviderHttpConfig {
                timeout_secs,
                connect_timeout_secs,
            },
            request_retry: super::RequestRetryConfig {
                max_retries: request_retry
                    .max_retries
                    .unwrap_or(ProviderRequestRetryConfig::default().max_retries),
                base_delay_ms,
                max_delay_ms,
            },
            stream_replay: super::StreamReplayConfig {
                max_retries_after_output: stream_replay
                    .max_retries_after_output
                    .unwrap_or(ProviderStreamReplayConfig::default().max_retries_after_output),
                max_tracked_events: stream_replay.max_tracked_events.unwrap_or(2_048),
            },
            reload: super::RuntimeReloadConfig {
                enabled: reload.enabled.unwrap_or(true),
                poll_interval_secs: reload_poll_interval_secs,
            },
            janitor: super::RuntimeJanitorConfig {
                enabled: janitor.enabled.unwrap_or(true),
                interval_secs: janitor_interval_secs,
            },
            session_cache: super::SessionCacheConfig {
                max_sessions: session_cache_max_sessions,
                ttl_secs: session_cache_ttl_secs,
                max_bytes: session_cache_max_bytes,
            },
            default_agent: raw
                .default_agent
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| Some("build".to_string())),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawProviderHttpConfig {
    pub(crate) timeout_secs: Option<u64>,
    pub(crate) connect_timeout_secs: Option<u64>,
}

impl Merge for RawProviderHttpConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.timeout_secs, overlay.timeout_secs);
        merge_option(&mut self.connect_timeout_secs, overlay.connect_timeout_secs);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawRequestRetryConfig {
    pub(crate) max_retries: Option<u32>,
    pub(crate) base_delay_ms: Option<u64>,
    pub(crate) max_delay_ms: Option<u64>,
}

impl Merge for RawRequestRetryConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.max_retries, overlay.max_retries);
        merge_option(&mut self.base_delay_ms, overlay.base_delay_ms);
        merge_option(&mut self.max_delay_ms, overlay.max_delay_ms);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawStreamReplayConfig {
    pub(crate) max_retries_after_output: Option<u32>,
    pub(crate) max_tracked_events: Option<usize>,
}

impl Merge for RawStreamReplayConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(
            &mut self.max_retries_after_output,
            overlay.max_retries_after_output,
        );
        merge_option(&mut self.max_tracked_events, overlay.max_tracked_events);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawRuntimeReloadConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) poll_interval_secs: Option<u64>,
}

impl Merge for RawRuntimeReloadConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.enabled, overlay.enabled);
        merge_option(&mut self.poll_interval_secs, overlay.poll_interval_secs);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawRuntimeJanitorConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) interval_secs: Option<u64>,
}

impl Merge for RawRuntimeJanitorConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.enabled, overlay.enabled);
        merge_option(&mut self.interval_secs, overlay.interval_secs);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawSessionCacheConfig {
    pub(crate) max_sessions: Option<usize>,
    pub(crate) ttl_secs: Option<u64>,
    pub(crate) max_bytes: Option<usize>,
}

impl Merge for RawSessionCacheConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.max_sessions, overlay.max_sessions);
        merge_option(&mut self.ttl_secs, overlay.ttl_secs);
        merge_option(&mut self.max_bytes, overlay.max_bytes);
    }
}

// PluginConfig (alias for agena_plugin_host::PluginsConfig) is parsed
// directly via serde; no `from_raw` adapter needed.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ProviderKind {
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "openai", alias = "open_ai")]
    OpenAi,
    #[serde(rename = "openai_compatible", alias = "open_ai_compatible")]
    OpenAiCompatible,
    #[serde(rename = "sap_ai_core")]
    SapAiCore,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "gitlab")]
    Gitlab,
    #[serde(rename = "copilot")]
    Copilot,
    #[serde(rename = "amazon_bedrock")]
    AmazonBedrock,
    #[serde(rename = "google_vertex")]
    GoogleVertex,
    #[serde(rename = "cloudflare_ai_gateway")]
    CloudflareAiGateway,
}

impl std::str::FromStr for ProviderKind {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "ollama" => Ok(Self::Ollama),
            "openai" => Ok(Self::OpenAi),
            "openai_compatible" => Ok(Self::OpenAiCompatible),
            "sap_ai_core" => Ok(Self::SapAiCore),
            "anthropic" => Ok(Self::Anthropic),
            "gemini" => Ok(Self::Gemini),
            "gitlab" => Ok(Self::Gitlab),
            "copilot" => Ok(Self::Copilot),
            "amazon_bedrock" => Ok(Self::AmazonBedrock),
            "google_vertex" => Ok(Self::GoogleVertex),
            "cloudflare_ai_gateway" => Ok(Self::CloudflareAiGateway),
            _ => Err(ConfigError::InvalidOverride(format!(
                "unknown provider kind `{value}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawProviderModelConfig {
    pub(crate) target_model: Option<String>,
    #[serde(flatten)]
    pub(crate) definition: ConfiguredModelDefinition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderAuthMode {
    None,
    Secret,
    BedrockSigv4,
    GoogleAdc,
    SapAiCore,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawProviderAuthConfig {
    pub(crate) mode: Option<ProviderAuthMode>,
    #[serde(alias = "api_key", alias = "access_token")]
    pub(crate) secret: Option<String>,
    #[serde(alias = "api_key_env", alias = "access_token_env")]
    pub(crate) secret_env: Option<String>,
    #[serde(alias = "auth_provider_id")]
    pub(crate) credential_provider_id: Option<String>,
    pub(crate) profile: Option<String>,
    pub(crate) access_key_id: Option<String>,
    pub(crate) secret_access_key: Option<String>,
    pub(crate) session_token: Option<String>,
    pub(crate) service_key_env: Option<String>,
}

impl Merge for RawProviderAuthConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.mode, overlay.mode);
        merge_option(&mut self.secret, overlay.secret);
        merge_option(&mut self.secret_env, overlay.secret_env);
        merge_option(
            &mut self.credential_provider_id,
            overlay.credential_provider_id,
        );
        merge_option(&mut self.profile, overlay.profile);
        merge_option(&mut self.access_key_id, overlay.access_key_id);
        merge_option(&mut self.secret_access_key, overlay.secret_access_key);
        merge_option(&mut self.session_token, overlay.session_token);
        merge_option(&mut self.service_key_env, overlay.service_key_env);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawProviderAdapterConfig {
    pub(crate) kind: Option<ProviderKind>,
    pub(crate) backend: Option<super::OpenAiBackendConfig>,
    pub(crate) default_model: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) auth_header: Option<String>,
    pub(crate) auth_scheme: Option<String>,
    pub(crate) extra_headers: BTreeMap<String, String>,
    pub(crate) api_mode: Option<OpenAiApiModeConfig>,
    pub(crate) stream_mode: Option<StreamTransportMode>,
    pub(crate) realtime_ws_url: Option<String>,
    pub(crate) instance_url: Option<String>,
    pub(crate) ai_gateway_url: Option<String>,
    pub(crate) ai_gateway_headers: BTreeMap<String, String>,
    pub(crate) feature_flags: BTreeMap<String, bool>,
    pub(crate) models_url: Option<String>,
    pub(crate) region: Option<String>,
    pub(crate) models: BTreeMap<String, RawProviderModelConfig>,
}

impl Merge for RawProviderAdapterConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.kind, overlay.kind);
        merge_option(&mut self.backend, overlay.backend);
        merge_option(&mut self.default_model, overlay.default_model);
        merge_option(&mut self.base_url, overlay.base_url);
        merge_option(&mut self.auth_header, overlay.auth_header);
        merge_option(&mut self.auth_scheme, overlay.auth_scheme);
        self.extra_headers.extend(overlay.extra_headers);
        merge_option(&mut self.api_mode, overlay.api_mode);
        merge_option(&mut self.stream_mode, overlay.stream_mode);
        merge_option(&mut self.realtime_ws_url, overlay.realtime_ws_url);
        merge_option(&mut self.instance_url, overlay.instance_url);
        merge_option(&mut self.ai_gateway_url, overlay.ai_gateway_url);
        self.ai_gateway_headers.extend(overlay.ai_gateway_headers);
        self.feature_flags.extend(overlay.feature_flags);
        merge_option(&mut self.models_url, overlay.models_url);
        merge_option(&mut self.region, overlay.region);
        self.models.extend(overlay.models);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawProviderConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) auth: Option<RawProviderAuthConfig>,
    pub(crate) adapters: BTreeMap<String, RawProviderAdapterConfig>,
    pub(crate) kind: Option<ProviderKind>,
    pub(crate) backend: Option<super::OpenAiBackendConfig>,
    pub(crate) auth_provider_id: Option<String>,
    pub(crate) default_model: Option<String>,
    pub(crate) base_url: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_key_env: Option<String>,
    pub(crate) auth_header: Option<String>,
    pub(crate) auth_scheme: Option<String>,
    pub(crate) extra_headers: BTreeMap<String, String>,
    pub(crate) api_mode: Option<OpenAiApiModeConfig>,
    pub(crate) stream_mode: Option<StreamTransportMode>,
    pub(crate) realtime_ws_url: Option<String>,
    pub(crate) instance_url: Option<String>,
    pub(crate) ai_gateway_url: Option<String>,
    pub(crate) ai_gateway_headers: BTreeMap<String, String>,
    pub(crate) feature_flags: BTreeMap<String, bool>,
    pub(crate) models_url: Option<String>,
    pub(crate) region: Option<String>,
    pub(crate) profile: Option<String>,
    pub(crate) access_token: Option<String>,
    pub(crate) access_token_env: Option<String>,
    pub(crate) access_key_id: Option<String>,
    pub(crate) secret_access_key: Option<String>,
    pub(crate) session_token: Option<String>,
    pub(crate) models: BTreeMap<String, RawProviderModelConfig>,
}

impl Merge for RawProviderConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.enabled, overlay.enabled);
        merge_option_struct(&mut self.auth, overlay.auth);
        self.adapters.extend(overlay.adapters);
        merge_option(&mut self.kind, overlay.kind);
        merge_option(&mut self.backend, overlay.backend);
        merge_option(&mut self.auth_provider_id, overlay.auth_provider_id);
        merge_option(&mut self.default_model, overlay.default_model);
        merge_option(&mut self.base_url, overlay.base_url);
        merge_option(&mut self.api_key, overlay.api_key);
        merge_option(&mut self.api_key_env, overlay.api_key_env);
        merge_option(&mut self.auth_header, overlay.auth_header);
        merge_option(&mut self.auth_scheme, overlay.auth_scheme);
        self.extra_headers.extend(overlay.extra_headers);
        merge_option(&mut self.api_mode, overlay.api_mode);
        merge_option(&mut self.stream_mode, overlay.stream_mode);
        merge_option(&mut self.realtime_ws_url, overlay.realtime_ws_url);
        merge_option(&mut self.instance_url, overlay.instance_url);
        merge_option(&mut self.ai_gateway_url, overlay.ai_gateway_url);
        self.ai_gateway_headers.extend(overlay.ai_gateway_headers);
        self.feature_flags.extend(overlay.feature_flags);
        merge_option(&mut self.models_url, overlay.models_url);
        merge_option(&mut self.region, overlay.region);
        merge_option(&mut self.profile, overlay.profile);
        merge_option(&mut self.access_token, overlay.access_token);
        merge_option(&mut self.access_token_env, overlay.access_token_env);
        merge_option(&mut self.access_key_id, overlay.access_key_id);
        merge_option(&mut self.secret_access_key, overlay.secret_access_key);
        merge_option(&mut self.session_token, overlay.session_token);
        self.models.extend(overlay.models);
    }
}

impl RawProviderConfig {
    fn resolve(
        mut self,
        provider_id: String,
        _env: &dyn ConfigEnvironment,
    ) -> Result<(String, ResolvedProviderConfig), ConfigError> {
        let enabled = self.enabled.unwrap_or(true);
        if self.adapters.is_empty() {
            let (adapter_id, adapter) = resolve_legacy_adapter(provider_id.as_str(), &self)?;
            let auth = resolve_legacy_auth(provider_id.as_str(), &self, &adapter.definition)?;
            validate_provider_auth(provider_id.as_str(), &auth, std::iter::once(&adapter))?;
            normalize_model_configs(&mut self.models);
            validate_configured_models(provider_id.as_str(), "provider", &self.models)?;
            let models = self
                .models
                .into_iter()
                .map(|(model_id, configured)| {
                    Ok((
                        model_id.clone(),
                        ResolvedProviderModelConfig {
                            adapter: adapter_id.clone(),
                            target_model: normalize_optional(configured.target_model)
                                .unwrap_or_else(|| model_id.clone()),
                            definition: configured.definition,
                        },
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
            let default_model =
                normalize_optional(self.default_model).unwrap_or_else(|| adapter.default_model.clone());
            return Ok((
                provider_id,
                ResolvedProviderConfig {
                    enabled,
                    default_model,
                    auth,
                    adapters: BTreeMap::from([(adapter_id, adapter)]),
                    models,
                },
            ));
        }

        ensure_no_legacy_provider_fields(provider_id.as_str(), &self)?;

        let mut adapters = BTreeMap::new();
        let mut models = BTreeMap::new();
        let adapter_count = self.adapters.len();
        for (adapter_id, mut adapter_raw) in self.adapters {
            normalize_model_configs(&mut adapter_raw.models);
            validate_configured_models(
                provider_id.as_str(),
                format!("adapter `{adapter_id}`").as_str(),
                &adapter_raw.models,
            )?;
            if adapter_count > 1 && adapter_raw.models.is_empty() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.clone(),
                    message: format!(
                        "multi-adapter provider requires explicit models under `providers.{provider_id}.adapters.{adapter_id}.models`"
                    ),
                });
            }
            let adapter = resolve_adapter(provider_id.as_str(), adapter_id.as_str(), adapter_raw)?;
            for (model_id, configured) in &adapter.models {
                if models.contains_key(model_id) {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.clone(),
                        message: format!("duplicate routed model `{model_id}` across adapters"),
                    });
                }
                models.insert(
                    model_id.clone(),
                    ResolvedProviderModelConfig {
                        adapter: adapter_id.clone(),
                        target_model: configured.target_model.clone(),
                        definition: configured.definition.clone(),
                    },
                );
            }
            adapters.insert(adapter_id, adapter.config);
        }

        let auth = resolve_provider_auth(provider_id.as_str(), self.auth, adapters.values())?;
        validate_provider_auth(provider_id.as_str(), &auth, adapters.values())?;
        let default_model = normalize_optional(self.default_model).unwrap_or_else(|| {
            adapters
                .values()
                .next()
                .map(|adapter| adapter.default_model.clone())
                .unwrap_or_default()
        });
        if default_model.is_empty() {
            return Err(ConfigError::MissingProviderField {
                provider_id: provider_id.clone(),
                field: "default_model",
            });
        }
        if adapters.len() > 1 && !models.contains_key(default_model.as_str()) {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: format!(
                    "multi-adapter provider default_model `{default_model}` must be declared under adapter models"
                ),
            });
        }

        Ok((
            provider_id,
            ResolvedProviderConfig {
                enabled,
                default_model,
                auth,
                adapters,
                models,
            },
        ))
    }
}

#[derive(Debug, Clone)]
struct ResolvedAdapterWithModels {
    config: ResolvedProviderAdapterConfig,
    models: BTreeMap<String, ResolvedProviderModelConfig>,
}

const DEFAULT_ADAPTER_ID: &str = "default";
const DEFAULT_SAP_AI_CORE_SERVICE_KEY_ENV: &str = "AICORE_SERVICE_KEY";

fn normalize_model_configs(models: &mut BTreeMap<String, RawProviderModelConfig>) {
    for configured in models.values_mut() {
        configured.definition.capabilities.normalize_compact_patch();
    }
}

fn ensure_no_legacy_provider_fields(
    provider_id: &str,
    provider: &RawProviderConfig,
) -> Result<(), ConfigError> {
    let has_legacy_fields = provider.kind.is_some()
        || provider.backend.is_some()
        || provider.auth_provider_id.is_some()
        || provider.base_url.is_some()
        || provider.api_key.is_some()
        || provider.api_key_env.is_some()
        || provider.auth_header.is_some()
        || provider.auth_scheme.is_some()
        || !provider.extra_headers.is_empty()
        || provider.api_mode.is_some()
        || provider.stream_mode.is_some()
        || provider.realtime_ws_url.is_some()
        || provider.instance_url.is_some()
        || provider.ai_gateway_url.is_some()
        || !provider.ai_gateway_headers.is_empty()
        || !provider.feature_flags.is_empty()
        || provider.models_url.is_some()
        || provider.region.is_some()
        || provider.profile.is_some()
        || provider.access_token.is_some()
        || provider.access_token_env.is_some()
        || provider.access_key_id.is_some()
        || provider.secret_access_key.is_some()
        || provider.session_token.is_some()
        || !provider.models.is_empty();

    if has_legacy_fields {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "provider with `adapters` must move legacy provider fields into `auth` or `adapters.<id>` blocks".to_owned(),
        });
    }

    Ok(())
}

fn resolve_legacy_adapter(
    provider_id: &str,
    provider: &RawProviderConfig,
) -> Result<(String, ResolvedProviderAdapterConfig), ConfigError> {
    let kind = provider.kind.ok_or_else(|| ConfigError::MissingProviderKind {
        provider_id: provider_id.to_owned(),
    })?;
    Ok((
        DEFAULT_ADAPTER_ID.to_owned(),
        resolve_adapter_config(
            provider_id,
            DEFAULT_ADAPTER_ID,
            kind,
            provider.backend,
            provider.default_model.clone(),
            provider.base_url.clone(),
            provider.auth_header.clone(),
            provider.auth_scheme.clone(),
            provider.extra_headers.clone(),
            provider.api_mode,
            provider.stream_mode,
            provider.realtime_ws_url.clone(),
            provider.instance_url.clone(),
            provider.ai_gateway_url.clone(),
            provider.ai_gateway_headers.clone(),
            provider.feature_flags.clone(),
            provider.models_url.clone(),
            provider.region.clone(),
            provider.api_key.clone(),
            provider.api_key_env.clone(),
        )?,
    ))
}

fn resolve_adapter(
    provider_id: &str,
    adapter_id: &str,
    raw: RawProviderAdapterConfig,
) -> Result<ResolvedAdapterWithModels, ConfigError> {
    let kind = raw.kind.ok_or_else(|| ConfigError::MissingProviderKind {
        provider_id: provider_id.to_owned(),
    })?;
    let config = resolve_adapter_config(
        provider_id,
        adapter_id,
        kind,
        raw.backend,
        raw.default_model,
        raw.base_url,
        raw.auth_header,
        raw.auth_scheme,
        raw.extra_headers,
        raw.api_mode,
        raw.stream_mode,
        raw.realtime_ws_url,
        raw.instance_url,
        raw.ai_gateway_url,
        raw.ai_gateway_headers,
        raw.feature_flags,
        raw.models_url,
        raw.region,
        None,
        None,
    )?;
    let models = raw
        .models
        .into_iter()
        .map(|(model_id, configured)| {
            Ok((
                model_id.clone(),
                ResolvedProviderModelConfig {
                    adapter: adapter_id.to_owned(),
                    target_model: normalize_optional(configured.target_model)
                        .unwrap_or_else(|| model_id.clone()),
                    definition: configured.definition,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ConfigError>>()?;
    Ok(ResolvedAdapterWithModels { config, models })
}

#[allow(clippy::too_many_arguments)]
fn resolve_adapter_config(
    provider_id: &str,
    adapter_id: &str,
    kind: ProviderKind,
    backend: Option<super::OpenAiBackendConfig>,
    default_model: Option<String>,
    base_url: Option<String>,
    auth_header: Option<String>,
    auth_scheme: Option<String>,
    extra_headers: BTreeMap<String, String>,
    api_mode: Option<OpenAiApiModeConfig>,
    stream_mode: Option<StreamTransportMode>,
    realtime_ws_url: Option<String>,
    instance_url: Option<String>,
    ai_gateway_url: Option<String>,
    ai_gateway_headers: BTreeMap<String, String>,
    feature_flags: BTreeMap<String, bool>,
    models_url: Option<String>,
    region: Option<String>,
    legacy_api_key: Option<String>,
    legacy_api_key_env: Option<String>,
) -> Result<ResolvedProviderAdapterConfig, ConfigError> {
    let field_provider_id = if adapter_id == DEFAULT_ADAPTER_ID {
        provider_id.to_owned()
    } else {
        format!("{provider_id}:{adapter_id}")
    };

    let (default_model, definition) = match kind {
        ProviderKind::Ollama => (
            required_string(
                field_provider_id.as_str(),
                "default_model",
                default_model,
            )?,
            ProviderAdapterDefinition::Ollama(super::OllamaProviderOptions {
                base_url: base_url.unwrap_or_else(|| "http://localhost:11434".to_owned()),
            }),
        ),
        ProviderKind::OpenAi => {
            let backend = backend.unwrap_or_default();
            let api_mode = api_mode.unwrap_or(OpenAiApiModeConfig::Responses);
            let stream_mode = stream_mode.unwrap_or(StreamTransportMode::Sse);
            let realtime_ws_url = normalize_optional(realtime_ws_url);
            if matches!(backend, super::OpenAiBackendConfig::ChatgptCodex) {
                if normalize_optional(legacy_api_key).is_some()
                    || normalize_optional(legacy_api_key_env).is_some()
                {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "openai backend `chatgpt_codex` uses auth-store OAuth; do not set `api_key` or `api_key_env`".to_owned(),
                    });
                }
                if api_mode != OpenAiApiModeConfig::Responses {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "openai backend `chatgpt_codex` only supports `api_mode = \"responses\"`".to_owned(),
                    });
                }
                if stream_mode != StreamTransportMode::Sse {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "openai backend `chatgpt_codex` only supports `stream_mode = \"sse\"`".to_owned(),
                    });
                }
                if realtime_ws_url.is_some() {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message:
                            "openai backend `chatgpt_codex` does not support `realtime_ws_url`"
                                .to_owned(),
                    });
                }
            }
            (
                required_string(
                    field_provider_id.as_str(),
                    "default_model",
                    default_model,
                )?,
                ProviderAdapterDefinition::OpenAi(HttpProviderAdapterConfig {
                    base_url: normalize_optional(base_url)
                        .unwrap_or_else(|| default_openai_base_url(backend).to_owned()),
                    extra_headers,
                    options: super::OpenAiProviderOptions {
                        backend,
                        api_mode,
                        stream_mode,
                        realtime_ws_url,
                    },
                }),
            )
        }
        ProviderKind::OpenAiCompatible => (
            required_string(
                field_provider_id.as_str(),
                "default_model",
                default_model,
            )?,
            ProviderAdapterDefinition::OpenAiCompatible(HttpProviderAdapterConfig {
                base_url: required_string(field_provider_id.as_str(), "base_url", base_url)?,
                extra_headers,
                options: super::OpenAiCompatibleProviderOptions {
                    auth_header: auth_header.unwrap_or_else(|| "authorization".to_owned()),
                    auth_scheme: normalize_optional(auth_scheme)
                        .or_else(|| Some("Bearer".to_owned())),
                    stream_mode: stream_mode.unwrap_or(StreamTransportMode::Sse),
                    realtime_ws_url: normalize_optional(realtime_ws_url),
                },
            }),
        ),
        ProviderKind::SapAiCore => (
            required_string(
                field_provider_id.as_str(),
                "default_model",
                default_model,
            )?,
            ProviderAdapterDefinition::SapAiCore(HttpProviderAdapterConfig {
                base_url: required_string(field_provider_id.as_str(), "base_url", base_url)?,
                extra_headers,
                options: super::OpenAiCompatibleProviderOptions {
                    auth_header: auth_header.unwrap_or_else(|| "authorization".to_owned()),
                    auth_scheme: normalize_optional(auth_scheme)
                        .or_else(|| Some("Bearer".to_owned())),
                    stream_mode: stream_mode.unwrap_or(StreamTransportMode::Sse),
                    realtime_ws_url: normalize_optional(realtime_ws_url),
                },
            }),
        ),
        ProviderKind::Anthropic => (
            required_string(
                field_provider_id.as_str(),
                "default_model",
                default_model,
            )?,
            ProviderAdapterDefinition::Anthropic(HttpProviderAdapterConfig {
                base_url: required_string(field_provider_id.as_str(), "base_url", base_url)?,
                extra_headers,
                options: super::AnthropicProviderOptions {
                    auth_header: auth_header.unwrap_or_else(|| "x-api-key".to_owned()),
                    auth_scheme: normalize_optional(auth_scheme),
                },
            }),
        ),
        ProviderKind::Gemini => (
            required_string(
                field_provider_id.as_str(),
                "default_model",
                default_model,
            )?,
            ProviderAdapterDefinition::Gemini(HttpProviderAdapterConfig {
                base_url: required_string(field_provider_id.as_str(), "base_url", base_url)?,
                extra_headers,
                options: super::SimpleHttpProviderOptions,
            }),
        ),
        ProviderKind::Gitlab => (
            default_model.unwrap_or_else(|| "claude-sonnet-4-5".to_owned()),
            ProviderAdapterDefinition::Gitlab(super::GitlabProviderOptions {
                instance_url: instance_url.unwrap_or_else(|| "https://gitlab.com".to_owned()),
                ai_gateway_url: ai_gateway_url
                    .unwrap_or_else(|| "https://cloud.gitlab.com".to_owned()),
                ai_gateway_headers,
                feature_flags,
            }),
        ),
        ProviderKind::Copilot => (
            default_model.unwrap_or_else(|| "gpt-4o-mini".to_owned()),
            ProviderAdapterDefinition::Copilot(super::CopilotProviderOptions {
                base_url: base_url
                    .unwrap_or_else(|| "https://api.githubcopilot.com".to_owned()),
                models_url: normalize_optional(models_url),
            }),
        ),
        ProviderKind::AmazonBedrock => (
            required_string(
                field_provider_id.as_str(),
                "default_model",
                default_model,
            )?,
            ProviderAdapterDefinition::AmazonBedrock(super::AmazonBedrockProviderOptions {
                base_url: required_string(field_provider_id.as_str(), "base_url", base_url)?,
                region: required_string(field_provider_id.as_str(), "region", region)?,
            }),
        ),
        ProviderKind::GoogleVertex => (
            required_string(
                field_provider_id.as_str(),
                "default_model",
                default_model,
            )?,
            ProviderAdapterDefinition::GoogleVertex(super::GoogleVertexProviderOptions {
                base_url: required_string(field_provider_id.as_str(), "base_url", base_url)?,
            }),
        ),
        ProviderKind::CloudflareAiGateway => (
            required_string(
                field_provider_id.as_str(),
                "default_model",
                default_model,
            )?,
            ProviderAdapterDefinition::CloudflareAiGateway(
                super::CloudflareAiGatewayProviderOptions {
                    base_url: required_string(field_provider_id.as_str(), "base_url", base_url)?,
                },
            ),
        ),
    };

    Ok(ResolvedProviderAdapterConfig {
        default_model,
        definition,
    })
}

fn resolve_legacy_auth(
    provider_id: &str,
    raw: &RawProviderConfig,
    adapter: &ProviderAdapterDefinition,
) -> Result<ProviderAuthConfig, ConfigError> {
    let auth = match adapter {
        ProviderAdapterDefinition::Ollama(_) => RawProviderAuthConfig {
            mode: Some(ProviderAuthMode::None),
            ..Default::default()
        },
        ProviderAdapterDefinition::OpenAi(config) => RawProviderAuthConfig {
            mode: Some(ProviderAuthMode::Secret),
            secret: raw.api_key.clone(),
            secret_env: raw.api_key_env.clone(),
            credential_provider_id: Some(
                raw.auth_provider_id.clone().unwrap_or_else(|| {
                    default_openai_auth_provider_id(provider_id, config.options.backend)
                }),
            ),
            ..Default::default()
        },
        ProviderAdapterDefinition::OpenAiCompatible(_)
        | ProviderAdapterDefinition::Anthropic(_)
        | ProviderAdapterDefinition::Gemini(_)
        | ProviderAdapterDefinition::CloudflareAiGateway(_) => RawProviderAuthConfig {
            mode: Some(ProviderAuthMode::Secret),
            secret: raw.api_key.clone(),
            secret_env: raw.api_key_env.clone(),
            credential_provider_id: raw.auth_provider_id.clone(),
            ..Default::default()
        },
        ProviderAdapterDefinition::SapAiCore(_) => RawProviderAuthConfig {
            mode: Some(ProviderAuthMode::SapAiCore),
            secret: raw.api_key.clone(),
            secret_env: raw.api_key_env.clone(),
            credential_provider_id: raw.auth_provider_id.clone(),
            service_key_env: Some(DEFAULT_SAP_AI_CORE_SERVICE_KEY_ENV.to_owned()),
            ..Default::default()
        },
        ProviderAdapterDefinition::Gitlab(_) => RawProviderAuthConfig {
            mode: Some(ProviderAuthMode::Secret),
            secret: raw.api_key.clone(),
            secret_env: raw.api_key_env.clone(),
            credential_provider_id: Some(
                raw.auth_provider_id
                    .clone()
                    .unwrap_or_else(|| "gitlab".to_owned()),
            ),
            ..Default::default()
        },
        ProviderAdapterDefinition::Copilot(_) => RawProviderAuthConfig {
            mode: Some(ProviderAuthMode::Secret),
            credential_provider_id: Some(
                raw.auth_provider_id
                    .clone()
                    .unwrap_or_else(|| provider_id.to_owned()),
            ),
            ..Default::default()
        },
        ProviderAdapterDefinition::AmazonBedrock(_) => {
            if raw.api_key.is_some() || raw.api_key_env.is_some() {
                RawProviderAuthConfig {
                    mode: Some(ProviderAuthMode::Secret),
                    secret: raw.api_key.clone(),
                    secret_env: raw.api_key_env.clone(),
                    credential_provider_id: raw.auth_provider_id.clone(),
                    ..Default::default()
                }
            } else {
                RawProviderAuthConfig {
                    mode: Some(ProviderAuthMode::BedrockSigv4),
                    profile: raw.profile.clone(),
                    access_key_id: raw.access_key_id.clone(),
                    secret_access_key: raw.secret_access_key.clone(),
                    session_token: raw.session_token.clone(),
                    ..Default::default()
                }
            }
        }
        ProviderAdapterDefinition::GoogleVertex(_) => {
            if raw.access_token.is_some() || raw.access_token_env.is_some() {
                RawProviderAuthConfig {
                    mode: Some(ProviderAuthMode::Secret),
                    secret: raw.access_token.clone(),
                    secret_env: raw.access_token_env.clone(),
                    credential_provider_id: raw.auth_provider_id.clone(),
                    ..Default::default()
                }
            } else {
                RawProviderAuthConfig {
                    mode: Some(ProviderAuthMode::GoogleAdc),
                    ..Default::default()
                }
            }
        }
    };

    let adapter = ResolvedProviderAdapterConfig {
        default_model: String::new(),
        definition: adapter.clone(),
    };
    resolve_provider_auth(provider_id, Some(auth), std::iter::once(&adapter))
}

fn resolve_provider_auth<'a>(
    provider_id: &str,
    raw_auth: Option<RawProviderAuthConfig>,
    adapters: impl IntoIterator<Item = &'a ResolvedProviderAdapterConfig>,
) -> Result<ProviderAuthConfig, ConfigError> {
    let adapters = adapters.into_iter().collect::<Vec<_>>();
    let raw_auth = raw_auth.unwrap_or_default();
    let mode = raw_auth.mode.unwrap_or_else(|| infer_provider_auth_mode(&raw_auth, &adapters));
    match mode {
        ProviderAuthMode::None => Ok(ProviderAuthConfig::None),
        ProviderAuthMode::Secret => Ok(ProviderAuthConfig::Secret(ProviderSecretAuthConfig {
            secret: normalize_optional(raw_auth.secret),
            secret_env: normalize_optional(raw_auth.secret_env),
            credential_provider_id: normalize_optional(raw_auth.credential_provider_id),
        })),
        ProviderAuthMode::BedrockSigv4 => {
            let access_key_id = normalize_optional(raw_auth.access_key_id);
            let secret_access_key = normalize_optional(raw_auth.secret_access_key);
            if access_key_id.is_some() ^ secret_access_key.is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "access_key_id and secret_access_key must be set together"
                        .to_owned(),
                });
            }
            Ok(ProviderAuthConfig::BedrockSigv4(BedrockSigv4AuthConfig {
                profile: normalize_optional(raw_auth.profile),
                access_key_id,
                secret_access_key,
                session_token: normalize_optional(raw_auth.session_token),
            }))
        }
        ProviderAuthMode::GoogleAdc => Ok(ProviderAuthConfig::GoogleAdc),
        ProviderAuthMode::SapAiCore => Ok(ProviderAuthConfig::SapAiCore(
            ProviderSapAiCoreAuthConfig {
                secret: ProviderSecretAuthConfig {
                    secret: normalize_optional(raw_auth.secret),
                    secret_env: normalize_optional(raw_auth.secret_env),
                    credential_provider_id: normalize_optional(raw_auth.credential_provider_id),
                },
                service_key_env: normalize_optional(raw_auth.service_key_env)
                    .unwrap_or_else(|| DEFAULT_SAP_AI_CORE_SERVICE_KEY_ENV.to_owned()),
            },
        )),
    }
}

fn infer_provider_auth_mode(
    raw_auth: &RawProviderAuthConfig,
    adapters: &[&ResolvedProviderAdapterConfig],
) -> ProviderAuthMode {
    if raw_auth.secret.is_some()
        || raw_auth.secret_env.is_some()
        || raw_auth.credential_provider_id.is_some()
    {
        return ProviderAuthMode::Secret;
    }
    if raw_auth.access_key_id.is_some()
        || raw_auth.secret_access_key.is_some()
        || raw_auth.profile.is_some()
        || raw_auth.session_token.is_some()
    {
        return ProviderAuthMode::BedrockSigv4;
    }
    if raw_auth.service_key_env.is_some() {
        return ProviderAuthMode::SapAiCore;
    }
    if adapters
        .iter()
        .all(|adapter| matches!(adapter.definition, ProviderAdapterDefinition::Ollama(_)))
    {
        return ProviderAuthMode::None;
    }
    if adapters.iter().all(|adapter| {
        matches!(
            adapter.definition,
            ProviderAdapterDefinition::GoogleVertex(_)
        )
    }) {
        return ProviderAuthMode::GoogleAdc;
    }
    if adapters.iter().all(|adapter| {
        matches!(
            adapter.definition,
            ProviderAdapterDefinition::AmazonBedrock(_)
        )
    }) {
        return ProviderAuthMode::BedrockSigv4;
    }
    if adapters
        .iter()
        .all(|adapter| matches!(adapter.definition, ProviderAdapterDefinition::SapAiCore(_)))
    {
        return ProviderAuthMode::SapAiCore;
    }
    ProviderAuthMode::Secret
}

fn validate_provider_auth<'a>(
    provider_id: &str,
    auth: &ProviderAuthConfig,
    adapters: impl IntoIterator<Item = &'a ResolvedProviderAdapterConfig>,
) -> Result<(), ConfigError> {
    for adapter in adapters {
        match (auth, &adapter.definition) {
            (ProviderAuthConfig::None, ProviderAdapterDefinition::Ollama(_)) => {}
            (ProviderAuthConfig::None, _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "auth mode `none` only supports `ollama` adapters".to_owned(),
                });
            }
            (ProviderAuthConfig::GoogleAdc, ProviderAdapterDefinition::GoogleVertex(_)) => {}
            (ProviderAuthConfig::GoogleAdc, _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "auth mode `google_adc` only supports `google_vertex` adapters"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::BedrockSigv4(_), ProviderAdapterDefinition::AmazonBedrock(_)) => {
            }
            (ProviderAuthConfig::BedrockSigv4(_), _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "auth mode `bedrock_sigv4` only supports `amazon_bedrock` adapters"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::SapAiCore(_), ProviderAdapterDefinition::SapAiCore(_)) => {}
            (ProviderAuthConfig::SapAiCore(_), _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "auth mode `sap_ai_core` only supports `sap_ai_core` adapters"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::Secret(_secret), ProviderAdapterDefinition::Ollama(_)) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "secret auth is not supported by `ollama` adapters".to_owned(),
                });
            }
            (
                ProviderAuthConfig::Secret(secret),
                ProviderAdapterDefinition::Copilot(_),
            ) if secret.secret.is_some() || secret.secret_env.is_some() => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "`copilot` adapter only supports auth-store credential auth"
                        .to_owned(),
                });
            }
            (
                ProviderAuthConfig::Secret(secret),
                ProviderAdapterDefinition::OpenAi(config),
            ) if matches!(
                config.options.backend,
                super::OpenAiBackendConfig::ChatgptCodex
            ) && (secret.secret.is_some() || secret.secret_env.is_some()) =>
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "openai backend `chatgpt_codex` only supports auth-store credential auth"
                            .to_owned(),
                });
            }
            (ProviderAuthConfig::Secret(_), _) => {}
        }
    }

    Ok(())
}

pub(crate) trait Merge {
    fn merge_from(&mut self, overlay: Self);
}

pub(crate) fn merge_option<T>(base: &mut Option<T>, overlay: Option<T>) {
    if let Some(value) = overlay {
        *base = Some(value);
    }
}

pub(crate) fn merge_option_struct<T>(base: &mut Option<T>, overlay: Option<T>)
where
    T: Merge,
{
    match (base.as_mut(), overlay) {
        (Some(base), Some(overlay)) => base.merge_from(overlay),
        (None, Some(overlay)) => *base = Some(overlay),
        _ => {}
    }
}

pub(crate) fn merge_map<T>(base: &mut BTreeMap<String, T>, overlay: BTreeMap<String, T>)
where
    T: Merge,
{
    for (key, value) in overlay {
        match base.get_mut(&key) {
            Some(existing) => existing.merge_from(value),
            None => {
                base.insert(key, value);
            }
        }
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn validate_permission_config(
    label: &str,
    permission: &crate::agent::PermissionConfig,
) -> Result<(), ConfigError> {
    crate::agent::Agent::new(
        "__validate__",
        crate::permission::PermissionPolicy::allow_all(),
    )
    .try_with_permission_config(permission)
    .map(|_| ())
    .map_err(|err| ConfigError::Validation(format!("{label} is invalid: {err}")))
}

fn validate_configured_models(
    provider_id: &str,
    scope: &str,
    models: &BTreeMap<String, RawProviderModelConfig>,
) -> Result<(), ConfigError> {
    for (model_id, configured) in models {
        if model_id.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {scope} model id cannot be empty"
            )));
        }
        if configured.definition.is_empty() && normalize_optional(configured.target_model.clone()).is_none()
        {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {scope} model `{model_id}` must set at least one field or target_model"
            )));
        }
        if let Err(message) = configured.definition.capabilities.validate() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {scope} model `{model_id}` has invalid capability patch: {message}"
            )));
        }
        validate_configured_variants(
            provider_id,
            format!("{scope} model `{model_id}` variants").as_str(),
            &configured.definition.variants,
        )?;
    }
    Ok(())
}

fn validate_configured_variants(
    provider_id: &str,
    scope: &str,
    variants: &BTreeMap<String, ConfiguredModelVariant>,
) -> Result<(), ConfigError> {
    for (variant_name, variant) in variants {
        if variant_name.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {scope} variant name cannot be empty"
            )));
        }
        if variant.is_empty() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {scope} variant `{variant_name}` must set at least one field or disabled = true"
            )));
        }
    }
    Ok(())
}

fn required_string(
    provider_id: &str,
    field: &'static str,
    value: Option<String>,
) -> Result<String, ConfigError> {
    normalize_optional(value).ok_or_else(|| ConfigError::MissingProviderField {
        provider_id: provider_id.to_owned(),
        field,
    })
}

fn default_openai_base_url(backend: super::OpenAiBackendConfig) -> &'static str {
    match backend {
        super::OpenAiBackendConfig::Api => "https://api.openai.com/v1",
        super::OpenAiBackendConfig::ChatgptCodex => "https://chatgpt.com/backend-api/codex",
    }
}

fn default_openai_auth_provider_id(
    provider_id: &str,
    backend: super::OpenAiBackendConfig,
) -> String {
    match backend {
        super::OpenAiBackendConfig::Api => provider_id.to_owned(),
        super::OpenAiBackendConfig::ChatgptCodex => "openai".to_owned(),
    }
}

fn normalize_env_provider_id(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

fn parse_bool(key: &str, value: &str) -> Result<bool, ConfigError> {
    match value.trim() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ConfigError::InvalidOverride(format!(
            "{key} expects bool, got `{value}`"
        ))),
    }
}

fn apply_env_number<T, F>(
    env: &dyn ConfigEnvironment,
    key: &str,
    mut apply: F,
) -> Result<(), ConfigError>
where
    T: std::str::FromStr,
    F: FnMut(T),
{
    if let Some(value) = env.var(key) {
        apply(super::parse_numeric::<T>(value.as_str(), key)?);
    }
    Ok(())
}
