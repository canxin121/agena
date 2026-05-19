use std::{collections::BTreeMap, fs, path::Path, str::FromStr};

use serde::{Deserialize, Serialize};
use toml::Value;

use crate::provider::{
    ConfiguredModelDefinition, ConfiguredModelSpeedMode, ConfiguredModelThinkingMode,
    ProviderRequestRetryConfig, ProviderStreamReplayConfig,
    auth::{AuthData, CredentialIssuer},
};

use super::{
    AgentConfig, BedrockSigv4AuthConfig, ConfigEnvironment, ConfigError, DefaultConfig,
    HttpProviderAdapterConfig, McpConfig, MemoryConfig, OpenAiApiModeConfig, PluginConfig,
    ProjectInstructionsConfig, ProviderAdapterDefinition, ProviderApiAuthConfig,
    ProviderAuthConfig, ProviderCapabilityFamilyConfig, ProviderCredentialAuthConfig,
    ProviderGoogleAdcAuthConfig, ProviderModelDiscoveryConfig, ProviderProtocolPathsConfig,
    ProviderSapAiCoreAuthConfig, ResolvedConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, ResolvedProviderModelConfig, RuntimeConfig, RuntimeModelCatalogConfig,
    StreamTransportMode, TelemetryConfig, TracingConfig, UiConfig, WebToolsConfig,
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

pub(crate) fn validate_config_text(
    path: &Path,
    text: &str,
    env: &dyn ConfigEnvironment,
) -> Result<(), ConfigError> {
    reject_unsupported_fields(path, text)?;
    let config = toml::from_str::<RawConfig>(text).map_err(|source| ConfigError::ParseFile {
        path: path.to_path_buf(),
        source,
    })?;
    config.resolve_with_env(env)?;
    Ok(())
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
                "`{field}` must be configured as plugin options under `[plugins.list.\"agena.{field}\".options]`"
            )));
        }
    }
    if let Some(providers) = table.get("providers").and_then(Value::as_table) {
        for (provider_id, provider) in providers {
            let Some(provider) = provider.as_table() else {
                continue;
            };
            if provider.contains_key("variants")
                || provider.contains_key("thinking_modes")
                || provider.contains_key("speed_modes")
            {
                return Err(ConfigError::Validation(format!(
                    "provider `{provider_id}` model modes must be configured under `providers.{provider_id}.models.\"<model-id>\".thinking_modes` or `.speed_modes`; provider-level modes are not supported"
                )));
            }
            if let Some(adapters) = provider.get("adapters").and_then(Value::as_table) {
                for (adapter_id, adapter) in adapters {
                    let Some(adapter) = adapter.as_table() else {
                        continue;
                    };
                    if let Some(models) = adapter.get("models").and_then(Value::as_table) {
                        for (model_id, model) in models {
                            let Some(model) = model.as_table() else {
                                continue;
                            };
                            for field in ["target_model"] {
                                if model.contains_key(field) {
                                    return Err(ConfigError::Validation(format!(
                                        "provider `{provider_id}` adapter `{adapter_id}` model `{model_id}` does not support `{field}`"
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawConfig {
    pub(crate) default: Option<RawDefaultConfig>,
    pub(crate) tracing: Option<RawTracingConfig>,
    pub(crate) telemetry: Option<RawTelemetryConfig>,
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
        merge_option_struct(&mut self.default, overlay.default);
        merge_option_struct(&mut self.tracing, overlay.tracing);
        merge_option_struct(&mut self.telemetry, overlay.telemetry);
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
            && self.default.is_none()
            && self.telemetry.is_none()
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

    pub(crate) fn from_env(env: &dyn ConfigEnvironment) -> Result<Self, ConfigError> {
        reject_legacy_provider_env_overrides(env)?;
        let mut config = Self::default();

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
        apply_env_number(env, "AGENA_MODEL_CATALOG_CACHE_MAX_AGE_SECS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .model_catalog
                .get_or_insert_with(RawRuntimeModelCatalogConfig::default)
                .cache_max_age_secs = Some(value);
        })?;

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

        let ui = UiConfig {
            locale: self
                .ui
                .and_then(|value| value.locale)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        };

        let raw_default = self.default.unwrap_or_default();
        let raw_runtime = self.runtime.unwrap_or_default();
        let default = raw_default.clone().resolve();
        let runtime = RuntimeConfig::from_raw(raw_runtime)?;
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
            .map(|(provider_id, raw)| raw.resolve(provider_id, env, &raw_default))
            .collect::<Result<BTreeMap<_, _>, _>>()?;

        validate_default_config(&default, &providers)?;
        validate_permission_config("permission", &permission)?;
        for (agent_name, agent) in &self.agents {
            let effective = agent.permission.effective_with_defaults(&permission);
            validate_permission_config(
                format!("agents.{agent_name}.permission").as_str(),
                &effective,
            )?;
        }

        Ok(ResolvedConfig {
            default,
            tracing,
            telemetry,
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

fn validate_default_config(
    default: &DefaultConfig,
    providers: &BTreeMap<String, ResolvedProviderConfig>,
) -> Result<(), ConfigError> {
    let Some(provider_id) = default.provider.as_deref() else {
        return Ok(());
    };
    let Some(provider) = providers.get(provider_id) else {
        return Err(ConfigError::Validation(format!(
            "default.provider `{provider_id}` references unknown provider"
        )));
    };
    if !provider.enabled {
        return Err(ConfigError::Validation(format!(
            "default.provider `{provider_id}` references disabled provider"
        )));
    }

    let (Some(adapter_id), Some(model_id)) = (default.adapter.as_deref(), default.model.as_deref())
    else {
        return Ok(());
    };
    let adapter = provider.adapters.get(adapter_id).ok_or_else(|| {
        ConfigError::Validation(format!(
            "default.adapter `{adapter_id}` references unknown adapter on provider `{provider_id}`"
        ))
    })?;
    if !adapter.enabled {
        return Err(ConfigError::Validation(format!(
            "default.adapter `{adapter_id}` references disabled adapter on provider `{provider_id}`"
        )));
    }

    let route = format!("{adapter_id}/{model_id}");
    if matches!(provider.models.get(route.as_str()), Some(configured) if !configured.enabled) {
        return Err(ConfigError::Validation(format!(
            "default.model `{model_id}` references disabled model route `{route}` on provider `{provider_id}`"
        )));
    }

    Ok(())
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
        // Overlay completely replaces nested plugin tools; otherwise we'd
        // need tool-level merge logic. List entries from a more-specific
        // mode override the parent.
        self.enabled = overlay.enabled;
        if !overlay.list.is_empty() {
            self.list = overlay.list;
        }
        self.timeouts = overlay.timeouts;
        if !overlay.quotas.is_empty() {
            self.quotas = overlay.quotas;
        }
        if !overlay.trusted_keys.is_empty() {
            self.trusted_keys = overlay.trusted_keys;
        }
        if overlay.default_quota != Default::default() {
            self.default_quota = overlay.default_quota;
        }
        if !overlay.tool_presentation.is_default() {
            self.tool_presentation = overlay.tool_presentation;
        }
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
#[serde(deny_unknown_fields)]
pub(crate) struct RawDefaultConfig {
    pub(crate) provider: Option<String>,
    pub(crate) adapter: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) agent: Option<String>,
}

impl Merge for RawDefaultConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.provider, overlay.provider);
        merge_option(&mut self.adapter, overlay.adapter);
        merge_option(&mut self.model, overlay.model);
        merge_option(&mut self.agent, overlay.agent);
    }
}

impl RawDefaultConfig {
    fn resolve(self) -> DefaultConfig {
        DefaultConfig {
            provider: normalize_optional_string(self.provider),
            adapter: normalize_optional_string(self.adapter),
            model: normalize_optional_string(self.model),
            agent: normalize_optional_string(self.agent).unwrap_or_else(|| "build".to_owned()),
        }
    }

    fn provider_default_adapter(&self, provider_id: &str) -> Option<String> {
        let provider = self.provider.as_deref()?.trim();
        if provider != provider_id {
            return None;
        }
        let adapter = self.adapter.as_deref()?.trim();
        if adapter.is_empty() {
            return None;
        }
        Some(adapter.to_owned())
    }

    fn provider_default_model(&self, provider_id: &str) -> Option<String> {
        let provider = self.provider.as_deref()?.trim();
        if provider != provider_id {
            return None;
        }
        let model = self.model.as_deref()?.trim();
        if model.is_empty() {
            return None;
        }
        Some(model.to_owned())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawRuntimeConfig {
    pub(crate) provider_http: Option<RawProviderHttpConfig>,
    pub(crate) request_retry: Option<RawRequestRetryConfig>,
    pub(crate) stream_replay: Option<RawStreamReplayConfig>,
    pub(crate) model_catalog: Option<RawRuntimeModelCatalogConfig>,
    pub(crate) reload: Option<RawRuntimeReloadConfig>,
    pub(crate) janitor: Option<RawRuntimeJanitorConfig>,
    pub(crate) session_cache: Option<RawSessionCacheConfig>,
}

impl Merge for RawRuntimeConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option_struct(&mut self.provider_http, overlay.provider_http);
        merge_option_struct(&mut self.request_retry, overlay.request_retry);
        merge_option_struct(&mut self.stream_replay, overlay.stream_replay);
        merge_option_struct(&mut self.model_catalog, overlay.model_catalog);
        merge_option_struct(&mut self.reload, overlay.reload);
        merge_option_struct(&mut self.janitor, overlay.janitor);
        merge_option_struct(&mut self.session_cache, overlay.session_cache);
    }
}

impl RuntimeConfig {
    pub(crate) fn from_raw(raw: RawRuntimeConfig) -> Result<Self, ConfigError> {
        let provider_http = raw.provider_http.unwrap_or_default();
        let request_retry = raw.request_retry.unwrap_or_default();
        let stream_replay = raw.stream_replay.unwrap_or_default();
        let model_catalog = raw.model_catalog.unwrap_or_default();
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
        let model_catalog_cache_max_age_secs = model_catalog
            .cache_max_age_secs
            .unwrap_or(crate::model_catalog::DEFAULT_CACHE_MAX_AGE_SECS);

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
        if model_catalog_cache_max_age_secs == 0 {
            return Err(ConfigError::Validation(
                "runtime.model_catalog.cache_max_age_secs must be greater than 0".to_owned(),
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
            model_catalog: RuntimeModelCatalogConfig {
                cache_max_age_secs: model_catalog_cache_max_age_secs,
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
        })
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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
pub(crate) struct RawRuntimeModelCatalogConfig {
    pub(crate) cache_max_age_secs: Option<u64>,
}

impl Merge for RawRuntimeModelCatalogConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.cache_max_age_secs, overlay.cache_max_age_secs);
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
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "gitlab")]
    Gitlab,
    #[serde(rename = "amazon_bedrock")]
    AmazonBedrock,
}

impl std::str::FromStr for ProviderKind {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "ollama" => Ok(Self::Ollama),
            "openai" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            "gemini" => Ok(Self::Gemini),
            "gitlab" => Ok(Self::Gitlab),
            "amazon_bedrock" => Ok(Self::AmazonBedrock),
            _ => Err(ConfigError::InvalidOverride(format!(
                "unknown provider kind `{value}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct RawProviderModelConfig {
    pub(crate) enabled: Option<bool>,
    #[serde(flatten)]
    pub(crate) definition: ConfiguredModelDefinition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderAuthMode {
    None,
    Api,
    Credential,
    BedrockSigv4,
    GoogleAdc,
    SapAiCore,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProviderProtocolPathsConfig {
    pub(crate) openai: Option<String>,
    pub(crate) anthropic: Option<String>,
    pub(crate) gemini: Option<String>,
}

impl Merge for RawProviderProtocolPathsConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.openai, overlay.openai);
        merge_option(&mut self.anthropic, overlay.anthropic);
        merge_option(&mut self.gemini, overlay.gemini);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProviderAuthConfig {
    pub(crate) mode: Option<ProviderAuthMode>,
    pub(crate) base_url: Option<String>,
    pub(crate) protocol_paths: Option<RawProviderProtocolPathsConfig>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_key_env: Option<String>,
    pub(crate) issuer: Option<CredentialIssuer>,
    #[serde(default)]
    pub(crate) credential: Option<AuthData>,
    pub(crate) profile: Option<String>,
    pub(crate) access_key_id: Option<String>,
    pub(crate) secret_access_key: Option<String>,
    pub(crate) session_token: Option<String>,
    pub(crate) region: Option<String>,
    pub(crate) service_key_env: Option<String>,
}

impl Merge for RawProviderAuthConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.mode, overlay.mode);
        merge_option(&mut self.base_url, overlay.base_url);
        merge_option_struct(&mut self.protocol_paths, overlay.protocol_paths);
        merge_option(&mut self.api_key, overlay.api_key);
        merge_option(&mut self.api_key_env, overlay.api_key_env);
        merge_option(&mut self.issuer, overlay.issuer);
        merge_option(&mut self.credential, overlay.credential);
        merge_option(&mut self.profile, overlay.profile);
        merge_option(&mut self.access_key_id, overlay.access_key_id);
        merge_option(&mut self.secret_access_key, overlay.secret_access_key);
        merge_option(&mut self.session_token, overlay.session_token);
        merge_option(&mut self.region, overlay.region);
        merge_option(&mut self.service_key_env, overlay.service_key_env);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProviderAdapterConfig {
    pub(crate) backend: Option<super::OpenAiBackendConfig>,
    pub(crate) enabled: Option<bool>,
    pub(crate) model_discovery: Option<ProviderModelDiscoveryConfig>,
    pub(crate) base_url: Option<String>,
    pub(crate) models_url: Option<String>,
    pub(crate) capability_family: Option<ProviderCapabilityFamilyConfig>,
    pub(crate) messages_url: Option<String>,
    pub(crate) auth_header: Option<String>,
    pub(crate) auth_scheme: Option<String>,
    pub(crate) extra_beta_header: Option<String>,
    pub(crate) eager_input_streaming: Option<bool>,
    pub(crate) extra_headers: BTreeMap<String, String>,
    pub(crate) api_mode: Option<OpenAiApiModeConfig>,
    pub(crate) stream_mode: Option<StreamTransportMode>,
    pub(crate) realtime_ws_url: Option<String>,
    pub(crate) instance_url: Option<String>,
    pub(crate) ai_gateway_url: Option<String>,
    pub(crate) ai_gateway_headers: BTreeMap<String, String>,
    pub(crate) feature_flags: BTreeMap<String, bool>,
    pub(crate) models: BTreeMap<String, RawProviderModelConfig>,
}

impl Merge for RawProviderAdapterConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.backend, overlay.backend);
        merge_option(&mut self.enabled, overlay.enabled);
        merge_option(&mut self.model_discovery, overlay.model_discovery);
        merge_option(&mut self.base_url, overlay.base_url);
        merge_option(&mut self.models_url, overlay.models_url);
        merge_option(&mut self.capability_family, overlay.capability_family);
        merge_option(&mut self.messages_url, overlay.messages_url);
        merge_option(&mut self.auth_header, overlay.auth_header);
        merge_option(&mut self.auth_scheme, overlay.auth_scheme);
        merge_option(&mut self.extra_beta_header, overlay.extra_beta_header);
        merge_option(
            &mut self.eager_input_streaming,
            overlay.eager_input_streaming,
        );
        self.extra_headers.extend(overlay.extra_headers);
        merge_option(&mut self.api_mode, overlay.api_mode);
        merge_option(&mut self.stream_mode, overlay.stream_mode);
        merge_option(&mut self.realtime_ws_url, overlay.realtime_ws_url);
        merge_option(&mut self.instance_url, overlay.instance_url);
        merge_option(&mut self.ai_gateway_url, overlay.ai_gateway_url);
        self.ai_gateway_headers.extend(overlay.ai_gateway_headers);
        self.feature_flags.extend(overlay.feature_flags);
        self.models.extend(overlay.models);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProviderConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) default_adapter: Option<String>,
    pub(crate) default_model: Option<String>,
    pub(crate) auth: Option<RawProviderAuthConfig>,
    pub(crate) adapters: BTreeMap<String, RawProviderAdapterConfig>,
}

impl Merge for RawProviderConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.enabled, overlay.enabled);
        merge_option(&mut self.default_adapter, overlay.default_adapter);
        merge_option(&mut self.default_model, overlay.default_model);
        merge_option_struct(&mut self.auth, overlay.auth);
        self.adapters.extend(overlay.adapters);
    }
}

impl RawProviderConfig {
    fn resolve(
        self,
        provider_id: String,
        _env: &dyn ConfigEnvironment,
        defaults: &RawDefaultConfig,
    ) -> Result<(String, ResolvedProviderConfig), ConfigError> {
        let enabled = self.enabled.unwrap_or(true);
        if self.adapters.is_empty() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id,
                message: "provider must declare at least one adapter under `providers.<id>.adapters.<kind>`".to_owned(),
            });
        }

        let mut adapters = BTreeMap::new();
        let mut models = BTreeMap::new();
        for (adapter_id, mut adapter_raw) in self.adapters {
            normalize_model_configs(&mut adapter_raw.models);
            validate_configured_models(
                provider_id.as_str(),
                format!("adapter `{adapter_id}`").as_str(),
                &adapter_raw.models,
            )?;
            let adapter = resolve_adapter(provider_id.as_str(), adapter_id.as_str(), adapter_raw)?;
            for (model_id, configured) in &adapter.models {
                let route_id = format!("{adapter_id}/{model_id}");
                if models.contains_key(route_id.as_str()) {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.clone(),
                        message: format!("duplicate routed model `{route_id}` across adapters"),
                    });
                }
                models.insert(
                    route_id,
                    ResolvedProviderModelConfig {
                        enabled: configured.enabled,
                        definition: configured.definition.clone(),
                    },
                );
            }
            adapters.insert(adapter_id, adapter.config);
        }

        let auth = resolve_provider_auth(provider_id.as_str(), self.auth, adapters.values())?;
        validate_provider_auth(provider_id.as_str(), &auth, adapters.values())?;
        let default_adapter = match self.default_adapter {
            Some(default_adapter) => default_adapter,
            None => defaults
                .provider_default_adapter(provider_id.as_str())
                .or_else(|| {
                    let enabled_adapters = adapters
                        .iter()
                        .filter(|(_, adapter)| adapter.enabled)
                        .map(|(adapter_id, _)| adapter_id.clone())
                        .collect::<Vec<_>>();
                    (enabled_adapters.len() == 1).then(|| enabled_adapters[0].clone())
                })
                .ok_or_else(|| ConfigError::MissingProviderField {
                    provider_id: provider_id.clone(),
                    field: "default_adapter",
                })?,
        };
        if default_adapter.trim().is_empty() {
            return Err(ConfigError::MissingProviderField {
                provider_id: provider_id.clone(),
                field: "default_adapter",
            });
        }
        let default_model = match self.default_model {
            Some(default_model) => default_model,
            None => defaults
                .provider_default_model(provider_id.as_str())
                .ok_or_else(|| ConfigError::MissingProviderField {
                    provider_id: provider_id.clone(),
                    field: "default_model",
                })?,
        };
        if default_model.is_empty() {
            return Err(ConfigError::MissingProviderField {
                provider_id: provider_id.clone(),
                field: "default_model",
            });
        }
        let default_adapter_id = default_adapter.trim().to_owned();
        let default_adapter = adapters.get(default_adapter_id.as_str()).ok_or_else(|| {
            ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: format!(
                    "provider default_adapter `{default_adapter_id}` references unknown adapter"
                ),
            }
        })?;
        if !default_adapter.enabled {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: format!(
                    "provider default_adapter `{default_adapter_id}` references disabled adapter"
                ),
            });
        }
        let default_route = format!("{default_adapter_id}/{default_model}");
        if matches!(models.get(default_route.as_str()), Some(configured) if !configured.enabled) {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: format!(
                    "provider default_model `{default_model}` references disabled model route `{default_route}`"
                ),
            });
        }

        Ok((
            provider_id,
            ResolvedProviderConfig {
                enabled,
                default_adapter: default_adapter_id,
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

const DEFAULT_SAP_AI_CORE_SERVICE_KEY_ENV: &str = "AICORE_SERVICE_KEY";

fn normalize_model_configs(models: &mut BTreeMap<String, RawProviderModelConfig>) {
    for configured in models.values_mut() {
        configured.definition.capabilities.normalize_compact_patch();
    }
}

fn resolve_adapter(
    provider_id: &str,
    adapter_id: &str,
    raw: RawProviderAdapterConfig,
) -> Result<ResolvedAdapterWithModels, ConfigError> {
    let kind =
        ProviderKind::from_str(adapter_id).map_err(|_| ConfigError::MissingProviderKind {
            provider_id: provider_id.to_owned(),
        })?;
    let config = resolve_adapter_config(
        provider_id,
        adapter_id,
        kind,
        raw.backend,
        raw.enabled,
        raw.model_discovery,
        raw.base_url,
        raw.models_url,
        raw.capability_family,
        raw.messages_url,
        raw.auth_header,
        raw.auth_scheme,
        raw.extra_beta_header,
        raw.eager_input_streaming,
        raw.extra_headers,
        raw.api_mode,
        raw.stream_mode,
        raw.realtime_ws_url,
        raw.instance_url,
        raw.ai_gateway_url,
        raw.ai_gateway_headers,
        raw.feature_flags,
    )?;
    let models = raw
        .models
        .into_iter()
        .map(|(model_id, configured)| {
            Ok((
                model_id.clone(),
                ResolvedProviderModelConfig {
                    enabled: configured.enabled.unwrap_or(true),
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
    _adapter_id: &str,
    kind: ProviderKind,
    backend: Option<super::OpenAiBackendConfig>,
    enabled: Option<bool>,
    model_discovery: Option<ProviderModelDiscoveryConfig>,
    base_url: Option<String>,
    models_url: Option<String>,
    capability_family: Option<ProviderCapabilityFamilyConfig>,
    messages_url: Option<String>,
    auth_header: Option<String>,
    auth_scheme: Option<String>,
    extra_beta_header: Option<String>,
    eager_input_streaming: Option<bool>,
    extra_headers: BTreeMap<String, String>,
    api_mode: Option<OpenAiApiModeConfig>,
    stream_mode: Option<StreamTransportMode>,
    realtime_ws_url: Option<String>,
    instance_url: Option<String>,
    ai_gateway_url: Option<String>,
    ai_gateway_headers: BTreeMap<String, String>,
    feature_flags: BTreeMap<String, bool>,
) -> Result<ResolvedProviderAdapterConfig, ConfigError> {
    let definition = match kind {
        ProviderKind::Ollama => ProviderAdapterDefinition::Ollama(super::OllamaProviderOptions {
            base_url: normalize_optional(base_url),
        }),
        ProviderKind::OpenAi => {
            let backend = backend.unwrap_or_default();
            let api_mode_explicit = api_mode.is_some();
            let api_mode = api_mode.unwrap_or(OpenAiApiModeConfig::Responses);
            let stream_mode = stream_mode.unwrap_or(StreamTransportMode::Sse);
            let realtime_ws_url = normalize_optional(realtime_ws_url);
            if normalize_optional(base_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "openai adapter does not support `base_url`; configure provider auth endpoint instead"
                            .to_owned(),
                });
            }
            if matches!(backend, super::OpenAiBackendConfig::ChatgptCodex) {
                if api_mode != OpenAiApiModeConfig::Responses {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "openai backend `chatgpt_codex` only supports `api_mode = \"responses\"`".to_owned(),
                    });
                }
                if stream_mode != StreamTransportMode::Sse {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message:
                            "openai backend `chatgpt_codex` only supports `stream_mode = \"sse\"`"
                                .to_owned(),
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
            ProviderAdapterDefinition::OpenAi(HttpProviderAdapterConfig {
                extra_headers,
                options: super::OpenAiProviderOptions {
                    backend,
                    api_mode,
                    api_mode_explicit,
                    stream_mode,
                    realtime_ws_url,
                    models_url: normalize_optional(models_url),
                    auth_header: auth_header.unwrap_or_else(|| "authorization".to_owned()),
                    auth_scheme: normalize_optional(auth_scheme)
                        .or_else(|| Some("Bearer".to_owned())),
                    capability_family,
                },
            })
        }
        ProviderKind::Anthropic => {
            if normalize_optional(base_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "anthropic adapter does not support `base_url`; configure provider auth endpoint instead"
                            .to_owned(),
                });
            }
            ProviderAdapterDefinition::Anthropic(HttpProviderAdapterConfig {
                extra_headers,
                options: super::AnthropicProviderOptions {
                    models_url: normalize_optional(models_url),
                    messages_url: normalize_optional(messages_url),
                    auth_header: auth_header.unwrap_or_else(|| "x-api-key".to_owned()),
                    auth_scheme: normalize_optional(auth_scheme),
                    extra_beta_header: normalize_optional(extra_beta_header),
                    eager_input_streaming,
                },
            })
        }
        ProviderKind::Gemini => {
            if normalize_optional(base_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "gemini adapter does not support `base_url`; configure provider auth endpoint instead"
                            .to_owned(),
                });
            }
            ProviderAdapterDefinition::Gemini(HttpProviderAdapterConfig {
                extra_headers,
                options: super::SimpleHttpProviderOptions {
                    auth_header: normalize_optional(auth_header),
                    auth_scheme: normalize_optional(auth_scheme),
                },
            })
        }
        ProviderKind::Gitlab => ProviderAdapterDefinition::Gitlab(super::GitlabProviderOptions {
            instance_url: normalize_optional(instance_url),
            ai_gateway_url: normalize_optional(ai_gateway_url),
            ai_gateway_headers,
            feature_flags,
        }),
        ProviderKind::AmazonBedrock => {
            ProviderAdapterDefinition::AmazonBedrock(super::AmazonBedrockProviderOptions)
        }
    };

    Ok(ResolvedProviderAdapterConfig {
        enabled: enabled.unwrap_or(false),
        model_discovery: model_discovery.unwrap_or_default(),
        definition,
    })
}

fn resolve_provider_auth<'a>(
    provider_id: &str,
    raw_auth: Option<RawProviderAuthConfig>,
    adapters: impl IntoIterator<Item = &'a ResolvedProviderAdapterConfig>,
) -> Result<ProviderAuthConfig, ConfigError> {
    let adapters = adapters.into_iter().collect::<Vec<_>>();
    let raw_auth = raw_auth.unwrap_or_default();
    let mode = raw_auth
        .mode
        .unwrap_or_else(|| infer_provider_auth_mode(&raw_auth, &adapters));
    match mode {
        ProviderAuthMode::None => Ok(ProviderAuthConfig::None),
        ProviderAuthMode::Api => {
            let base_url = required_string(provider_id, "base_url", raw_auth.base_url)?;
            let has_explicit_protocol_paths = raw_auth.protocol_paths.is_some();
            if raw_auth.credential.is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "auth mode `api` does not accept `credential`".to_owned(),
                });
            }
            let protocol_paths =
                resolve_protocol_paths(provider_id, raw_auth.protocol_paths, "protocol_paths")?;
            Ok(ProviderAuthConfig::Api(ProviderApiAuthConfig {
                base_url: if has_explicit_protocol_paths {
                    base_url
                } else {
                    strip_default_protocol_path_from_base_url(base_url)
                },
                protocol_paths,
                api_key: normalize_optional(raw_auth.api_key),
                api_key_env: normalize_optional(raw_auth.api_key_env),
            }))
        }
        ProviderAuthMode::Credential => {
            if normalize_optional(raw_auth.base_url.clone()).is_some()
                || raw_auth.protocol_paths.is_some()
                || normalize_optional(raw_auth.api_key.clone()).is_some()
                || normalize_optional(raw_auth.api_key_env.clone()).is_some()
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "auth mode `credential` does not accept `base_url`, `protocol_paths`, `api_key`, or `api_key_env`"
                            .to_owned(),
                });
            }
            let issuer = raw_auth
                .issuer
                .ok_or_else(|| ConfigError::MissingProviderField {
                    provider_id: provider_id.to_owned(),
                    field: "issuer",
                })?;
            let credential = raw_auth
                .credential
                .map(|credential| credential.with_issuer(issuer));
            Ok(ProviderAuthConfig::Credential(
                ProviderCredentialAuthConfig { issuer, credential },
            ))
        }
        ProviderAuthMode::BedrockSigv4 => {
            let access_key_id = normalize_optional(raw_auth.access_key_id);
            let secret_access_key = normalize_optional(raw_auth.secret_access_key);
            if access_key_id.is_some() ^ secret_access_key.is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "access_key_id and secret_access_key must be set together".to_owned(),
                });
            }
            Ok(ProviderAuthConfig::BedrockSigv4(BedrockSigv4AuthConfig {
                base_url: normalize_optional(raw_auth.base_url).unwrap_or_else(|| {
                    "https://bedrock-runtime.us-east-1.amazonaws.com".to_owned()
                }),
                region: normalize_optional(raw_auth.region)
                    .unwrap_or_else(|| "us-east-1".to_owned()),
                profile: normalize_optional(raw_auth.profile),
                access_key_id,
                secret_access_key,
                session_token: normalize_optional(raw_auth.session_token),
            }))
        }
        ProviderAuthMode::GoogleAdc => {
            Ok(ProviderAuthConfig::GoogleAdc(ProviderGoogleAdcAuthConfig {
                base_url: required_string(provider_id, "base_url", raw_auth.base_url)?,
                protocol_paths: resolve_protocol_paths(
                    provider_id,
                    raw_auth.protocol_paths,
                    "protocol_paths",
                )?,
            }))
        }
        ProviderAuthMode::SapAiCore => {
            Ok(ProviderAuthConfig::SapAiCore(ProviderSapAiCoreAuthConfig {
                api: ProviderApiAuthConfig {
                    base_url: required_string(provider_id, "base_url", raw_auth.base_url)?,
                    protocol_paths: resolve_protocol_paths(
                        provider_id,
                        raw_auth.protocol_paths,
                        "protocol_paths",
                    )?,
                    api_key: normalize_optional(raw_auth.api_key),
                    api_key_env: normalize_optional(raw_auth.api_key_env),
                },
                service_key_env: normalize_optional(raw_auth.service_key_env)
                    .unwrap_or_else(|| DEFAULT_SAP_AI_CORE_SERVICE_KEY_ENV.to_owned()),
            }))
        }
    }
}

