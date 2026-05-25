use std::{collections::BTreeMap, fs, path::Path, str::FromStr};

use merge::Merge as DeriveMerge;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::{
    ConfiguredModelSpeedMode, ConfiguredModelThinkingMode, ProviderRequestRetryConfig,
    ProviderStreamReplayConfig, auth::CredentialIssuer,
};

use super::{
    AgentConfig, BedrockSigv4AuthConfig, ConfigEnvironment, ConfigError, HarnessViewportConfig,
    HarnessesConfig, HttpProviderAdapterConfig, OpenAiApiModeConfig, PluginConfig,
    ProviderAdapterDefinition, ProviderAdapterOverlay, ProviderApiAuthConfig, ProviderAuthConfig,
    ProviderAuthMode, ProviderAuthOverlay, ProviderCapabilityFamilyConfig,
    ProviderCredentialAuthConfig, ProviderDefaultsConfig, ProviderGitlabAuthConfig,
    ProviderHostedToolConfigs, ProviderModelDiscoveryConfig, ProviderModelOverlay,
    ProviderNativeToolKind, ProviderNativeToolRoute, ProviderNativeToolsConfig, ProviderOverlay,
    ProviderProtocolPathsConfig, ProviderProtocolPathsOverlay, ResolvedConfig,
    ResolvedProviderAdapterConfig, ResolvedProviderConfig, ResolvedProviderModelConfig,
    RuntimeConfig, RuntimeGcConfig, RuntimeModelCatalogConfig, RuntimeProvidersConfig,
    RuntimeSessionConfig, SessionCacheConfig, SessionCompactionConfig, SessionConfig,
    StreamTransportMode, TracingConfig, UiConfig,
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
    if table.contains_key("telemetry") {
        return Err(ConfigError::Validation(
            "`telemetry` has been removed".to_string(),
        ));
    }
    if table.contains_key("hooks") {
        return Err(ConfigError::Validation(
            "`hooks` has been removed; implement hook behavior as a regular plugin under `plugins.list.<id>`".to_string(),
        ));
    }
    for (field, plugin_id) in [
        ("memory", "agena.memory"),
        ("web", "agena.web"),
        ("crawl", "agena.web"),
        ("mcp", "agena.mcp"),
        ("lsp", "agena.lsp"),
    ] {
        if table.contains_key(field) {
            return Err(ConfigError::Validation(format!(
                "`{field}` has moved under `plugins.list.\"{plugin_id}\".config`"
            )));
        }
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawProvidersConfig {
    #[merge(strategy = option_override)]
    pub(crate) default: Option<String>,
    #[serde(flatten)]
    #[merge(strategy = map_extend)]
    pub(crate) providers: BTreeMap<String, ProviderOverlay>,
}

impl RawProvidersConfig {
    fn is_empty(&self) -> bool {
        self.default.is_none() && self.providers.is_empty()
    }
}

impl Merge for RawProvidersConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.default, overlay.default);
        merge_map(&mut self.providers, overlay.providers);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub(crate) struct RawAgentsConfig {
    #[merge(strategy = option_override)]
    pub(crate) default: Option<String>,
    #[serde(flatten)]
    #[merge(strategy = map_extend)]
    pub(crate) agents: BTreeMap<String, AgentConfig>,
}

impl RawAgentsConfig {
    fn is_empty(&self) -> bool {
        self.default.is_none() && self.agents.is_empty()
    }
}

impl Merge for RawAgentsConfig {
    fn merge_from(&mut self, overlay: Self) {
        merge_option(&mut self.default, overlay.default);
        merge_map(&mut self.agents, overlay.agents);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawConfig {
    pub(crate) tracing: Option<RawTracingConfig>,
    pub(crate) ui: Option<RawUiConfig>,
    pub(crate) runtime: Option<RawRuntimeConfig>,
    pub(crate) session: Option<RawSessionConfig>,
    pub(crate) permission: Option<crate::agent::PermissionConfig>,
    pub(crate) agents: RawAgentsConfig,
    pub(crate) plugins: Option<PluginConfig>,
    pub(crate) harnesses: Option<HarnessesConfig>,
    pub(crate) providers: RawProvidersConfig,
}

impl RawConfig {
    pub(crate) fn merge_from(&mut self, overlay: Self) {
        merge_option_struct(&mut self.tracing, overlay.tracing);
        merge_option_struct(&mut self.ui, overlay.ui);
        merge_option_struct(&mut self.runtime, overlay.runtime);
        merge_option_struct(&mut self.session, overlay.session);
        merge_option_struct(&mut self.permission, overlay.permission);
        self.agents.merge_from(overlay.agents);
        merge_option_struct(&mut self.plugins, overlay.plugins);
        merge_option_struct(&mut self.harnesses, overlay.harnesses);
        self.providers.merge_from(overlay.providers);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tracing.is_none()
            && self.ui.is_none()
            && self.runtime.is_none()
            && self.session.is_none()
            && self.permission.is_none()
            && self.agents.is_empty()
            && self.plugins.is_none()
            && self.harnesses.is_none()
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
        if let Some(locale) = env.var("AGENA_LOCALE") {
            config.ui.get_or_insert_with(RawUiConfig::default).locale = Some(locale);
        }
        // Note: `AGENA_PLUGIN_PATHS` is no longer supported — the new plugin
        // config requires explicit `plugins.list.<id>` entries.

        apply_env_number(env, "AGENA_PROVIDER_HTTP_TIMEOUT_SECS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .providers
                .get_or_insert_with(RawRuntimeProvidersConfig::default)
                .http
                .get_or_insert_with(RawProviderHttpConfig::default)
                .timeout_secs = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_CONNECT_TIMEOUT_SECS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .providers
                .get_or_insert_with(RawRuntimeProvidersConfig::default)
                .http
                .get_or_insert_with(RawProviderHttpConfig::default)
                .connect_timeout_secs = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_REQUEST_MAX_RETRIES", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .providers
                .get_or_insert_with(RawRuntimeProvidersConfig::default)
                .retry
                .get_or_insert_with(RawRequestRetryConfig::default)
                .max_retries = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_RETRY_BASE_DELAY_MS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .providers
                .get_or_insert_with(RawRuntimeProvidersConfig::default)
                .retry
                .get_or_insert_with(RawRequestRetryConfig::default)
                .base_delay_ms = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_RETRY_MAX_DELAY_MS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .providers
                .get_or_insert_with(RawRuntimeProvidersConfig::default)
                .retry
                .get_or_insert_with(RawRequestRetryConfig::default)
                .max_delay_ms = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_STREAM_REPLAY_MAX_RETRIES", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .providers
                .get_or_insert_with(RawRuntimeProvidersConfig::default)
                .stream_replay
                .get_or_insert_with(RawStreamReplayConfig::default)
                .max_retries_after_output = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_STREAM_REPLAY_MAX_EVENTS", |value| {
            config
                .runtime
                .get_or_insert_with(RawRuntimeConfig::default)
                .providers
                .get_or_insert_with(RawRuntimeProvidersConfig::default)
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
        if let Some(enabled) = env.var("AGENA_SESSION_COMPACTION_AUTO") {
            config
                .session
                .get_or_insert_with(RawSessionConfig::default)
                .compaction
                .get_or_insert_with(RawSessionCompactionConfig::default)
                .auto = Some(parse_bool(
                "AGENA_SESSION_COMPACTION_AUTO",
                enabled.as_str(),
            )?);
        }
        apply_env_number(env, "AGENA_SESSION_COMPACTION_RESERVED_TOKENS", |value| {
            config
                .session
                .get_or_insert_with(RawSessionConfig::default)
                .compaction
                .get_or_insert_with(RawSessionCompactionConfig::default)
                .reserved_tokens = Some(value);
        })?;

        Ok(config)
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
        let ui = UiConfig {
            locale: self
                .ui
                .and_then(|value| value.locale)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        };

        let raw_runtime = self.runtime.unwrap_or_default();
        let raw_session = self.session.unwrap_or_default();
        let runtime = RuntimeConfig::from_raw(raw_runtime)?;
        let session = SessionConfig::from_raw(raw_session)?;
        let permission = self.permission.unwrap_or_default();
        let plugins: PluginConfig = self.plugins.unwrap_or_default();
        PluginRuntimeOptions::validate_builtin_plugins(&plugins)?;
        let mcp = crate::plugins::provided::mcp::config_from_plugins(&plugins)
            .map_err(ConfigError::Validation)?;
        let harnesses: HarnessesConfig = self.harnesses.unwrap_or_default();
        let providers_default = self.providers.default.clone();
        let agents_default = self.agents.default.clone();

        validate_harnesses(&harnesses)?;

        let providers = self
            .providers
            .providers
            .into_iter()
            .map(|(provider_id, raw)| raw.resolve(provider_id, env, &harnesses, &mcp))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let default_selection =
            resolve_default_selection(providers_default.as_deref(), &providers)?;
        let default_agent = resolve_default_agent(agents_default.as_deref(), &self.agents.agents)?;

        validate_permission_config("permission", &permission)?;
        for (agent_name, agent) in &self.agents.agents {
            let effective = permission.merged_with(&agent.permission);
            validate_permission_config(
                format!("agents.{agent_name}.permission").as_str(),
                &effective,
            )?;
        }

        Ok(ResolvedConfig {
            default_selection,
            default_agent,
            tracing,
            ui,
            runtime,
            session,
            permission,
            agents: self.agents.agents,
            plugins,
            plugin_storage: crate::config::types::PluginStorageConfig::default(),
            harnesses,
            providers,
        })
    }
}

fn resolve_default_selection(
    explicit_provider: Option<&str>,
    providers: &BTreeMap<String, ResolvedProviderConfig>,
) -> Result<crate::execution_prefs::ExecutionSelection, ConfigError> {
    let provider_id = if let Some(explicit_provider) = explicit_provider
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let provider = providers.get(explicit_provider).ok_or_else(|| {
            ConfigError::Validation(format!(
                "providers.default `{explicit_provider}` references unknown provider"
            ))
        })?;
        if !provider.enabled {
            return Err(ConfigError::Validation(format!(
                "providers.default `{explicit_provider}` references disabled provider"
            )));
        }
        Some(explicit_provider.to_owned())
    } else {
        providers
            .iter()
            .find_map(|(provider_id, provider)| provider.enabled.then(|| provider_id.clone()))
    };

    let Some(provider_id) = provider_id else {
        return Ok(crate::execution_prefs::ExecutionSelection::default());
    };
    let provider = providers.get(provider_id.as_str()).ok_or_else(|| {
        ConfigError::Validation(format!(
            "providers.default `{provider_id}` references unknown provider"
        ))
    })?;

    Ok(crate::execution_prefs::ExecutionSelection {
        provider: Some(provider_id),
        adapter: provider.defaults.adapter.clone(),
        model: provider.defaults.model.clone(),
        thinking_mode: provider.defaults.thinking_mode.clone(),
        speed_mode: provider.defaults.speed_mode.clone(),
        verbosity: provider.defaults.verbosity.clone(),
        parallel_tool_calls: provider.defaults.parallel_tool_calls,
        ..Default::default()
    })
}

fn resolve_default_agent(
    explicit_agent: Option<&str>,
    agents: &BTreeMap<String, AgentConfig>,
) -> Result<Option<String>, ConfigError> {
    if let Some(explicit_agent) = explicit_agent
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let agent = agents.get(explicit_agent).ok_or_else(|| {
            ConfigError::Validation(format!(
                "agents.default `{explicit_agent}` references unknown agent"
            ))
        })?;
        if agent.disabled {
            return Err(ConfigError::Validation(format!(
                "agents.default `{explicit_agent}` references disabled agent"
            )));
        }
        return Ok(Some(explicit_agent.to_owned()));
    }

    Ok(agents
        .iter()
        .find_map(|(agent_name, agent)| (!agent.disabled).then(|| agent_name.clone())))
}

#[derive(Debug, Default)]
struct PluginRuntimeOptions;

impl PluginRuntimeOptions {
    fn validate_builtin_plugins(plugins: &PluginConfig) -> Result<(), ConfigError> {
        for plugin_id in plugins.list.keys() {
            if plugin_id == "agena.hooks" {
                return Err(ConfigError::Validation(
                    "`plugins.list.\"agena.hooks\"` has been removed; implement hook behavior as a regular plugin under `plugins.list.<id>`".to_string(),
                ));
            }
            if plugin_id == "agena.crawl" {
                return Err(ConfigError::Validation(
                    "`plugins.list.\"agena.crawl\"` has been renamed to `plugins.list.\"agena.web\"`; move configuration fields under `plugins.list.\"agena.web\".config`".to_string(),
                ));
            }
        }
        Ok(())
    }
}

impl Merge for PluginConfig {
    fn merge_from(&mut self, overlay: Self) {
        // Overlay completely replaces nested plugin tools; otherwise we'd
        // need tool-level merge logic. List entries from a more-specific
        // mode override the parent.
        if !overlay.list.is_empty() {
            self.list = overlay.list;
        }
        if !overlay.host.is_default() {
            self.host = overlay.host;
        }
        if !overlay.policy.is_default() {
            self.policy = overlay.policy;
        }
    }
}

impl Merge for HarnessesConfig {
    fn merge_from(&mut self, overlay: Self) {
        for (name, config) in overlay.browser {
            self.browser.insert(name, config);
        }
        for (name, config) in overlay.shell {
            self.shell.insert(name, config);
        }
        for (name, config) in overlay.editor {
            self.editor.insert(name, config);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawTracingConfig {
    #[merge(strategy = option_override)]
    pub(crate) filter: Option<String>,
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
#[serde(default, deny_unknown_fields)]
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
        self.permission.merge_from(overlay.permission);
        merge_option(&mut self.defaults.provider, overlay.defaults.provider);
        merge_option(&mut self.defaults.adapter, overlay.defaults.adapter);
        merge_option(&mut self.defaults.model, overlay.defaults.model);
        merge_option(
            &mut self.defaults.thinking_mode,
            overlay.defaults.thinking_mode,
        );
        merge_option(&mut self.defaults.speed_mode, overlay.defaults.speed_mode);
        merge_option(&mut self.defaults.verbosity, overlay.defaults.verbosity);
        merge_option(
            &mut self.defaults.parallel_tool_calls,
            overlay.defaults.parallel_tool_calls,
        );
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawRuntimeConfig {
    #[merge(strategy = option_struct_merge)]
    pub(crate) providers: Option<RawRuntimeProvidersConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) model_catalog: Option<RawRuntimeModelCatalogConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) reload: Option<RawRuntimeReloadConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) session: Option<RawRuntimeSessionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawRuntimeProvidersConfig {
    #[merge(strategy = option_struct_merge)]
    pub(crate) http: Option<RawProviderHttpConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) retry: Option<RawRequestRetryConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) stream_replay: Option<RawStreamReplayConfig>,
}

impl RuntimeConfig {
    pub(crate) fn from_raw(raw: RawRuntimeConfig) -> Result<Self, ConfigError> {
        let providers = raw.providers.unwrap_or_default();
        let provider_http = providers.http.unwrap_or_default();
        let request_retry = providers.retry.unwrap_or_default();
        let stream_replay = providers.stream_replay.unwrap_or_default();
        let model_catalog = raw.model_catalog.unwrap_or_default();
        let reload = raw.reload.unwrap_or_default();
        let session = raw.session.unwrap_or_default();

        let timeout_secs = provider_http.timeout_secs.unwrap_or(120);
        let connect_timeout_secs = provider_http.connect_timeout_secs.unwrap_or(15);
        if timeout_secs == 0 || connect_timeout_secs == 0 {
            return Err(ConfigError::Validation(
                "runtime.providers.http timeout values must be greater than 0".to_owned(),
            ));
        }

        let base_delay_ms = request_retry.base_delay_ms.unwrap_or(250);
        let max_delay_ms = request_retry
            .max_delay_ms
            .unwrap_or(2_000)
            .max(base_delay_ms);
        let reload_poll_interval_secs = reload.poll_interval_secs.unwrap_or(2);
        let model_catalog_cache_max_age_secs = model_catalog
            .cache_max_age_secs
            .unwrap_or(crate::model_catalog::DEFAULT_CACHE_MAX_AGE_SECS);

        if reload_poll_interval_secs == 0 {
            return Err(ConfigError::Validation(
                "runtime.reload.poll_interval_secs must be greater than 0".to_owned(),
            ));
        }
        if model_catalog_cache_max_age_secs == 0 {
            return Err(ConfigError::Validation(
                "runtime.model_catalog.cache_max_age_secs must be greater than 0".to_owned(),
            ));
        }

        let runtime_session = RuntimeSessionConfig::from_raw(session)?;

        Ok(Self {
            providers: RuntimeProvidersConfig {
                http: super::ProviderHttpConfig {
                    timeout_secs,
                    connect_timeout_secs,
                },
                retry: super::RequestRetryConfig {
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
            },
            model_catalog: RuntimeModelCatalogConfig {
                cache_max_age_secs: model_catalog_cache_max_age_secs,
            },
            reload: super::RuntimeReloadConfig {
                enabled: reload.enabled.unwrap_or(true),
                poll_interval_secs: reload_poll_interval_secs,
            },
            session: runtime_session,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawSessionConfig {
    #[merge(strategy = option_struct_merge)]
    pub(crate) compaction: Option<RawSessionCompactionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawSessionCompactionConfig {
    #[merge(strategy = option_override)]
    pub(crate) auto: Option<bool>,
    #[merge(strategy = option_override)]
    pub(crate) reserved_tokens: Option<u32>,
}

impl SessionConfig {
    pub(crate) fn from_raw(raw: RawSessionConfig) -> Result<Self, ConfigError> {
        let compaction = raw.compaction.unwrap_or_default();
        Ok(Self {
            compaction: SessionCompactionConfig {
                auto: compaction.auto.unwrap_or(true),
                reserved_tokens: compaction.reserved_tokens,
            },
        })
    }
}

impl RuntimeSessionConfig {
    pub(crate) fn from_raw(raw: RawRuntimeSessionConfig) -> Result<Self, ConfigError> {
        let cache = raw.cache.unwrap_or_default();
        let gc = raw.gc.unwrap_or_default();
        let cache_ttl_secs = cache.ttl_secs.unwrap_or(15 * 60);
        let cache_max_sessions = cache.max_sessions.unwrap_or(128);
        let cache_max_bytes = cache.max_bytes.unwrap_or(64 * 1024 * 1024);
        let gc_interval_secs = gc.interval_secs.unwrap_or(30);

        if gc_interval_secs == 0 {
            return Err(ConfigError::Validation(
                "runtime.session.gc.interval_secs must be greater than 0".to_owned(),
            ));
        }
        if cache_ttl_secs == 0 {
            return Err(ConfigError::Validation(
                "runtime.session.cache.ttl_secs must be greater than 0".to_owned(),
            ));
        }
        if cache_max_sessions == 0 {
            return Err(ConfigError::Validation(
                "runtime.session.cache.max_sessions must be greater than 0".to_owned(),
            ));
        }
        if cache_max_bytes == 0 {
            return Err(ConfigError::Validation(
                "runtime.session.cache.max_bytes must be greater than 0".to_owned(),
            ));
        }

        Ok(Self {
            cache: SessionCacheConfig {
                max_sessions: cache_max_sessions,
                ttl_secs: cache_ttl_secs,
                max_bytes: cache_max_bytes,
            },
            gc: RuntimeGcConfig {
                enabled: gc.enabled.unwrap_or(true),
                interval_secs: gc_interval_secs,
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
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawProviderHttpConfig {
    #[merge(strategy = option_override)]
    pub(crate) timeout_secs: Option<u64>,
    #[merge(strategy = option_override)]
    pub(crate) connect_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawRequestRetryConfig {
    #[merge(strategy = option_override)]
    pub(crate) max_retries: Option<u32>,
    #[merge(strategy = option_override)]
    pub(crate) base_delay_ms: Option<u64>,
    #[merge(strategy = option_override)]
    pub(crate) max_delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawStreamReplayConfig {
    #[merge(strategy = option_override)]
    pub(crate) max_retries_after_output: Option<u32>,
    #[merge(strategy = option_override)]
    pub(crate) max_tracked_events: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawRuntimeModelCatalogConfig {
    #[merge(strategy = option_override)]
    pub(crate) cache_max_age_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawRuntimeReloadConfig {
    #[merge(strategy = option_override)]
    pub(crate) enabled: Option<bool>,
    #[merge(strategy = option_override)]
    pub(crate) poll_interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawRuntimeGcConfig {
    #[merge(strategy = option_override)]
    pub(crate) enabled: Option<bool>,
    #[merge(strategy = option_override)]
    pub(crate) interval_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawRuntimeSessionConfig {
    #[merge(strategy = option_struct_merge)]
    pub(crate) cache: Option<RawSessionCacheConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) gc: Option<RawRuntimeGcConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
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
        harnesses: &HarnessesConfig,
        mcp: &crate::plugins::provided::mcp::McpConfig,
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
                        native_tools: configured.native_tools.clone(),
                        definition: configured.definition.clone(),
                    },
                );
            }
            adapters.insert(adapter_id, adapter.config);
        }

        let provider_defaults = self.defaults.unwrap_or_default();
        if let Some(default_provider) =
            normalize_optional_string(provider_defaults.provider.clone())
        {
            if default_provider.as_str() != provider_id.as_str() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.clone(),
                    message: format!(
                        "provider defaults.provider `{default_provider}` must match provider key `{provider_id}`"
                    ),
                });
            }
        }
        let auth = resolve_provider_auth(provider_id.as_str(), self.auth, adapters.values())?;
        validate_provider_auth(provider_id.as_str(), &auth, adapters.values())?;
        validate_provider_model_native_tools(provider_id.as_str(), &models, harnesses, mcp)?;
        let default_adapter = if let Some(default_adapter) = provider_defaults.adapter.clone() {
            default_adapter
        } else {
            let enabled_adapters = adapters
                .iter()
                .filter(|(_, adapter)| adapter.enabled)
                .map(|(adapter_id, _)| adapter_id.clone())
                .collect::<Vec<_>>();
            (enabled_adapters.len() == 1)
                .then(|| enabled_adapters[0].clone())
                .ok_or_else(|| ConfigError::MissingProviderField {
                    provider_id: provider_id.clone(),
                    field: "defaults.adapter",
                })?
        };
        if default_adapter.trim().is_empty() {
            return Err(ConfigError::MissingProviderField {
                provider_id: provider_id.clone(),
                field: "defaults.adapter",
            });
        }
        let default_model = if let Some(default_model) = provider_defaults.model.clone() {
            default_model
        } else {
            return Err(ConfigError::MissingProviderField {
                provider_id: provider_id.clone(),
                field: "defaults.model",
            });
        };
        if default_model.is_empty() {
            return Err(ConfigError::MissingProviderField {
                provider_id: provider_id.clone(),
                field: "defaults.model",
            });
        }
        let default_adapter_id = default_adapter.trim().to_owned();
        let default_adapter = adapters.get(default_adapter_id.as_str()).ok_or_else(|| {
            ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: format!(
                    "provider defaults.adapter `{default_adapter_id}` references unknown adapter"
                ),
            }
        })?;
        if !default_adapter.enabled {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: format!(
                    "provider defaults.adapter `{default_adapter_id}` references disabled adapter"
                ),
            });
        }
        let default_route = format!("{default_adapter_id}/{default_model}");
        if matches!(models.get(default_route.as_str()), Some(configured) if !configured.enabled) {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.clone(),
                message: format!(
                    "provider defaults.model `{default_model}` references disabled model route `{default_route}`"
                ),
            });
        }

        let resolved_provider_id = provider_id.clone();

        Ok((
            provider_id,
            ResolvedProviderConfig {
                enabled,
                defaults: ProviderDefaultsConfig {
                    provider: Some(resolved_provider_id),
                    adapter: Some(default_adapter_id),
                    model: Some(default_model),
                    thinking_mode: provider_defaults.thinking_mode,
                    speed_mode: provider_defaults.speed_mode,
                    verbosity: provider_defaults.verbosity,
                    parallel_tool_calls: provider_defaults.parallel_tool_calls,
                },
                auth,
                adapters,
                models,
            },
        ))
    }
}

fn validate_provider_model_native_tools(
    provider_id: &str,
    models: &BTreeMap<String, ResolvedProviderModelConfig>,
    harnesses: &HarnessesConfig,
    mcp: &crate::plugins::provided::mcp::McpConfig,
) -> Result<(), ConfigError> {
    for (route_id, model) in models {
        validate_provider_native_tools(
            provider_id,
            Some(route_id.as_str()),
            &model.native_tools,
            harnesses,
            mcp,
        )?;
    }
    Ok(())
}

fn validate_provider_native_tools(
    provider_id: &str,
    route_id: Option<&str>,
    config: &ProviderNativeToolsConfig,
    harnesses: &HarnessesConfig,
    mcp: &crate::plugins::provided::mcp::McpConfig,
) -> Result<(), ConfigError> {
    validate_hosted_native_tool_config(provider_id, route_id, &config.hosted)?;

    for tool in ProviderNativeToolKind::ALL {
        if let Some(route) = config.routes.route_for(tool) {
            if !tool.supports_route(route) {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} native tool `{}` does not support route `{route:?}`",
                        native_tool_scope(route_id),
                        tool.config_key(),
                    ),
                });
            }
            if route == ProviderNativeToolRoute::ProviderHarness
                && config.harness.binding_for(tool).is_none()
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} native tool `{}` routed to `provider_harness` requires a harness binding",
                        native_tool_scope(route_id),
                        tool.config_key(),
                    ),
                });
            }
            if route == ProviderNativeToolRoute::ProviderConnector
                && tool == ProviderNativeToolKind::RemoteMcp
                && config.connectors.is_empty()
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} native tool `remote_mcp` routed to `provider_connector` requires at least one connector",
                        native_tool_scope(route_id)
                    ),
                });
            }
        }
    }

    for tool in [
        ProviderNativeToolKind::Computer,
        ProviderNativeToolKind::Bash,
        ProviderNativeToolKind::TextEditor,
    ] {
        if let Some(reference) = config.harness.binding_for(tool) {
            if reference.name.trim().is_empty() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} native tool `{}` references an empty harness name",
                        native_tool_scope(route_id),
                        tool.config_key(),
                    ),
                });
            }
            if !harnesses.contains(reference) {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} native tool `{}` references missing {:?} harness `{}`",
                        native_tool_scope(route_id),
                        tool.config_key(),
                        reference.kind,
                        reference.name
                    ),
                });
            }
        }
    }

    for (name, connector) in &config.connectors {
        if name.trim().is_empty() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "{} connector name cannot be empty",
                    native_tool_scope(route_id)
                ),
            });
        }
        if connector.server.trim().is_empty() {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "{} connector `{name}` must set non-empty `server`",
                    native_tool_scope(route_id)
                ),
            });
        }
        if !mcp.servers.contains_key(connector.server.as_str()) {
            return Err(ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message: format!(
                    "{} connector `{name}` references unknown MCP server `{}`",
                    native_tool_scope(route_id),
                    connector.server
                ),
            });
        }
        for tool_name in &connector.tool_filter {
            if tool_name.trim().is_empty() {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!(
                        "{} connector `{name}` contains an empty tool name in `tool_filter`",
                        native_tool_scope(route_id)
                    ),
                });
            }
        }
    }

    Ok(())
}

