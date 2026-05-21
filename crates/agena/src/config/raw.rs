use std::{collections::BTreeMap, fs, path::Path, str::FromStr};

use merge::Merge as DeriveMerge;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::{
    ConfiguredModelSpeedMode, ConfiguredModelThinkingMode, ProviderRequestRetryConfig,
    ProviderStreamReplayConfig, auth::CredentialIssuer,
};

use super::{
    AgentConfig, BedrockSigv4AuthConfig, ConfigEnvironment, ConfigError, DefaultConfig,
    HttpProviderAdapterConfig, McpConfig, MemoryConfig, OpenAiApiModeConfig, PluginConfig,
    ProjectInstructionsConfig, ProviderAdapterDefinition, ProviderAdapterOverlay,
    ProviderApiAuthConfig, ProviderAuthConfig, ProviderAuthMode, ProviderAuthOverlay,
    ProviderCapabilityFamilyConfig, ProviderCredentialAuthConfig, ProviderGitlabAuthConfig,
    ProviderModelDiscoveryConfig, ProviderModelOverlay, ProviderOverlay,
    ProviderProtocolPathsConfig, ProviderProtocolPathsOverlay, ResolvedConfig,
    ResolvedProviderAdapterConfig, ResolvedProviderConfig, ResolvedProviderModelConfig,
    RuntimeConfig, RuntimeModelCatalogConfig, StreamTransportMode, TelemetryConfig, TracingConfig,
    UiConfig, WebToolsConfig,
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
                let config = serde_json::from_str::<RawConfig>(&text).map_err(|source| {
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
    let config =
        serde_json::from_str::<RawConfig>(text).map_err(|source| ConfigError::ParseFile {
            path: path.to_path_buf(),
            source,
        })?;
    config.resolve_with_env(env)?;
    Ok(())
}

fn reject_unsupported_fields(path: &Path, text: &str) -> Result<(), ConfigError> {
    let value = serde_json::from_str::<Value>(text).map_err(|source| ConfigError::ParseFile {
        path: path.to_path_buf(),
        source,
    })?;
    let Some(table) = value.as_object() else {
        return Ok(());
    };
    if table.contains_key("mode") {
        return Err(ConfigError::UnsupportedModeConfig { field: "mode" });
    }
    if table.contains_key("modes") {
        return Err(ConfigError::UnsupportedModeConfig { field: "modes" });
    }
    for field in ["memory", "mcp", "lsp", "web"] {
        if table.contains_key(field) {
            return Err(ConfigError::Validation(format!(
                "`{field}` must be configured as plugin options under `[plugins.list.\"agena.{field}\".options]`"
            )));
        }
    }
    if table.contains_key("hooks") {
        return Err(ConfigError::Validation(
            "`hooks` has been removed; implement hook behavior as a regular plugin under `[plugins.list.<id>]`".to_string(),
        ));
    }
    if let Some(providers) = table.get("providers").and_then(Value::as_object) {
        for (provider_id, provider) in providers {
            let Some(provider) = provider.as_object() else {
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
            if let Some(adapters) = provider.get("adapters").and_then(Value::as_object) {
                for (adapter_id, adapter) in adapters {
                    let Some(adapter) = adapter.as_object() else {
                        continue;
                    };
                    if let Some(models) = adapter.get("models").and_then(Value::as_object) {
                        for (model_id, model) in models {
                            let Some(model) = model.as_object() else {
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
    pub(crate) providers: BTreeMap<String, ProviderOverlay>,
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
                .database = Some(level);
        }
        if let Some(level) = env.var("AGENA_ADAPTER_LOG") {
            config
                .tracing
                .get_or_insert_with(RawTracingConfig::default)
                .adapter = Some(level);
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
        let database = raw_tracing
            .database
            .unwrap_or_else(|| DEFAULT_DATABASE_LOG_LEVEL.to_owned());
        validate_database_log_level(database.as_str())?;
        let adapter = raw_tracing
            .adapter
            .unwrap_or_else(|| TracingConfig::default().adapter);
        validate_tracing_level("tracing.adapter", adapter.as_str())?;
        let tracing = TracingConfig {
            filter: raw_tracing
                .filter
                .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_owned()),
            database,
            adapter,
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

    let adapter_id = match default.adapter.as_deref() {
        Some(adapter_id) => {
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
            Some(adapter_id)
        }
        None if default.model.is_some() => Some(provider.default_adapter.as_str()),
        None => None,
    };

    if let (Some(adapter_id), Some(model_id)) = (adapter_id, default.model.as_deref()) {
        let route = format!("{adapter_id}/{model_id}");
        if matches!(provider.models.get(route.as_str()), Some(configured) if !configured.enabled) {
            return Err(ConfigError::Validation(format!(
                "default.model `{model_id}` references disabled model route `{route}` on provider `{provider_id}`"
            )));
        }
    }

    Ok(())
}

#[derive(Debug, Default)]
struct PluginRuntimeOptions {
    memory: Option<MemoryConfig>,
    mcp: Option<McpConfig>,
    lsp: Option<crate::config::types::LspConfig>,
    web: Option<WebToolsConfig>,
}

impl PluginRuntimeOptions {
    fn from_plugins(plugins: &PluginConfig) -> Result<Self, ConfigError> {
        let mut out = Self::default();
        for (plugin_id, entry) in &plugins.list {
            if plugin_id == "agena.hooks" {
                return Err(ConfigError::Validation(
                    "`plugins.list.\"agena.hooks\"` has been removed; implement hook behavior as a regular plugin under `[plugins.list.<id>]`".to_string(),
                ));
            }
            let options = entry.options();
            if options.is_null() {
                continue;
            }
            match plugin_id.as_str() {
                "agena.memory" => {
                    out.memory = Some(parse_plugin_options(plugin_id, options.clone())?);
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::validate_config_text;
    use crate::config::{ConfigError, ProcessEnvironment};

    #[test]
    fn validate_config_text_rejects_removed_top_level_hooks() {
        let err = validate_config_text(
            Path::new("config.json"),
            r#"{"hooks":[{"event":"user_prompt_submit","command":"true"}]}"#,
            &ProcessEnvironment,
        )
        .expect_err("top-level hooks should be rejected");

        match err {
            ConfigError::Validation(message) => {
                assert!(message.contains("`hooks` has been removed"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn validate_config_text_rejects_removed_agena_hooks_plugin() {
        let err = validate_config_text(
            Path::new("config.json"),
            r#"{"plugins":{"list":{"agena.hooks":{"kind":"static","options":{"hooks":[]}}}}}"#,
            &ProcessEnvironment,
        )
        .expect_err("agena.hooks plugin should be rejected");

        match err {
            ConfigError::Validation(message) => {
                assert!(message.contains("`plugins.list.\"agena.hooks\"` has been removed"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[allow(dead_code)]
pub(crate) struct RawPluginConfig;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawTracingConfig {
    #[merge(strategy = option_override)]
    pub(crate) filter: Option<String>,
    #[serde(alias = "database_level")]
    #[merge(strategy = option_override)]
    pub(crate) database: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) adapter: Option<String>,
}

fn validate_database_log_level(value: &str) -> Result<(), ConfigError> {
    validate_tracing_level("tracing.database", value)
}

fn validate_tracing_level(field: &str, value: &str) -> Result<(), ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "error" | "warn" | "info" | "debug" | "trace" => Ok(()),
        _ => Err(ConfigError::Validation(format!(
            "{field} expects one of off,error,warn,info,debug,trace, got `{value}`"
        ))),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawTelemetryConfig {
    #[merge(strategy = option_override)]
    pub(crate) enabled: Option<bool>,
    #[merge(strategy = option_override)]
    pub(crate) service_name: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) otlp_endpoint: Option<String>,
    #[serde(default)]
    #[merge(strategy = map_extend)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawUiConfig {
    #[merge(strategy = option_override)]
    pub(crate) locale: Option<String>,
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
        merge_option(&mut self.default.provider, overlay.default.provider);
        merge_option(&mut self.default.adapter, overlay.default.adapter);
        merge_option(&mut self.default.model, overlay.default.model);
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawDefaultConfig {
    #[merge(strategy = option_override)]
    pub(crate) provider: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) adapter: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) model: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) agent: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawRuntimeConfig {
    #[merge(strategy = option_struct_merge)]
    pub(crate) provider_http: Option<RawProviderHttpConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) request_retry: Option<RawRequestRetryConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) stream_replay: Option<RawStreamReplayConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) model_catalog: Option<RawRuntimeModelCatalogConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) reload: Option<RawRuntimeReloadConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) janitor: Option<RawRuntimeJanitorConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) session_cache: Option<RawSessionCacheConfig>,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawProviderHttpConfig {
    #[merge(strategy = option_override)]
    pub(crate) timeout_secs: Option<u64>,
    #[merge(strategy = option_override)]
    pub(crate) connect_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawRequestRetryConfig {
    #[merge(strategy = option_override)]
    pub(crate) max_retries: Option<u32>,
    #[merge(strategy = option_override)]
    pub(crate) base_delay_ms: Option<u64>,
    #[merge(strategy = option_override)]
    pub(crate) max_delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawStreamReplayConfig {
    #[merge(strategy = option_override)]
    pub(crate) max_retries_after_output: Option<u32>,
    #[merge(strategy = option_override)]
    pub(crate) max_tracked_events: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawRuntimeModelCatalogConfig {
    #[merge(strategy = option_override)]
    pub(crate) cache_max_age_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawRuntimeReloadConfig {
    #[merge(strategy = option_override)]
    pub(crate) enabled: Option<bool>,
    #[merge(strategy = option_override)]
    pub(crate) poll_interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawRuntimeJanitorConfig {
    #[merge(strategy = option_override)]
    pub(crate) enabled: Option<bool>,
    #[merge(strategy = option_override)]
    pub(crate) interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawSessionCacheConfig {
    #[merge(strategy = option_override)]
    pub(crate) max_sessions: Option<usize>,
    #[merge(strategy = option_override)]
    pub(crate) ttl_secs: Option<u64>,
    #[merge(strategy = option_override)]
    pub(crate) max_bytes: Option<usize>,
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

impl ProviderOverlay {
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

fn normalize_model_configs(models: &mut BTreeMap<String, ProviderModelOverlay>) {
    for configured in models.values_mut() {
        configured.definition.capabilities.normalize_compact_patch();
    }
}

fn resolve_adapter(
    provider_id: &str,
    adapter_id: &str,
    raw: ProviderAdapterOverlay,
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
        raw.user_agent,
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
        .map(|(model_id, configured)| Ok((model_id.clone(), configured)))
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
    user_agent: Option<String>,
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
                user_agent: normalize_optional(user_agent),
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
                user_agent: normalize_optional(user_agent),
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
            let stream_mode = stream_mode.unwrap_or(StreamTransportMode::Sse);
            let realtime_ws_url = normalize_optional(realtime_ws_url);
            if normalize_optional(base_url.clone()).is_some() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "gemini adapter does not support `base_url`; configure provider auth endpoint instead"
                            .to_owned(),
                });
            }
            ProviderAdapterDefinition::Gemini(HttpProviderAdapterConfig {
                user_agent: normalize_optional(user_agent),
                extra_headers,
                options: super::GeminiProviderOptions {
                    auth_header: normalize_optional(auth_header),
                    auth_scheme: normalize_optional(auth_scheme),
                    stream_mode,
                    realtime_ws_url,
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
    raw_auth: Option<ProviderAuthOverlay>,
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
                base_url: normalize_optional(raw_auth.base_url).map(|base_url| {
                    if has_explicit_protocol_paths {
                        base_url
                    } else {
                        strip_default_protocol_path_from_base_url(base_url)
                    }
                }),
                protocol_paths,
                api_key: normalize_optional(raw_auth.api_key),
                api_key_env: normalize_optional(raw_auth.api_key_env),
            }))
        }
        ProviderAuthMode::Gitlab => resolve_gitlab_auth(provider_id, raw_auth),
        ProviderAuthMode::Credential => {
            let issuer = raw_auth
                .issuer
                .ok_or_else(|| ConfigError::MissingProviderField {
                    provider_id: provider_id.to_owned(),
                    field: "issuer",
                })?;
            resolve_credential_auth(provider_id, raw_auth, issuer)
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
            resolve_credential_auth(provider_id, raw_auth, CredentialIssuer::GoogleAdc)
        }
        ProviderAuthMode::SapAiCore => {
            if normalize_optional(raw_auth.api_key.clone()).is_some()
                || normalize_optional(raw_auth.api_key_env.clone()).is_some()
            {
                let has_explicit_protocol_paths = raw_auth.protocol_paths.is_some();
                return Ok(ProviderAuthConfig::Api(ProviderApiAuthConfig {
                    base_url: normalize_optional(raw_auth.base_url).map(|base_url| {
                        if has_explicit_protocol_paths {
                            base_url
                        } else {
                            strip_default_protocol_path_from_base_url(base_url)
                        }
                    }),
                    protocol_paths: resolve_protocol_paths(
                        provider_id,
                        raw_auth.protocol_paths,
                        "protocol_paths",
                    )?,
                    api_key: normalize_optional(raw_auth.api_key),
                    api_key_env: normalize_optional(raw_auth.api_key_env),
                }));
            }

            resolve_credential_auth(provider_id, raw_auth, CredentialIssuer::SapAiCore)
        }
    }
}

fn resolve_credential_auth(
    provider_id: &str,
    raw_auth: ProviderAuthOverlay,
    issuer: CredentialIssuer,
) -> Result<ProviderAuthConfig, ConfigError> {
    let credential = raw_auth
        .credential
        .clone()
        .map(|credential| credential.with_issuer(issuer));
    let base_url = normalize_optional(raw_auth.base_url.clone());
    let api_key = normalize_optional(raw_auth.api_key.clone());
    let api_key_env = normalize_optional(raw_auth.api_key_env.clone());
    let service_key_env = normalize_optional(raw_auth.service_key_env.clone());
    let instance_url = normalize_optional(raw_auth.instance_url.clone());
    let ai_gateway_url = normalize_optional(raw_auth.ai_gateway_url.clone());

    if issuer.uses_http_endpoint() {
        if api_key.is_some() || api_key_env.is_some() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "credential issuer `{}` does not accept `api_key` or `api_key_env`; use auth mode `api` for direct tokens",
                    issuer_label(issuer)
                ),
            });
        }
        if credential.is_some() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "credential issuer `{}` does not accept inline `credential` data",
                    issuer_label(issuer)
                ),
            });
        }
        let base_url = required_string(provider_id, "base_url", raw_auth.base_url)?;
        let protocol_paths =
            resolve_protocol_paths(provider_id, raw_auth.protocol_paths, "protocol_paths")?;
        return Ok(ProviderAuthConfig::Credential(
            ProviderCredentialAuthConfig {
                issuer,
                credential: None,
                base_url: Some(base_url),
                protocol_paths,
                service_key_env: if issuer.requires_service_key_env() {
                    Some(
                        service_key_env
                            .unwrap_or_else(|| DEFAULT_SAP_AI_CORE_SERVICE_KEY_ENV.to_owned()),
                    )
                } else {
                    if service_key_env.is_some() {
                        return Err(ConfigError::InvalidProviderConfig {
                            provider_id: provider_id.to_owned(),
                            message: format!(
                                "credential issuer `{}` does not accept `service_key_env`",
                                issuer_label(issuer)
                            ),
                        });
                    }
                    None
                },
                instance_url: None,
                ai_gateway_url: None,
                ai_gateway_headers: BTreeMap::new(),
                feature_flags: BTreeMap::new(),
            },
        ));
    }

    if issuer == CredentialIssuer::Gitlab {
        if base_url.is_some()
            || raw_auth.protocol_paths.is_some()
            || api_key.is_some()
            || api_key_env.is_some()
            || service_key_env.is_some()
        {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message:
                    "credential issuer `gitlab` does not accept `base_url`, `protocol_paths`, `api_key`, `api_key_env`, or `service_key_env`"
                        .to_owned(),
            });
        }

        return Ok(ProviderAuthConfig::Credential(
            ProviderCredentialAuthConfig {
                issuer,
                credential,
                base_url: None,
                protocol_paths: ProviderProtocolPathsConfig::default(),
                service_key_env: None,
                instance_url,
                ai_gateway_url,
                ai_gateway_headers: raw_auth.ai_gateway_headers,
                feature_flags: raw_auth.feature_flags,
            },
        ));
    }

    if base_url.is_some()
        || raw_auth.protocol_paths.is_some()
        || api_key.is_some()
        || api_key_env.is_some()
        || instance_url.is_some()
        || ai_gateway_url.is_some()
        || !raw_auth.ai_gateway_headers.is_empty()
        || !raw_auth.feature_flags.is_empty()
        || service_key_env.is_some()
    {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message:
                "auth mode `credential` does not accept `base_url`, `protocol_paths`, `api_key`, `api_key_env`, `instance_url`, `ai_gateway_url`, `ai_gateway_headers`, `feature_flags`, or `service_key_env` for this issuer"
                    .to_owned(),
        });
    }

    Ok(ProviderAuthConfig::Credential(
        ProviderCredentialAuthConfig {
            issuer,
            credential,
            base_url: None,
            protocol_paths: ProviderProtocolPathsConfig::default(),
            service_key_env: None,
            instance_url: None,
            ai_gateway_url: None,
            ai_gateway_headers: BTreeMap::new(),
            feature_flags: BTreeMap::new(),
        },
    ))
}

fn resolve_gitlab_auth(
    provider_id: &str,
    raw_auth: ProviderAuthOverlay,
) -> Result<ProviderAuthConfig, ConfigError> {
    if raw_auth.base_url.is_some()
        || raw_auth.protocol_paths.is_some()
        || raw_auth.profile.is_some()
        || raw_auth.access_key_id.is_some()
        || raw_auth.secret_access_key.is_some()
        || raw_auth.session_token.is_some()
        || raw_auth.region.is_some()
        || raw_auth.service_key_env.is_some()
    {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "auth mode `gitlab_api` does not accept `base_url`, `protocol_paths`, `profile`, `access_key_id`, `secret_access_key`, `session_token`, `region`, or `service_key_env`".to_owned(),
        });
    }

    let api_key = normalize_optional(raw_auth.api_key);
    let api_key_env = normalize_optional(raw_auth.api_key_env);
    let credential = raw_auth
        .credential
        .map(|credential| credential.with_issuer(CredentialIssuer::Gitlab));

    if api_key.is_none() && api_key_env.is_none() && credential.is_none() {
        return Err(ConfigError::MissingProviderField {
            provider_id: provider_id.to_owned(),
            field: "api_key",
        });
    }

    Ok(ProviderAuthConfig::Gitlab(ProviderGitlabAuthConfig {
        api_key,
        api_key_env,
        credential,
        instance_url: normalize_optional(raw_auth.instance_url),
        ai_gateway_url: normalize_optional(raw_auth.ai_gateway_url),
        ai_gateway_headers: raw_auth.ai_gateway_headers,
        feature_flags: raw_auth.feature_flags,
    }))
}

fn resolve_protocol_paths(
    provider_id: &str,
    raw: Option<ProviderProtocolPathsOverlay>,
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
    raw_auth: &ProviderAuthOverlay,
    adapters: &[&ResolvedProviderAdapterConfig],
) -> ProviderAuthMode {
    if raw_auth.credential.is_some() || raw_auth.issuer.is_some() {
        return ProviderAuthMode::Credential;
    }
    if raw_auth.instance_url.is_some()
        || raw_auth.ai_gateway_url.is_some()
        || !raw_auth.ai_gateway_headers.is_empty()
        || !raw_auth.feature_flags.is_empty()
    {
        return ProviderAuthMode::Gitlab;
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
    }) && raw_auth.api_key.is_none()
        && raw_auth.api_key_env.is_none()
    {
        return ProviderAuthMode::GoogleAdc;
    }
    if raw_auth.base_url.is_some()
        || raw_auth.protocol_paths.is_some()
        || raw_auth.api_key.is_some()
        || raw_auth.api_key_env.is_some()
    {
        return ProviderAuthMode::Api;
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
            (ProviderAuthConfig::BedrockSigv4(_), ProviderAdapterDefinition::AmazonBedrock(_)) => {}
            (ProviderAuthConfig::BedrockSigv4(_), _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "auth mode `bedrock_sigv4` only supports `amazon_bedrock` adapters"
                        .to_owned(),
                });
            }
            (ProviderAuthConfig::Api(_), ProviderAdapterDefinition::Ollama(_)) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "api auth is not supported by `ollama` adapters".to_owned(),
                });
            }
            (ProviderAuthConfig::Api(_), ProviderAdapterDefinition::AmazonBedrock(_)) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "api auth is not supported by `amazon_bedrock` adapters".to_owned(),
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
            (ProviderAuthConfig::Api(api), definition) => {
                if api_auth_requires_base_url(definition) && api.base_url.is_none() {
                    let adapter_label = match definition {
                        ProviderAdapterDefinition::OpenAi(_) => "openai",
                        ProviderAdapterDefinition::Anthropic(_) => "anthropic",
                        ProviderAdapterDefinition::Gemini(_) => "gemini",
                        ProviderAdapterDefinition::Gitlab(_) => "gitlab",
                        ProviderAdapterDefinition::Ollama(_) => "ollama",
                        ProviderAdapterDefinition::AmazonBedrock(_) => "amazon_bedrock",
                    };
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: format!(
                            "api auth requires `base_url` for `{adapter_label}` adapters"
                        ),
                    });
                }
            }
            (ProviderAuthConfig::Gitlab(_), ProviderAdapterDefinition::OpenAi(config))
                if matches!(config.options.backend, super::OpenAiBackendConfig::Api) => {}
            (ProviderAuthConfig::Gitlab(_), ProviderAdapterDefinition::OpenAi(_)) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "auth mode `gitlab_api` only supports `openai` adapters with backend `api`"
                            .to_owned(),
                });
            }
            (ProviderAuthConfig::Gitlab(_), ProviderAdapterDefinition::Anthropic(_))
            | (ProviderAuthConfig::Gitlab(_), ProviderAdapterDefinition::Gitlab(_)) => {}
            (ProviderAuthConfig::Gitlab(_), _) => {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "auth mode `gitlab_api` only supports `openai` or `anthropic` adapters"
                            .to_owned(),
                });
            }
            (
                ProviderAuthConfig::Credential(config),
                ProviderAdapterDefinition::OpenAi(options),
            ) => match (config.issuer, options.options.backend) {
                (CredentialIssuer::OpenaiChatgpt, super::OpenAiBackendConfig::ChatgptCodex) => {}
                (CredentialIssuer::GithubCopilot, super::OpenAiBackendConfig::Api) => {}
                (CredentialIssuer::Gitlab, super::OpenAiBackendConfig::Api) => {}
                (CredentialIssuer::GoogleAdc, _)
                    if matches!(
                        options.options.capability_family,
                        Some(ProviderCapabilityFamilyConfig::Gemini)
                    ) => {}
                (CredentialIssuer::SapAiCore, _) => {}
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
                if !matches!(
                    config.issuer,
                    CredentialIssuer::GithubCopilot | CredentialIssuer::Gitlab
                ) {
                    return Err(ConfigError::InvalidProviderConfig {
                        provider_id: provider_id.to_owned(),
                        message: "credential issuer does not match `anthropic` adapter requirements; use `api` auth with a Claude Console API key for first-party Anthropic access"
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
            (ProviderAuthConfig::Credential(config), ProviderAdapterDefinition::Gitlab(_))
                if config.issuer == CredentialIssuer::Gitlab => {}
            (ProviderAuthConfig::Credential(config), _)
                if config.issuer == CredentialIssuer::GoogleAdc =>
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message:
                        "credential issuer `google_adc` only supports Vertex-style `openai` adapters"
                            .to_owned(),
                });
            }
            (ProviderAuthConfig::Credential(config), _)
                if config.issuer == CredentialIssuer::SapAiCore =>
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: "credential issuer `sap_ai_core` only supports `openai` adapters"
                        .to_owned(),
                });
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