fn resolve_protocol_paths(
    provider_id: &str,
    raw: Option<RawProviderProtocolPathsConfig>,
    field: &str,
) -> Result<ProviderProtocolPathsConfig, ConfigError> {
    let raw = raw.unwrap_or_default();
    Ok(ProviderProtocolPathsConfig {
        openai: normalize_protocol_path(
            provider_id,
            format!("{field}.openai").as_str(),
            raw.openai.unwrap_or_else(|| "/v1".to_owned()),
        )?,
        anthropic: normalize_protocol_path(
            provider_id,
            format!("{field}.anthropic").as_str(),
            raw.anthropic.unwrap_or_else(|| "/v1".to_owned()),
        )?,
        gemini: normalize_protocol_path(
            provider_id,
            format!("{field}.gemini").as_str(),
            raw.gemini.unwrap_or_else(|| "/v1beta".to_owned()),
        )?,
    })
}

fn normalize_protocol_path(
    provider_id: &str,
    field: &str,
    value: String,
) -> Result<String, ConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "/" {
        return Ok(String::new());
    }
    if trimmed.contains("://") || trimmed.contains('?') || trimmed.contains('#') {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("provider auth {field} must be a relative path, got `{trimmed}`"),
        });
    }
    Ok(format!("/{}", trimmed.trim_matches('/')))
}

