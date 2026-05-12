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
    AgentConfig, AuthConfig, AuthStoreBackend, ConfigEnvironment, ConfigError, McpConfig,
    MemoryConfig, OpenAiApiModeConfig, PluginConfig, ProjectInstructionsConfig, ProviderDefinition,
    ResolvedConfig, ResolvedProviderConfig, RuntimeConfig, StreamTransportMode, TelemetryConfig,
    TracingConfig, UiConfig, WebToolsConfig, provider_presets,
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
        let memory: MemoryConfig = self.memory.unwrap_or_default();
        let mcp: McpConfig = self.mcp.unwrap_or_default();
        let lsp: crate::config::types::LspConfig = self.lsp.unwrap_or_default();
        let web: WebToolsConfig = self.web.unwrap_or_default();

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
            hooks: crate::hooks::HooksConfig::new(self.hooks),
        })
    }
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
    #[serde(rename = "preset")]
    Preset,
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
    #[serde(rename = "codex")]
    Codex,
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
            "preset" => Ok(Self::Preset),
            "ollama" => Ok(Self::Ollama),
            "openai" => Ok(Self::OpenAi),
            "openai_compatible" => Ok(Self::OpenAiCompatible),
            "sap_ai_core" => Ok(Self::SapAiCore),
            "anthropic" => Ok(Self::Anthropic),
            "gemini" => Ok(Self::Gemini),
            "codex" => Ok(Self::Codex),
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
pub(crate) struct RawProviderConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) kind: Option<ProviderKind>,
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
    pub(crate) models: BTreeMap<String, ConfiguredModelDefinition>,
}