fn api_auth_requires_base_url(definition: &ProviderAdapterDefinition) -> bool {
    matches!(
        definition,
        ProviderAdapterDefinition::OpenAi(_)
            | ProviderAdapterDefinition::Anthropic(_)
            | ProviderAdapterDefinition::Gemini(_)
    )
}

fn issuer_label(issuer: CredentialIssuer) -> &'static str {
    match issuer {
        CredentialIssuer::OpenaiChatgpt => "openai_chatgpt",
        CredentialIssuer::GithubCopilot => "github_copilot",
        CredentialIssuer::Gitlab => "gitlab",
        CredentialIssuer::GoogleAdc => "google_adc",
        CredentialIssuer::SapAiCore => "sap_ai_core",
        CredentialIssuer::AtomGit => "atomgit",
    }
}

pub(crate) trait Merge {
    fn merge_from(&mut self, overlay: Self);
}

macro_rules! impl_local_merge_via_crate {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Merge for $ty {
                fn merge_from(&mut self, overlay: Self) {
                    <Self as merge::Merge>::merge(self, overlay);
                }
            }
        )*
    };
}

impl_local_merge_via_crate!(
    RawPluginConfig,
    RawTracingConfig,
    RawTelemetryConfig,
    RawUiConfig,
    RawDefaultConfig,
    RawRuntimeConfig,
    RawProviderHttpConfig,
    RawRequestRetryConfig,
    RawStreamReplayConfig,
    RawRuntimeModelCatalogConfig,
    RawRuntimeReloadConfig,
    RawRuntimeJanitorConfig,
    RawSessionCacheConfig,
    ProviderProtocolPathsOverlay,
    ProviderAuthOverlay,
    ProviderAdapterOverlay,
    ProviderOverlay,
);

pub(crate) fn merge_option<T>(base: &mut Option<T>, overlay: Option<T>) {
    if let Some(value) = overlay {
        *base = Some(value);
    }
}

pub(crate) fn option_override<T>(base: &mut Option<T>, overlay: Option<T>) {
    merge_option(base, overlay);
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

pub(crate) fn option_struct_merge<T>(base: &mut Option<T>, overlay: Option<T>)
where
    T: Merge,
{
    merge_option_struct(base, overlay);
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

pub(crate) fn map_extend<K, V>(base: &mut BTreeMap<K, V>, overlay: BTreeMap<K, V>)
where
    K: Ord,
{
    base.extend(overlay);
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
    models: &BTreeMap<String, ProviderModelOverlay>,
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