fn validate_hosted_native_tool_config(
    provider_id: &str,
    route_id: Option<&str>,
    hosted: &ProviderHostedToolConfigs,
) -> Result<(), ConfigError> {
    validate_non_empty_strings(
        provider_id,
        hosted_native_tool_path(route_id, "web_search.allowed_domains").as_str(),
        &hosted.web_search.allowed_domains,
    )?;
    validate_non_empty_strings(
        provider_id,
        hosted_native_tool_path(route_id, "web_search.blocked_domains").as_str(),
        &hosted.web_search.blocked_domains,
    )?;
    validate_non_empty_strings(
        provider_id,
        hosted_native_tool_path(route_id, "file_search.vector_store_ids").as_str(),
        &hosted.file_search.vector_store_ids,
    )?;
    validate_non_empty_strings(
        provider_id,
        hosted_native_tool_path(route_id, "code_execution.container.file_ids").as_str(),
        &hosted.code_execution.container.file_ids,
    )?;
    if matches!(hosted.web_search.max_results, Some(0)) {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!(
                "{} native tool `web_search` hosted `max_results` must be greater than 0",
                native_tool_scope(route_id)
            ),
        });
    }
    if matches!(hosted.file_search.max_results, Some(0)) {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!(
                "{} native tool `file_search` hosted `max_results` must be greater than 0",
                native_tool_scope(route_id)
            ),
        });
    }
    if matches!(hosted.url_context.max_urls, Some(0)) {
        return Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: format!(
                "{} native tool `url_context` hosted `max_urls` must be greater than 0",
                native_tool_scope(route_id)
            ),
        });
    }
    Ok(())
}