impl Merge for RawProviderConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.enabled, overlay.enabled);
        merge_option(&mut self.kind, overlay.kind);
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
        env: &dyn ConfigEnvironment,
    ) -> Result<(String, ResolvedProviderConfig), ConfigError> {
        if matches!(self.kind, Some(ProviderKind::Preset)) {
            self = provider_presets::apply_provider_preset(provider_id.as_str(), self, env)?;
        }

        let kind = self.kind.ok_or_else(|| ConfigError::MissingProviderKind {
            provider_id: provider_id.clone(),
        })?;
        let enabled = self.enabled.unwrap_or(true);
        for configured in self.models.values_mut() {
            configured.capabilities.normalize_compact_patch();
        }
        validate_configured_models(provider_id.as_str(), &self.models)?;
        let models = self.models.clone();

        let definition = match kind {
            ProviderKind::Preset => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id,
                    message: "preset provider must be resolved before building concrete definition"
                        .to_owned(),
                });
            }
            ProviderKind::Ollama => ProviderDefinition::Ollama(super::OllamaProviderOptions {
                base_url: self
                    .base_url
                    .unwrap_or_else(|| "http://localhost:11434".to_owned()),
                default_model: required_string(
                    provider_id.as_str(),
                    "default_model",
                    self.default_model,
                )?,
            }),
            ProviderKind::OpenAi => ProviderDefinition::OpenAi(super::HttpProviderConfig {
                base_url: required_string(provider_id.as_str(), "base_url", self.base_url)?,
                default_model: required_string(
                    provider_id.as_str(),
                    "default_model",
                    self.default_model,
                )?,
                api_key: normalize_optional(self.api_key),
                api_key_env: normalize_optional(self.api_key_env),
                extra_headers: self.extra_headers,
                options: super::OpenAiProviderOptions {
                    api_mode: self.api_mode.unwrap_or(OpenAiApiModeConfig::Responses),
                    stream_mode: self.stream_mode.unwrap_or(StreamTransportMode::Sse),
                    realtime_ws_url: normalize_optional(self.realtime_ws_url),
                },
            }),
            ProviderKind::OpenAiCompatible => {
                ProviderDefinition::OpenAiCompatible(super::HttpProviderConfig {
                    base_url: required_string(provider_id.as_str(), "base_url", self.base_url)?,
                    default_model: required_string(
                        provider_id.as_str(),
                        "default_model",
                        self.default_model,
                    )?,
                    api_key: normalize_optional(self.api_key),
                    api_key_env: normalize_optional(self.api_key_env),
                    extra_headers: self.extra_headers,
                    options: super::OpenAiCompatibleProviderOptions {
                        auth_header: self
                            .auth_header
                            .unwrap_or_else(|| "authorization".to_owned()),
                        auth_scheme: normalize_optional(self.auth_scheme)
                            .or_else(|| Some("Bearer".to_owned())),
                        stream_mode: self.stream_mode.unwrap_or(StreamTransportMode::Sse),
                        realtime_ws_url: normalize_optional(self.realtime_ws_url),
                    },
                })
            }
            ProviderKind::SapAiCore => ProviderDefinition::SapAiCore(super::HttpProviderConfig {
                base_url: required_string(provider_id.as_str(), "base_url", self.base_url)?,
                default_model: required_string(
                    provider_id.as_str(),
                    "default_model",
                    self.default_model,
                )?,
                api_key: normalize_optional(self.api_key),
                api_key_env: normalize_optional(self.api_key_env),
                extra_headers: self.extra_headers,
                options: super::OpenAiCompatibleProviderOptions {
                    auth_header: self
                        .auth_header
                        .unwrap_or_else(|| "authorization".to_owned()),
                    auth_scheme: normalize_optional(self.auth_scheme)
                        .or_else(|| Some("Bearer".to_owned())),
                    stream_mode: self.stream_mode.unwrap_or(StreamTransportMode::Sse),
                    realtime_ws_url: normalize_optional(self.realtime_ws_url),
                },
            }),
            ProviderKind::Anthropic => ProviderDefinition::Anthropic(super::HttpProviderConfig {
                base_url: required_string(provider_id.as_str(), "base_url", self.base_url)?,
                default_model: required_string(
                    provider_id.as_str(),
                    "default_model",
                    self.default_model,
                )?,
                api_key: normalize_optional(self.api_key),
                api_key_env: normalize_optional(self.api_key_env),
                extra_headers: self.extra_headers,
                options: super::AnthropicProviderOptions {
                    auth_header: self.auth_header.unwrap_or_else(|| "x-api-key".to_owned()),
                    auth_scheme: normalize_optional(self.auth_scheme),
                },
            }),
            ProviderKind::Gemini => ProviderDefinition::Gemini(super::HttpProviderConfig {
                base_url: required_string(provider_id.as_str(), "base_url", self.base_url)?,
                default_model: required_string(
                    provider_id.as_str(),
                    "default_model",
                    self.default_model,
                )?,
                api_key: normalize_optional(self.api_key),
                api_key_env: normalize_optional(self.api_key_env),
                extra_headers: self.extra_headers,
                options: super::SimpleHttpProviderOptions,
            }),
            ProviderKind::Codex => ProviderDefinition::Codex(super::CodexProviderOptions {
                default_model: required_string(
                    provider_id.as_str(),
                    "default_model",
                    self.default_model,
                )?,
                auth_provider_id: self.auth_provider_id.unwrap_or_else(|| "openai".to_owned()),
            }),
            ProviderKind::Gitlab => ProviderDefinition::Gitlab(super::GitlabProviderOptions {
                instance_url: self
                    .instance_url
                    .unwrap_or_else(|| "https://gitlab.com".to_owned()),
                ai_gateway_url: self
                    .ai_gateway_url
                    .unwrap_or_else(|| "https://cloud.gitlab.com".to_owned()),
                default_model: self
                    .default_model
                    .unwrap_or_else(|| "claude-sonnet-4-5".to_owned()),
                auth_provider_id: self.auth_provider_id.unwrap_or_else(|| "gitlab".to_owned()),
                api_key: normalize_optional(self.api_key),
                api_key_env: normalize_optional(self.api_key_env),
                ai_gateway_headers: self.ai_gateway_headers,
                feature_flags: self.feature_flags,
            }),
            ProviderKind::Copilot => ProviderDefinition::Copilot(super::CopilotProviderOptions {
                default_model: self
                    .default_model
                    .unwrap_or_else(|| "gpt-4o-mini".to_owned()),
                base_url: self
                    .base_url
                    .unwrap_or_else(|| "https://api.githubcopilot.com".to_owned()),
                models_url: normalize_optional(self.models_url),
                auth_provider_id: self.auth_provider_id.unwrap_or_else(|| provider_id.clone()),
            }),
            ProviderKind::AmazonBedrock => {
                let static_credential_count =
                    [self.access_key_id.as_ref(), self.secret_access_key.as_ref()]
                        .into_iter()
                        .flatten()
                        .count();

                let auth = if self.api_key.is_some() || self.api_key_env.is_some() {
                    super::BedrockAuthConfig::Bearer {
                        api_key: normalize_optional(self.api_key),
                        api_key_env: normalize_optional(self.api_key_env),
                    }
                } else {
                    if static_credential_count == 1 {
                        return Err(ConfigError::InvalidProviderConfig {
                            provider_id: provider_id.clone(),
                            message: "access_key_id and secret_access_key must be set together"
                                .to_owned(),
                        });
                    }
                    super::BedrockAuthConfig::Sigv4 {
                        profile: normalize_optional(self.profile),
                        access_key_id: normalize_optional(self.access_key_id),
                        secret_access_key: normalize_optional(self.secret_access_key),
                        session_token: normalize_optional(self.session_token),
                    }
                };

                ProviderDefinition::AmazonBedrock(super::AmazonBedrockProviderOptions {
                    base_url: required_string(provider_id.as_str(), "base_url", self.base_url)?,
                    default_model: required_string(
                        provider_id.as_str(),
                        "default_model",
                        self.default_model,
                    )?,
                    region: required_string(provider_id.as_str(), "region", self.region)?,
                    auth,
                })
            }
            ProviderKind::GoogleVertex => {
                let auth = if self.access_token.is_some() || self.access_token_env.is_some() {
                    super::GoogleVertexAuthConfig::StaticToken {
                        access_token: normalize_optional(self.access_token),
                        access_token_env: normalize_optional(self.access_token_env),
                    }
                } else {
                    super::GoogleVertexAuthConfig::Adc
                };
                ProviderDefinition::GoogleVertex(super::GoogleVertexProviderOptions {
                    base_url: required_string(provider_id.as_str(), "base_url", self.base_url)?,
                    default_model: required_string(
                        provider_id.as_str(),
                        "default_model",
                        self.default_model,
                    )?,
                    auth,
                })
            }
            ProviderKind::CloudflareAiGateway => {
                ProviderDefinition::CloudflareAiGateway(super::CloudflareAiGatewayProviderOptions {
                    base_url: required_string(provider_id.as_str(), "base_url", self.base_url)?,
                    default_model: required_string(
                        provider_id.as_str(),
                        "default_model",
                        self.default_model,
                    )?,
                    api_key: normalize_optional(self.api_key),
                    api_key_env: normalize_optional(self.api_key_env),
                })
            }
        };

        Ok((
            provider_id,
            ResolvedProviderConfig {
                enabled,
                models,
                definition,
            },
        ))
    }
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
    models: &BTreeMap<String, ConfiguredModelDefinition>,
) -> Result<(), ConfigError> {
    for (model_id, configured) in models {
        if model_id.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` model id cannot be empty"
            )));
        }
        if configured.is_empty() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` model `{model_id}` must set at least one field"
            )));
        }
        if let Err(message) = configured.capabilities.validate() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` model `{model_id}` has invalid capability patch: {message}"
            )));
        }
        validate_configured_variants(
            provider_id,
            format!("model `{model_id}` variants").as_str(),
            &configured.variants,
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