fn infer_provider_auth_mode(
    raw_auth: &RawProviderAuthConfig,
    adapters: &[&ResolvedProviderAdapterConfig],
) -> ProviderAuthMode {
    if raw_auth.credential.is_some() || raw_auth.issuer.is_some() {
        return ProviderAuthMode::Credential;
    }
    if raw_auth.base_url.is_some()
        || raw_auth.protocol_paths.is_some()
        || raw_auth.api_key.is_some()
        || raw_auth.api_key_env.is_some()
    {
        return ProviderAuthMode::Api;
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
            &adapter.definition,
            ProviderAdapterDefinition::OpenAi(config)
                if matches!(
                    config.options.capability_family,
                    Some(ProviderCapabilityFamilyConfig::Gemini)
                )
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
    ProviderAuthMode::Api
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
            (ProviderAuthConfig::GoogleAdc(_), ProviderAdapterDefinition::OpenAi(config))
                if matches!(
                    config.options.capability_family,
                    Some(ProviderCapabilityFamilyConfig::Gemini)
                ) => {}
            (ProviderAuthConfig::GoogleAdc(_), _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "auth mode `google_adc` only supports Vertex-style `openai` adapters"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::BedrockSigv4(_), ProviderAdapterDefinition::AmazonBedrock(_)) => {}
            (ProviderAuthConfig::BedrockSigv4(_), _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "auth mode `bedrock_sigv4` only supports `amazon_bedrock` adapters"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::SapAiCore(_), ProviderAdapterDefinition::OpenAi(_)) => {}
            (ProviderAuthConfig::SapAiCore(_), _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "auth mode `sap_ai_core` only supports `openai` adapters".to_owned(),
                });
            }
            (ProviderAuthConfig::Api(_), ProviderAdapterDefinition::Ollama(_)) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "api auth is not supported by `ollama` adapters".to_owned(),
                });
            }
            (ProviderAuthConfig::Api(_), ProviderAdapterDefinition::OpenAi(config))
                if matches!(
                    config.options.backend,
                    super::OpenAiBackendConfig::ChatgptCodex
                ) =>
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "openai backend `chatgpt_codex` only supports credential auth"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::Api(_), _) => {}
            (
                ProviderAuthConfig::Credential(config),
                ProviderAdapterDefinition::OpenAi(options),
            ) => match (config.issuer, options.options.backend) {
                (CredentialIssuer::OpenaiChatgpt, super::OpenAiBackendConfig::ChatgptCodex) => {}
                (CredentialIssuer::GithubCopilot, super::OpenAiBackendConfig::Api) => {}
                (CredentialIssuer::AtomGit, super::OpenAiBackendConfig::Api) => {}
                _ => {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "credential issuer does not match `openai` adapter requirements"
                            .to_owned(),
                    });
                }
            },
            (ProviderAuthConfig::Credential(config), ProviderAdapterDefinition::Anthropic(_)) => {
                if config.issuer != CredentialIssuer::GithubCopilot {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message:
                            "credential issuer does not match `anthropic` adapter requirements"
                                .to_owned(),
                    });
                }
            }
            (ProviderAuthConfig::Credential(config), ProviderAdapterDefinition::Gemini(_))
                if config.issuer == CredentialIssuer::GithubCopilot =>
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "github_copilot credential does not support `gemini` adapter; use `openai` for Copilot Gemini models"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::Credential(config), ProviderAdapterDefinition::Gitlab(_)) => {
                if config.issuer != CredentialIssuer::Gitlab {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "credential issuer does not match `gitlab` adapter requirements"
                            .to_owned(),
                    });
                }
            }
            (ProviderAuthConfig::Credential(_), _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "credential auth is not supported by this adapter".to_owned(),
                });
            }
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
        if let Err(message) = configured.definition.capabilities.validate() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {scope} model `{model_id}` has invalid capability patch: {message}"
            )));
        }
        validate_configured_modes(
            provider_id,
            format!("{scope} model `{model_id}` thinking modes").as_str(),
            &configured.definition.thinking_modes,
            format!("{scope} model `{model_id}` speed modes").as_str(),
            &configured.definition.speed_modes,
        )?;
    }
    Ok(())
}