fn native_tool_scope(route_id: Option<&str>) -> String {
    route_id
        .map(|route_id| format!("provider model `{route_id}`"))
        .unwrap_or_else(|| "provider".to_owned())
}

fn hosted_native_tool_path(route_id: Option<&str>, suffix: &str) -> String {
    route_id
        .map(|route_id| format!("models.{route_id}.native_tools.hosted.{suffix}"))
        .unwrap_or_else(|| format!("native_tools.hosted.{suffix}"))
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
    RawTracingConfig,
    RawUiConfig,
    RawRuntimeConfig,
    RawRuntimeProvidersConfig,
    RawSessionConfig,
    RawSessionCompactionConfig,
    RawProviderHttpConfig,
    RawRequestRetryConfig,
    RawStreamReplayConfig,
    RawRuntimeModelCatalogConfig,
    RawRuntimeReloadConfig,
    RawRuntimeGcConfig,
    RawRuntimeSessionConfig,
    RawSessionCacheConfig,
    ProviderProtocolPathsOverlay,
    ProviderAuthOverlay,
    super::overlay::ProviderNativeToolRoutesOverlay,
    super::overlay::NativeToolUserLocationOverlay,
    super::overlay::ProviderHostedWebSearchOverlay,
    super::overlay::ProviderHostedFileSearchOverlay,
    super::overlay::HostedCodeExecutionContainerOverlay,
    super::overlay::ProviderHostedCodeExecutionOverlay,
    super::overlay::ProviderHostedImageGenerationOverlay,
    super::overlay::ProviderHostedUrlContextOverlay,
    super::overlay::ProviderHostedToolsOverlay,
    super::overlay::ProviderNativeHarnessRefOverlay,
    super::overlay::ProviderNativeHarnessBindingsOverlay,
    super::overlay::ProviderNativeConnectorOverlay,
    super::overlay::ProviderNativeToolsOverlay,
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

fn validate_harnesses(harnesses: &HarnessesConfig) -> Result<(), ConfigError> {
    for (name, browser) in &harnesses.browser {
        validate_harness_name("harnesses.browser", name)?;
        if browser.driver.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "harnesses.browser.{name}.driver cannot be empty"
            )));
        }
        validate_non_empty_list(
            format!("harnesses.browser.{name}.allowed_domains").as_str(),
            &browser.allowed_domains,
        )?;
        validate_viewport(
            format!("harnesses.browser.{name}.viewport").as_str(),
            &browser.viewport,
        )?;
    }

    for (name, shell) in &harnesses.shell {
        validate_harness_name("harnesses.shell", name)?;
        validate_non_empty_list(
            format!("harnesses.shell.{name}.allow_commands").as_str(),
            &shell.allow_commands,
        )?;
        validate_non_empty_list(
            format!("harnesses.shell.{name}.deny_commands").as_str(),
            &shell.deny_commands,
        )?;
    }

    for (name, editor) in &harnesses.editor {
        validate_harness_name("harnesses.editor", name)?;
        validate_non_empty_list(
            format!("harnesses.editor.{name}.allowed_extensions").as_str(),
            &editor.allowed_extensions,
        )?;
        if matches!(editor.max_file_bytes, Some(0)) {
            return Err(ConfigError::Validation(format!(
                "harnesses.editor.{name}.max_file_bytes must be greater than 0"
            )));
        }
    }

    Ok(())
}