fn validate_configured_modes(
    provider_id: &str,
    thinking_scope: &str,
    thinking_modes: &BTreeMap<String, ConfiguredModelThinkingMode>,
    speed_scope: &str,
    speed_modes: &BTreeMap<String, ConfiguredModelSpeedMode>,
) -> Result<(), ConfigError> {
    for (name, mode) in thinking_modes {
        if name.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {thinking_scope} mode name cannot be empty"
            )));
        }
        if mode.is_empty() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {thinking_scope} mode `{name}` must set at least one field or disabled = true"
            )));
        }
    }
    for (name, mode) in speed_modes {
        if name.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {speed_scope} mode name cannot be empty"
            )));
        }
        if mode.is_empty() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {speed_scope} mode `{name}` must set at least one field or disabled = true"
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

fn strip_default_protocol_path_from_base_url(value: String) -> String {
    let trimmed = value.trim_end_matches('/');
    if let Ok(mut url) = url::Url::parse(trimmed) {
        match url.path().trim_end_matches('/') {
            "/v1" | "/v1beta" => {
                url.set_path("");
                return url.to_string().trim_end_matches('/').to_owned();
            }
            _ => {}
        }
    }
    value
}

pub(crate) fn parse_adapter_model_ref(
    provider_id: &str,
    value: &str,
) -> Result<(String, String), ConfigError> {
    let Some((adapter_id, model_id)) = value.split_once('/') else {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("model reference `{value}` must be in `<adapter>/<model>` format"),
        });
    };

    let adapter_id = adapter_id.trim();
    let model_id = model_id.trim();
    if adapter_id.is_empty() || model_id.is_empty() {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!("model reference `{value}` must be in `<adapter>/<model>` format"),
        });
    }

    Ok((adapter_id.to_owned(), model_id.to_owned()))
}

fn reject_legacy_provider_env_overrides(env: &dyn ConfigEnvironment) -> Result<(), ConfigError> {
    if let Some((key, _)) = env
        .vars()
        .into_iter()
        .find(|(key, _)| key.starts_with("AGENA_PROVIDER__"))
    {
        return Err(ConfigError::Validation(format!(
            "{key} is no longer supported; use canonical config files or `--set providers.<id>.auth.*` overrides"
        )));
    }
    Ok(())
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