fn validate_harness_name(scope: &str, name: &str) -> Result<(), ConfigError> {
    if name.trim().is_empty() {
        return Err(ConfigError::Validation(format!(
            "{scope} entry names cannot be empty"
        )));
    }
    Ok(())
}

fn validate_viewport(label: &str, viewport: &HarnessViewportConfig) -> Result<(), ConfigError> {
    if viewport.is_empty() {
        return Ok(());
    }
    if viewport.width == 0 || viewport.height == 0 {
        return Err(ConfigError::Validation(format!(
            "{label} must set both width and height"
        )));
    }
    Ok(())
}

fn validate_non_empty_list(label: &str, values: &[String]) -> Result<(), ConfigError> {
    if values.iter().any(|value| value.trim().is_empty()) {
        return Err(ConfigError::Validation(format!(
            "{label} cannot contain empty strings"
        )));
    }
    Ok(())
}

fn validate_non_empty_strings(
    provider_id: &str,
    field: &str,
    values: &[String],
) -> Result<(), ConfigError> {
    validate_non_empty_list(field, values).map_err(|_| ConfigError::InvalidProviderConfig {
        provider_id: provider_id.to_owned(),
        message: format!("{field} cannot contain empty strings"),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct TestEnvironment;

    impl ConfigEnvironment for TestEnvironment {
        fn var(&self, _key: &str) -> Option<String> {
            None
        }

        fn vars(&self) -> Vec<(String, String)> {
            Vec::new()
        }
    }

    fn resolve_config(value: serde_json::Value) -> Result<ResolvedConfig, ConfigError> {
        let raw = serde_json::from_value::<RawConfig>(value).expect("config should parse");
        raw.resolve_with_env(&TestEnvironment)
    }

    #[test]
    fn resolves_provider_native_tools_and_bindings() {
        let resolved = resolve_config(json!({
            "plugins": {
                "list": {
                    "agena.mcp": {
                        "package": {
                            "kind": "static"
                        },
                        "config": {
                            "servers": {
                                "docs": {
                                    "transport": "http",
                                    "url": "https://example.com/mcp"
                                }
                            }
                        }
                    }
                }
            },
            "harnesses": {
                "browser": {
                    "default": {
                        "driver": "playwright",
                        "headless": true
                    }
                },
                "shell": {
                    "default": {
                        "workspace_only": true
                    }
                }
            },
            "providers": {
                "default": "openai",
                "openai": {
                    "defaults": {
                        "adapter": "openai",
                        "model": "gpt-5"
                    },
                    "auth": {
                        "mode": "api",
                        "base_url": "https://api.openai.com",
                        "api_key": "test"
                    },
                    "adapters": {
                        "openai": {
                            "enabled": true,
                            "models": {
                                "gpt-5": {
                                    "native_tools": {
                                        "enabled": true,
                                        "routes": {
                                            "web_search": "provider_hosted",
                                            "file_search": "provider_hosted",
                                            "remote_mcp": "provider_connector"
                                        },
                                        "hosted": {
                                            "web_search": {
                                                "allowed_domains": ["example.com"]
                                            },
                                            "file_search": {
                                                "vector_store_ids": ["vs_docs"]
                                            }
                                        },
                                        "connectors": {
                                            "docs": {
                                                "server": "docs",
                                                "tool_filter": ["search"]
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }))
        .expect("native tool config should resolve");

        let provider = resolved
            .providers
            .get("openai")
            .expect("resolved provider should exist");
        let model = provider.models.get("openai/gpt-5").expect("resolved model");
        assert!(model.native_tools.enabled);
        assert_eq!(
            model.native_tools.routes.web_search,
            Some(ProviderNativeToolRoute::ProviderHosted)
        );
        assert_eq!(
            model.native_tools.hosted.file_search.vector_store_ids,
            vec!["vs_docs".to_owned()]
        );
        let bindings = model.native_tool_bindings();
        assert!(bindings.iter().any(|binding| {
            binding.tool == ProviderNativeToolKind::RemoteMcp
                && binding.route == ProviderNativeToolRoute::ProviderConnector
                && binding.connector_names == vec!["docs".to_owned()]
        }));
        assert_eq!(bindings.len(), 3);
    }

    #[test]
    fn old_flat_plugin_entry_shape_is_rejected() {
        let err = serde_json::from_value::<RawConfig>(json!({
            "plugins": {
                "list": {
                    "agena.web": {
                        "kind": "static",
                        "config": {
                            "default_max_pages": 20
                        }
                    }
                }
            }
        }))
        .expect_err("old flat plugin package fields should be rejected");

        assert!(err.to_string().contains("unknown field `kind`"));
    }

    #[test]
    fn old_crawl_plugin_id_is_rejected() {
        let raw = serde_json::from_value::<RawConfig>(json!({
            "plugins": {
                "list": {
                    "agena.crawl": {
                        "package": {
                            "kind": "static"
                        }
                    }
                }
            }
        }))
        .expect("config should parse");

        let err = raw
            .resolve_with_env(&TestEnvironment)
            .expect_err("old crawl plugin id should be rejected");
        assert!(err.to_string().contains("has been renamed"));
    }

    #[test]
    fn crawl_table_is_rejected() {
        let err = serde_json::from_value::<RawConfig>(json!({
            "crawl": {
                "default_max_pages": 7
            }
        }))
        .expect_err("crawl compatibility alias should be rejected");

        assert!(err.to_string().contains("unknown field `crawl`"));
    }

    #[test]
    fn web_table_is_rejected() {
        let err = serde_json::from_value::<RawConfig>(json!({
            "web": {
                "default_max_pages": 20,
                "max_pages_limit": 10
            }
        }))
        .expect_err("top-level web config should be rejected");

        assert!(err.to_string().contains("unknown field `web`"));
    }

    #[test]
    fn web_config_rejects_removed_search_engine_setting() {
        let err = serde_json::from_value::<RawConfig>(json!({
            "web": {
                "search_engine": "brave"
            }
        }))
        .expect_err("top-level web config should be rejected");

        assert!(err.to_string().contains("unknown field `web`"));
    }

    #[test]
    fn rejects_invalid_native_tool_route() {
        let err = resolve_config(json!({
            "providers": {
                "default": "openai",
                "openai": {
                    "defaults": {
                        "adapter": "openai",
                        "model": "gpt-5"
                    },
                    "auth": {
                        "mode": "api",
                        "base_url": "https://api.openai.com",
                        "api_key": "test"
                    },
                    "adapters": {
                        "openai": {
                            "enabled": true,
                            "models": {
                                "gpt-5": {
                                    "native_tools": {
                                        "enabled": true,
                                        "routes": {
                                            "file_search": "plugin"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }))
        .expect_err("invalid file_search route should fail");

        assert!(
            err.to_string()
                .contains("native tool `file_search` does not support route")
        );
    }

    #[test]
    fn rejects_missing_harness_binding_target() {
        let err = resolve_config(json!({
            "providers": {
                "default": "anthropic",
                "anthropic": {
                    "defaults": {
                        "adapter": "anthropic",
                        "model": "claude-sonnet-4-6"
                    },
                    "auth": {
                        "mode": "api",
                        "base_url": "https://api.anthropic.com",
                        "api_key": "test"
                    },
                    "adapters": {
                        "anthropic": {
                            "enabled": true,
                            "models": {
                                "claude-sonnet-4-6": {
                                    "native_tools": {
                                        "enabled": true,
                                        "routes": {
                                            "bash": "provider_harness"
                                        },
                                        "harness": {
                                            "bash": {
                                                "kind": "shell",
                                                "name": "missing"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }))
        .expect_err("missing harness target should fail");

        assert!(
            err.to_string()
                .contains("references missing Shell harness `missing`")
        );
    }

    #[test]
    fn rejects_missing_connector_server() {
        let err = resolve_config(json!({
            "providers": {
                "default": "openai",
                "openai": {
                    "defaults": {
                        "adapter": "openai",
                        "model": "gpt-5"
                    },
                    "auth": {
                        "mode": "api",
                        "base_url": "https://api.openai.com",
                        "api_key": "test"
                    },
                    "adapters": {
                        "openai": {
                            "enabled": true,
                            "models": {
                                "gpt-5": {
                                    "native_tools": {
                                        "enabled": true,
                                        "routes": {
                                            "remote_mcp": "provider_connector"
                                        },
                                        "connectors": {
                                            "docs": {
                                                "server": "missing"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }))
        .expect_err("missing connector server should fail");

        assert!(
            err.to_string()
                .contains("references unknown MCP server `missing`")
        );
    }

    #[test]
    fn official_provider_without_explicit_native_tools_stays_disabled() {
        let resolved = resolve_config(json!({
            "providers": {
                "default": "openai",
                "openai": {
                    "defaults": {
                        "adapter": "openai",
                        "model": "gpt-5"
                    },
                    "auth": {
                        "mode": "api",
                        "base_url": "https://api.openai.com",
                        "api_key": "test"
                    },
                    "adapters": {
                        "openai": {
                            "enabled": true
                        }
                    }
                }
            }
        }))
        .expect("official provider should resolve without implicit defaults");

        let provider = resolved.providers.get("openai").expect("provider");
        assert!(
            provider
                .models
                .get("openai/gpt-5")
                .map(|model| model.native_tools.is_empty())
                .unwrap_or(true)
        );
        assert!(
            resolved
                .provider_model_native_tool_bindings("openai")
                .unwrap_or_default()
                .is_empty()
        );
    }

    #[test]
    fn explicit_disable_keeps_native_tools_off() {
        let resolved = resolve_config(json!({
            "providers": {
                "default": "anthropic",
                "anthropic": {
                    "defaults": {
                        "adapter": "anthropic",
                        "model": "claude-sonnet-4-6"
                    },
                    "auth": {
                        "mode": "api",
                        "base_url": "https://api.anthropic.com",
                        "api_key": "test"
                    },
                    "adapters": {
                        "anthropic": {
                            "enabled": true,
                            "models": {
                                "claude-sonnet-4-6": {
                                    "native_tools": {
                                        "enabled": false
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }))
        .expect("explicit disable should win");

        let provider = resolved.providers.get("anthropic").expect("provider");
        let model = provider
            .models
            .get("anthropic/claude-sonnet-4-6")
            .expect("configured model");
        assert!(!model.native_tools.enabled);
        assert!(model.native_tool_bindings().is_empty());
        assert!(model.native_tools.routes.is_empty());
    }
}
