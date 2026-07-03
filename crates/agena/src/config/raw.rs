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
    pub(crate) merge_keys: RawProjectMergeKeys,
}

impl RawConfigFile {
    pub(crate) fn read(path: &Path) -> Result<Self, ConfigError> {
        match fs::read_to_string(path) {
            Ok(text) => {
                let value = parse_config_value(path, &text)?;
                reject_unsupported_fields_value(&value)?;
                let merge_keys = RawProjectMergeKeys::from_value(&value);
                let config = serde_json::from_value::<RawConfig>(value).map_err(|source| {
                    ConfigError::ParseFile {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;
                Ok(Self {
                    config,
                    found: true,
                    merge_keys,
                })
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                config: RawConfig::default(),
                found: false,
                merge_keys: RawProjectMergeKeys::default(),
            }),
            Err(source) => Err(ConfigError::ReadFile {
                path: path.to_path_buf(),
                source,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RawProjectMergeKeys {
    plugins_host: bool,
    plugins_host_timeouts: bool,
    plugins_host_default_quota: bool,
    plugins_host_quotas: bool,
    plugins_host_trusted_keys: bool,
    plugins_policy: bool,
    plugins_policy_tool_presentation: bool,
    plugins_policy_tool_presentation_default_mode: bool,
    plugins_policy_tool_presentation_plugins: bool,
    plugins_policy_tool_presentation_tools: bool,
    plugins_policy_ui_presentation: bool,
    plugins_policy_ui_presentation_default_mode: bool,
    plugins_policy_ui_presentation_plugins: bool,
    plugins_policy_ui_presentation_tools: bool,
}

impl RawProjectMergeKeys {
    fn from_value(value: &Value) -> Self {
        let plugins = value.get("plugins").and_then(Value::as_object);
        let plugins_host = plugins
            .and_then(|table| table.get("host"))
            .and_then(Value::as_object);
        let plugins_policy = plugins
            .and_then(|table| table.get("policy"))
            .and_then(Value::as_object);
        let tool_presentation = plugins_policy
            .and_then(|table| table.get("tool_presentation"))
            .and_then(Value::as_object);
        let ui_presentation = plugins_policy
            .and_then(|table| table.get("ui_presentation"))
            .and_then(Value::as_object);
        Self {
            plugins_host: plugins.is_some_and(|table| table.contains_key("host")),
            plugins_host_timeouts: plugins_host.is_some_and(|table| table.contains_key("timeouts")),
            plugins_host_default_quota: plugins_host
                .is_some_and(|table| table.contains_key("default_quota")),
            plugins_host_quotas: plugins_host.is_some_and(|table| table.contains_key("quotas")),
            plugins_host_trusted_keys: plugins_host
                .is_some_and(|table| table.contains_key("trusted_keys")),
            plugins_policy: plugins.is_some_and(|table| table.contains_key("policy")),
            plugins_policy_tool_presentation: plugins_policy
                .is_some_and(|table| table.contains_key("tool_presentation")),
            plugins_policy_tool_presentation_default_mode: tool_presentation
                .is_some_and(|table| table.contains_key("default_mode")),
            plugins_policy_tool_presentation_plugins: tool_presentation
                .is_some_and(|table| table.contains_key("plugins")),
            plugins_policy_tool_presentation_tools: tool_presentation
                .is_some_and(|table| table.contains_key("tools")),
            plugins_policy_ui_presentation: plugins_policy
                .is_some_and(|table| table.contains_key("ui_presentation")),
            plugins_policy_ui_presentation_default_mode: ui_presentation
                .is_some_and(|table| table.contains_key("default_mode")),
            plugins_policy_ui_presentation_plugins: ui_presentation
                .is_some_and(|table| table.contains_key("plugins")),
            plugins_policy_ui_presentation_tools: ui_presentation
                .is_some_and(|table| table.contains_key("tools")),
        }
    }
}

pub(crate) fn validate_config_text(
    path: &Path,
    text: &str,
    env: &dyn ConfigEnvironment,
) -> Result<(), ConfigError> {
    let value = parse_config_value(path, text)?;
    reject_unsupported_fields_value(&value)?;
    let config =
        serde_json::from_value::<RawConfig>(value).map_err(|source| ConfigError::ParseFile {
            path: path.to_path_buf(),
            source,
        })?;
    config.resolve_with_env(env)?;
    Ok(())
}

fn parse_config_value(path: &Path, text: &str) -> Result<Value, ConfigError> {
    serde_json::from_str::<Value>(text).map_err(|source| ConfigError::ParseFile {
        path: path.to_path_buf(),
        source,
    })
}

fn reject_unsupported_fields_value(value: &Value) -> Result<(), ConfigError> {
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

    fn merge_project_from(&mut self, overlay: Self) {
        merge_option(&mut self.default, overlay.default);
        for (provider_id, provider) in overlay.providers {
            match self.providers.get_mut(&provider_id) {
                Some(existing) => existing.merge_project_from(provider),
                None => {
                    self.providers.insert(provider_id, provider);
                }
            }
        }
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

    fn merge_project_from(&mut self, overlay: Self) {
        merge_option(&mut self.default, overlay.default);
        self.agents.extend(overlay.agents);
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
    pub(crate) desktop: Option<RawDesktopConfig>,
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
        merge_option_struct(&mut self.desktop, overlay.desktop);
        merge_option_struct(&mut self.runtime, overlay.runtime);
        merge_option_struct(&mut self.session, overlay.session);
        merge_option_struct(&mut self.permission, overlay.permission);
        self.agents.merge_from(overlay.agents);
        merge_option_struct(&mut self.plugins, overlay.plugins);
        merge_option_struct(&mut self.harnesses, overlay.harnesses);
        self.providers.merge_from(overlay.providers);
    }

    /// Merge a project/workspace config layer.
    ///
    /// Project config is partial, but keyed entities use their natural key as
    /// the conflict boundary: `agents.<name>` and `plugins.list.<id>` replace
    /// the lower-priority entry, while provider `defaults` and `auth` replace
    /// as whole selection/auth tuples. This keeps project overrides from
    /// inheriting unrelated nested fields by accident.
    pub(crate) fn merge_project_from_with_keys(
        &mut self,
        overlay: Self,
        merge_keys: RawProjectMergeKeys,
    ) {
        merge_option_struct(&mut self.tracing, overlay.tracing);
        merge_option_struct(&mut self.ui, overlay.ui);
        merge_option_struct(&mut self.desktop, overlay.desktop);
        merge_option_struct(&mut self.runtime, overlay.runtime);
        merge_option_struct(&mut self.session, overlay.session);
        merge_option_struct(&mut self.permission, overlay.permission);
        self.agents.merge_project_from(overlay.agents);
        merge_project_plugins(&mut self.plugins, overlay.plugins, merge_keys);
        merge_option_struct(&mut self.harnesses, overlay.harnesses);
        self.providers.merge_project_from(overlay.providers);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tracing.is_none()
            && self.ui.is_none()
            && self.desktop.is_none()
            && self.runtime.is_none()
            && self.session.is_none()
            && self.permission.is_none()
            && self.agents.is_empty()
            && self.plugins.is_none()
            && self.harnesses.is_none()
            && self.providers.is_empty()
    }

    pub(crate) fn from_env(env: &dyn ConfigEnvironment) -> Result<Self, ConfigError> {
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
        let desktop =
            crate::config::types::DesktopConfig::from_raw(self.desktop.unwrap_or_default());

        let raw_runtime = self.runtime.unwrap_or_default();
        let raw_session = self.session.unwrap_or_default();
        let runtime = RuntimeConfig::from_raw(raw_runtime)?;
        let session = SessionConfig::from_raw(raw_session)?;
        let mut permission = crate::agent::PermissionConfig::global_default();
        if let Some(overlay) = self.permission {
            permission.merge_from(overlay);
        }
        let plugins: PluginConfig =
            crate::plugins::sources::resolve_plugin_config(self.plugins.unwrap_or_default());
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
            desktop,
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
        if agents
            .get(explicit_agent)
            .is_some_and(|agent| agent.disabled)
        {
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

fn merge_project_plugins(
    base: &mut Option<PluginConfig>,
    overlay: Option<PluginConfig>,
    merge_keys: RawProjectMergeKeys,
) {
    let Some(overlay) = overlay else {
        return;
    };
    match base.as_mut() {
        Some(base) => merge_project_plugin_config(base, overlay, merge_keys),
        None => *base = Some(overlay),
    }
}

fn merge_project_plugin_config(
    base: &mut PluginConfig,
    overlay: PluginConfig,
    merge_keys: RawProjectMergeKeys,
) {
    if merge_keys.plugins_host {
        merge_project_plugin_host(&mut base.host, overlay.host, merge_keys);
    }
    if merge_keys.plugins_policy {
        merge_project_plugin_policy(&mut base.policy, overlay.policy, merge_keys);
    }
    base.list.extend(overlay.list);
}

fn merge_project_plugin_host(
    base: &mut crate::plugin::PluginHostConfig,
    overlay: crate::plugin::PluginHostConfig,
    merge_keys: RawProjectMergeKeys,
) {
    if merge_keys.plugins_host_timeouts {
        merge_timeouts(&mut base.timeouts, overlay.timeouts);
    }
    if merge_keys.plugins_host_default_quota {
        base.default_quota = overlay.default_quota;
    }
    if merge_keys.plugins_host_quotas {
        base.quotas.extend(overlay.quotas);
    }
    if merge_keys.plugins_host_trusted_keys {
        base.trusted_keys.extend(overlay.trusted_keys);
    }
}

fn merge_project_plugin_policy(
    base: &mut crate::plugin::PluginPolicyConfig,
    overlay: crate::plugin::PluginPolicyConfig,
    merge_keys: RawProjectMergeKeys,
) {
    if merge_keys.plugins_policy_tool_presentation {
        merge_tool_presentation(
            &mut base.tool_presentation,
            overlay.tool_presentation,
            merge_keys,
        );
    }
    if merge_keys.plugins_policy_ui_presentation {
        merge_ui_presentation(
            &mut base.ui_presentation,
            overlay.ui_presentation,
            merge_keys,
        );
    }
}

fn merge_tool_presentation(
    base: &mut crate::plugin::ToolPresentationConfig,
    overlay: crate::plugin::ToolPresentationConfig,
    merge_keys: RawProjectMergeKeys,
) {
    if merge_keys.plugins_policy_tool_presentation_default_mode {
        base.default_mode = overlay.default_mode;
    }
    if merge_keys.plugins_policy_tool_presentation_plugins {
        base.plugins.extend(overlay.plugins);
    }
    if merge_keys.plugins_policy_tool_presentation_tools {
        base.tools.extend(overlay.tools);
    }
}

fn merge_ui_presentation(
    base: &mut crate::plugin::UiPresentationConfig,
    overlay: crate::plugin::UiPresentationConfig,
    merge_keys: RawProjectMergeKeys,
) {
    if merge_keys.plugins_policy_ui_presentation_default_mode {
        base.default_mode = overlay.default_mode;
    }
    if merge_keys.plugins_policy_ui_presentation_plugins {
        base.plugins.extend(overlay.plugins);
    }
    if merge_keys.plugins_policy_ui_presentation_tools {
        base.tools.extend(overlay.tools);
    }
}

fn merge_timeouts(
    base: &mut crate::plugin::TimeoutsConfig,
    overlay: crate::plugin::TimeoutsConfig,
) {
    if overlay.init.is_some() {
        base.init = overlay.init;
    }
    if overlay.tool_hook.is_some() {
        base.tool_hook = overlay.tool_hook;
    }
    if overlay.tool_invoke.is_some() {
        base.tool_invoke = overlay.tool_invoke;
    }
    if overlay.permission_ask.is_some() {
        base.permission_ask = overlay.permission_ask;
    }
    if overlay.chat.is_some() {
        base.chat = overlay.chat;
    }
    if overlay.fast.is_some() {
        base.fast = overlay.fast;
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawDesktopConfig {
    #[merge(strategy = option_override)]
    pub(crate) autostart_on_boot: Option<bool>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) backend: Option<RawDesktopBackendConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawDesktopBackendConfig {
    #[merge(strategy = option_override)]
    pub(crate) host: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) port: Option<u16>,
    #[merge(strategy = option_override)]
    pub(crate) ui_dir: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) cors_origins: Option<Vec<String>>,
    #[merge(strategy = option_override)]
    pub(crate) cors_allow_all: Option<bool>,
    #[merge(strategy = option_override)]
    pub(crate) backend_log_level: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) ui_password: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) ui_cookie_samesite: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) workspace_root: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) database_path: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) database_url: Option<String>,
}

impl crate::config::types::DesktopConfig {
    pub(crate) fn from_raw(raw: RawDesktopConfig) -> Self {
        let backend = raw.backend.unwrap_or_default();
        let mut config = Self {
            autostart_on_boot: raw.autostart_on_boot.unwrap_or(true),
            backend: crate::config::types::DesktopBackendConfig {
                host: normalize_host(backend.host.as_deref()),
                port: normalize_desktop_port(backend.port),
                ui_dir: normalize_optional_text(backend.ui_dir),
                cors_origins: normalize_cors_origins(backend.cors_origins.unwrap_or_default()),
                cors_allow_all: backend.cors_allow_all.unwrap_or(false),
                backend_log_level: normalize_log_level(backend.backend_log_level),
                ui_password: backend.ui_password.or_else(|| Some(String::new())),
                ui_cookie_samesite: normalize_ui_cookie_samesite(backend.ui_cookie_samesite),
                workspace_root: normalize_optional_text(backend.workspace_root),
                database_path: normalize_optional_text(backend.database_path),
                database_url: normalize_optional_text(backend.database_url),
            },
        };
        config.backend.ui_password = Some(
            config
                .backend
                .ui_password
                .take()
                .unwrap_or_default()
                .trim()
                .to_string(),
        );
        config
    }
}

fn normalize_optional_text(raw: Option<String>) -> Option<String> {
    let value = raw?.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn normalize_host(raw: Option<&str>) -> String {
    let value = raw.unwrap_or_default().trim();
    if value.is_empty() {
        "127.0.0.1".to_string()
    } else {
        value.to_string()
    }
}

fn normalize_desktop_port(raw: Option<u16>) -> u16 {
    match raw.unwrap_or(3210) {
        0 => 3210,
        value => value,
    }
}

fn normalize_cors_origins(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::<String>::new();
    for raw in values {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.iter().any(|value| value == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn normalize_log_level(raw: Option<String>) -> Option<String> {
    let level = raw?.trim().to_ascii_uppercase();
    match level.as_str() {
        "DEBUG" | "INFO" | "WARN" | "ERROR" => Some(level),
        _ => None,
    }
}

fn normalize_ui_cookie_samesite(raw: Option<String>) -> Option<String> {
    let value = raw?.trim().to_ascii_lowercase();
    match value.as_str() {
        "auto" | "strict" | "lax" | "none" => Some(value),
        _ => None,
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
    fn merge_project_from(&mut self, overlay: Self) {
        merge_option(&mut self.enabled, overlay.enabled);
        if overlay.defaults.is_some() {
            self.defaults = overlay.defaults;
        }
        if overlay.auth.is_some() {
            self.auth = overlay.auth;
        }
        for (adapter_id, adapter) in overlay.adapters {
            match self.adapters.get_mut(&adapter_id) {
                Some(existing) => existing.merge_from(adapter),
                None => {
                    self.adapters.insert(adapter_id, adapter);
                }
            }
        }
    }

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
        normalize_provider_model_native_tools(&auth, &adapters, &mut models);
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
        let default_model = normalize_optional_string(provider_defaults.model.clone());
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
        if let Some(default_model) = default_model.as_deref() {
            let default_route = format!("{default_adapter_id}/{default_model}");
            if matches!(models.get(default_route.as_str()), Some(configured) if !configured.enabled)
            {
                return Err(ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.clone(),
                    message: format!(
                        "provider defaults.model `{default_model}` references disabled model route `{default_route}`"
                    ),
                });
            }
        }

        let resolved_provider_id = provider_id.clone();

        Ok((
            provider_id,
            ResolvedProviderConfig {
                enabled,
                defaults: ProviderDefaultsConfig {
                    provider: Some(resolved_provider_id),
                    adapter: Some(default_adapter_id),
                    model: default_model,
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

fn normalize_provider_model_native_tools(
    auth: &ProviderAuthConfig,
    adapters: &BTreeMap<String, ResolvedProviderAdapterConfig>,
    models: &mut BTreeMap<String, ResolvedProviderModelConfig>,
) {
    let uses_openai_chatgpt = matches!(
        auth,
        ProviderAuthConfig::Credential(config) if config.issuer == CredentialIssuer::OpenaiChatgpt
    );
    if !uses_openai_chatgpt {
        return;
    }

    for (route_id, model) in models.iter_mut() {
        let Some((adapter_id, _model_id)) = route_id.split_once('/') else {
            continue;
        };
        let Some(adapter) = adapters.get(adapter_id) else {
            continue;
        };
        let ProviderAdapterDefinition::OpenAi(config) = &adapter.definition else {
            continue;
        };
        if !matches!(
            config.options.backend,
            super::OpenAiBackendConfig::ChatgptCodex
        ) {
            continue;
        }
        normalize_openai_chatgpt_native_tools(&mut model.native_tools);
    }
}

fn normalize_openai_chatgpt_native_tools(config: &mut ProviderNativeToolsConfig) {
    if !openai_chatgpt_native_tools_match_legacy_default(config) {
        return;
    }

    *config = ProviderNativeToolsConfig {
        enabled: true,
        routes: super::ProviderNativeToolRoutesConfig {
            web_search: Some(ProviderNativeToolRoute::ProviderHosted),
            image_generation: Some(ProviderNativeToolRoute::ProviderHosted),
            ..Default::default()
        },
        ..Default::default()
    };
}

fn openai_chatgpt_native_tools_match_legacy_default(config: &ProviderNativeToolsConfig) -> bool {
    if !config.enabled
        || !config.hosted.is_empty()
        || !config.harness.is_empty()
        || !config.connectors.is_empty()
    {
        return false;
    }

    let routes = &config.routes;
    routes.web_search == Some(ProviderNativeToolRoute::ProviderHosted)
        && routes.image_generation.is_none()
        && matches!(
            (routes.file_search, routes.code_execution),
            (None, None)
                | (None, Some(ProviderNativeToolRoute::ProviderHosted))
                | (
                    Some(ProviderNativeToolRoute::ProviderHosted),
                    Some(ProviderNativeToolRoute::ProviderHosted)
                )
        )
        && routes.computer.is_none()
        && routes.bash.is_none()
        && routes.text_editor.is_none()
        && routes.url_context.is_none()
        && routes.remote_mcp.is_none()
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
                    auth_header: normalize_optional(auth_header)
                        .or_else(|| Some("x-goog-api-key".to_owned())),
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
    RawDesktopConfig,
    RawDesktopBackendConfig,
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
            "{scope} names cannot be empty"
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
            format!("{scope} model `{model_id}` think modes").as_str(),
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

    fn raw_config(value: serde_json::Value) -> RawConfig {
        serde_json::from_value::<RawConfig>(value).expect("config should parse")
    }

    #[test]
    fn project_merge_replaces_agents_by_name() {
        let mut global = raw_config(json!({
            "agents": {
                "default": "review",
                "review": {
                    "description": "Global review agent",
                    "prompt": "Global prompt",
                    "permission": {
                        "tools": {
                            "names": {
                                "shell": "allow"
                            }
                        }
                    }
                }
            }
        }));
        let project = raw_config(json!({
            "agents": {
                "review": {
                    "description": "Project review agent"
                }
            }
        }));

        global.merge_project_from_with_keys(project, RawProjectMergeKeys::default());
        let resolved = global
            .resolve_with_env(&TestEnvironment)
            .expect("project merged config should resolve");
        let review = resolved.agents.get("review").expect("review agent");

        assert_eq!(resolved.default_agent.as_deref(), Some("review"));
        assert_eq!(review.description, "Project review agent");
        assert!(review.prompt.is_empty());
        assert!(review.permission.is_empty());
    }

    #[test]
    fn project_merge_replaces_provider_defaults_as_selection_tuple() {
        let mut global = raw_config(json!({
            "providers": {
                "default": "openai",
                "openai": {
                    "defaults": {
                        "adapter": "openai",
                        "model": "gpt-4.1",
                        "thinking_mode": "high",
                        "speed_mode": "fast"
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
                                "gpt-4.1": {},
                                "gpt-5": {}
                            }
                        }
                    }
                }
            }
        }));
        let project = raw_config(json!({
            "providers": {
                "openai": {
                    "defaults": {
                        "adapter": "openai",
                        "model": "gpt-5"
                    }
                }
            }
        }));

        global.merge_project_from_with_keys(project, RawProjectMergeKeys::default());
        let resolved = global
            .resolve_with_env(&TestEnvironment)
            .expect("provider config should still inherit auth and adapters");

        assert_eq!(resolved.default_selection.model.as_deref(), Some("gpt-5"));
        assert_eq!(resolved.default_selection.thinking_mode, None);
        assert_eq!(resolved.default_selection.speed_mode, None);
        assert!(
            resolved
                .providers
                .get("openai")
                .is_some_and(|provider| provider.adapters.contains_key("openai"))
        );
    }

    #[test]
    fn provider_defaults_model_can_be_omitted() {
        let resolved = resolve_config(json!({
            "providers": {
                "default": "local",
                "local": {
                    "defaults": {
                        "adapter": "ollama"
                    },
                    "auth": {
                        "mode": "none"
                    },
                    "adapters": {
                        "ollama": {
                            "enabled": true
                        }
                    }
                }
            }
        }))
        .expect("provider config should resolve without defaults.model");

        let provider = resolved.providers.get("local").expect("provider");
        assert_eq!(provider.defaults.adapter.as_deref(), Some("ollama"));
        assert_eq!(provider.defaults.model, None);
    }

    #[test]
    fn project_merge_replaces_plugins_by_plugin_id() {
        let mut global = raw_config(json!({
            "plugins": {
                "list": {
                    "agena.web": {
                        "package": {
                            "kind": "static"
                        },
                        "config": {
                            "source": "global"
                        }
                    },
                    "agena.memory": {
                        "package": {
                            "kind": "static"
                        },
                        "config": {
                            "source": "global"
                        }
                    }
                }
            }
        }));
        let project = raw_config(json!({
            "plugins": {
                "list": {
                    "agena.web": {
                        "package": {
                            "kind": "static"
                        },
                        "config": {
                            "source": "project"
                        }
                    }
                }
            }
        }));

        global.merge_project_from_with_keys(project, RawProjectMergeKeys::default());
        let plugins = &global.plugins.as_ref().expect("plugins").list;

        assert_eq!(plugins.len(), 2);
        assert_eq!(
            plugins
                .get("agena.web")
                .and_then(|plugin| plugin.config().get("source"))
                .and_then(serde_json::Value::as_str),
            Some("project")
        );
        assert_eq!(
            plugins
                .get("agena.memory")
                .and_then(|plugin| plugin.config().get("source"))
                .and_then(serde_json::Value::as_str),
            Some("global")
        );
    }

    #[test]
    fn project_merge_can_reset_plugin_policy_to_default_when_policy_key_is_present() {
        let mut global = raw_config(json!({
            "plugins": {
                "policy": {
                    "tool_presentation": {
                        "default_mode": "brief"
                    }
                }
            }
        }));
        let project = raw_config(json!({
            "plugins": {
                "policy": {
                    "tool_presentation": {
                        "default_mode": "detailed"
                    }
                }
            }
        }));

        global.merge_project_from_with_keys(
            project,
            RawProjectMergeKeys {
                plugins_policy: true,
                plugins_policy_tool_presentation: true,
                plugins_policy_tool_presentation_default_mode: true,
                ..RawProjectMergeKeys::default()
            },
        );
        let policy = &global.plugins.as_ref().expect("plugins").policy;

        assert_eq!(
            policy.tool_presentation.default_mode,
            crate::plugin::ToolDescriptionMode::Detailed
        );
    }

    #[test]
    fn project_merge_merges_plugin_host_by_nested_keys() {
        let mut global = raw_config(json!({
            "plugins": {
                "host": {
                    "timeouts": {
                        "init": "10s",
                        "tool_invoke": "60s"
                    },
                    "default_quota": {
                        "rate_per_sec": 5,
                        "burst": 10,
                        "max_concurrent": 2
                    },
                    "quotas": {
                        "agena.web": {
                            "rate_per_sec": 1
                        },
                        "agena.memory": {
                            "rate_per_sec": 2
                        }
                    },
                    "trusted_keys": {
                        "global": "aaaa"
                    }
                }
            }
        }));
        let project = raw_config(json!({
            "plugins": {
                "host": {
                    "timeouts": {
                        "tool_invoke": "30s"
                    },
                    "quotas": {
                        "agena.web": {
                            "rate_per_sec": 9,
                            "max_concurrent": 1
                        }
                    },
                    "trusted_keys": {
                        "project": "bbbb"
                    }
                }
            }
        }));

        global.merge_project_from_with_keys(
            project,
            RawProjectMergeKeys {
                plugins_host: true,
                plugins_host_timeouts: true,
                plugins_host_quotas: true,
                plugins_host_trusted_keys: true,
                ..RawProjectMergeKeys::default()
            },
        );
        let host = &global.plugins.as_ref().expect("plugins").host;

        assert!(host.timeouts.init.is_some());
        assert_eq!(
            host.timeouts
                .tool_invoke
                .as_ref()
                .map(|value| value.0.as_secs()),
            Some(30)
        );
        assert_eq!(host.default_quota.rate_per_sec, 5);
        assert_eq!(
            host.quotas.get("agena.web").map(|quota| quota.rate_per_sec),
            Some(9)
        );
        assert_eq!(
            host.quotas
                .get("agena.web")
                .map(|quota| quota.max_concurrent),
            Some(1)
        );
        assert_eq!(
            host.quotas
                .get("agena.memory")
                .map(|quota| quota.rate_per_sec),
            Some(2)
        );
        assert_eq!(
            host.trusted_keys.get("global").map(String::as_str),
            Some("aaaa")
        );
        assert_eq!(
            host.trusted_keys.get("project").map(String::as_str),
            Some("bbbb")
        );
    }

    #[test]
    fn project_merge_merges_tool_presentation_by_plugin_and_tool_keys() {
        let mut global = raw_config(json!({
            "plugins": {
                "policy": {
                    "tool_presentation": {
                        "default_mode": "brief",
                        "plugins": {
                            "agena.web": "brief",
                            "agena.memory": "brief"
                        },
                        "tools": {
                            "bash": "brief"
                        }
                    }
                }
            }
        }));
        let project = raw_config(json!({
            "plugins": {
                "policy": {
                    "tool_presentation": {
                        "plugins": {
                            "agena.web": "detailed"
                        },
                        "tools": {
                            "read": "detailed"
                        }
                    }
                }
            }
        }));

        global.merge_project_from_with_keys(
            project,
            RawProjectMergeKeys {
                plugins_policy: true,
                plugins_policy_tool_presentation: true,
                plugins_policy_tool_presentation_plugins: true,
                plugins_policy_tool_presentation_tools: true,
                ..RawProjectMergeKeys::default()
            },
        );
        let presentation = &global
            .plugins
            .as_ref()
            .expect("plugins")
            .policy
            .tool_presentation;

        assert_eq!(
            presentation.default_mode,
            crate::plugin::ToolDescriptionMode::Brief
        );
        assert_eq!(
            presentation.plugins.get("agena.web").copied(),
            Some(crate::plugin::ToolDescriptionOverride::Detailed)
        );
        assert_eq!(
            presentation.plugins.get("agena.memory").copied(),
            Some(crate::plugin::ToolDescriptionOverride::Brief)
        );
        assert_eq!(
            presentation.tools.get("bash").copied(),
            Some(crate::plugin::ToolDescriptionOverride::Brief)
        );
        assert_eq!(
            presentation.tools.get("read").copied(),
            Some(crate::plugin::ToolDescriptionOverride::Detailed)
        );
    }

    #[test]
    fn project_merge_merges_ui_presentation_by_plugin_and_tool_keys() {
        let mut global = raw_config(json!({
            "plugins": {
                "policy": {
                    "ui_presentation": {
                        "default_mode": "summary",
                        "plugins": {
                            "agena.web": "summary",
                            "agena.memory": "summary"
                        },
                        "tools": {
                            "web.fetch": "summary"
                        }
                    }
                }
            }
        }));
        let project = raw_config(json!({
            "plugins": {
                "policy": {
                    "ui_presentation": {
                        "plugins": {
                            "agena.web": "detailed"
                        },
                        "tools": {
                            "web.open": "detailed"
                        }
                    }
                }
            }
        }));

        global.merge_project_from_with_keys(
            project,
            RawProjectMergeKeys {
                plugins_policy: true,
                plugins_policy_ui_presentation: true,
                plugins_policy_ui_presentation_plugins: true,
                plugins_policy_ui_presentation_tools: true,
                ..RawProjectMergeKeys::default()
            },
        );
        let presentation = &global
            .plugins
            .as_ref()
            .expect("plugins")
            .policy
            .ui_presentation;

        assert_eq!(
            presentation.default_mode,
            crate::plugin::UiTextDisplayMode::Summary
        );
        assert_eq!(
            presentation.plugins.get("agena.web").copied(),
            Some(crate::plugin::UiPresentationOverride::Detailed)
        );
        assert_eq!(
            presentation.plugins.get("agena.memory").copied(),
            Some(crate::plugin::UiPresentationOverride::Summary)
        );
        assert_eq!(
            presentation.tools.get("web.fetch").copied(),
            Some(crate::plugin::UiPresentationOverride::Summary)
        );
        assert_eq!(
            presentation.tools.get("web.open").copied(),
            Some(crate::plugin::UiPresentationOverride::Detailed)
        );
    }

    #[test]
    fn project_merge_merges_permission_and_harnesses_by_natural_keys() {
        let mut global = raw_config(json!({
            "permission": {
                "path": {
                    "rules": {
                        "src/**": "read"
                    }
                },
                "network": {
                    "rules": {
                        "api.example.com": "allow"
                    }
                },
                "tools": {
                    "names": {
                        "bash": "ask"
                    }
                }
            },
            "harnesses": {
                "shell": {
                    "local": {
                        "allow_commands": ["git status"]
                    }
                }
            }
        }));
        let project = raw_config(json!({
            "permission": {
                "path": {
                    "rules": {
                        "src/**": "deny",
                        "docs/**": "read"
                    }
                },
                "network": {
                    "rules": {
                        "api.example.com": "deny"
                    }
                },
                "tools": {
                    "names": {
                        "bash": "deny"
                    }
                }
            },
            "harnesses": {
                "shell": {
                    "local": {
                        "deny_commands": ["rm -rf *"]
                    }
                }
            }
        }));

        global.merge_project_from_with_keys(project, RawProjectMergeKeys::default());
        let permission = global.permission.as_ref().expect("permission");
        let path_rules = &permission.path.as_ref().expect("path permission").rules;
        let network_rules = &permission
            .network
            .as_ref()
            .expect("network permission")
            .rules;
        let tool_names = &permission.tools.as_ref().expect("tool permission").names;
        let shell = &global.harnesses.as_ref().expect("harnesses").shell["local"];

        assert!(matches!(
            path_rules.get("src/**"),
            Some(crate::agent::PathAccessRuleConfig::Shorthand(value)) if value == "deny"
        ));
        assert!(path_rules.contains_key("docs/**"));
        assert_eq!(
            network_rules.get("api.example.com"),
            Some(&crate::permission::PermissionMode::Deny)
        );
        assert_eq!(
            tool_names.get("bash"),
            Some(&crate::permission::PermissionMode::Deny)
        );
        assert!(shell.allow_commands.is_empty());
        assert_eq!(shell.deny_commands, vec!["rm -rf *"]);
    }

    #[test]
    fn resolve_config_accepts_desktop_section() {
        let resolved = resolve_config(json!({
            "desktop": {
                "autostart_on_boot": false,
                "backend": {
                    "host": " 0.0.0.0 ",
                    "port": 0,
                    "cors_origins": [" https://studio.example ", "https://studio.example"],
                    "backend_log_level": "debug",
                    "ui_password": " secret ",
                    "ui_cookie_samesite": "STRICT",
                    "workspace_root": " /tmp/workspace ",
                    "database_path": " /tmp/agena.db ",
                    "database_url": " sqlite:///tmp/agena.db ",
                    "ui_dir": " /tmp/dist "
                }
            }
        }))
        .expect("desktop config should resolve");

        assert!(!resolved.desktop.autostart_on_boot);
        assert_eq!(resolved.desktop.backend.host, "0.0.0.0");
        assert_eq!(resolved.desktop.backend.port, 3210);
        assert_eq!(
            resolved.desktop.backend.cors_origins,
            vec!["https://studio.example".to_string()]
        );
        assert_eq!(
            resolved.desktop.backend.backend_log_level.as_deref(),
            Some("DEBUG")
        );
        assert_eq!(
            resolved.desktop.backend.ui_password.as_deref(),
            Some("secret")
        );
        assert_eq!(
            resolved.desktop.backend.ui_cookie_samesite.as_deref(),
            Some("strict")
        );
        assert_eq!(
            resolved.desktop.backend.workspace_root.as_deref(),
            Some("/tmp/workspace")
        );
        assert_eq!(
            resolved.desktop.backend.database_path.as_deref(),
            Some("/tmp/agena.db")
        );
        assert_eq!(
            resolved.desktop.backend.database_url.as_deref(),
            Some("sqlite:///tmp/agena.db")
        );
        assert_eq!(
            resolved.desktop.backend.ui_dir.as_deref(),
            Some("/tmp/dist")
        );
    }

    #[test]
    fn resolve_config_applies_global_permission_defaults() {
        let resolved = resolve_config(json!({})).expect("empty config should resolve");
        let path = resolved.permission.path.expect("path defaults");
        let workspace = path.workspace.expect("workspace path defaults");
        let external = path.external.expect("external path defaults");
        assert_eq!(
            workspace.read,
            Some(crate::permission::PermissionMode::Allow)
        );
        assert_eq!(
            workspace.write,
            Some(crate::permission::PermissionMode::Ask)
        );
        assert_eq!(external.read, Some(crate::permission::PermissionMode::Ask));
        assert_eq!(external.write, Some(crate::permission::PermissionMode::Ask));

        let network = resolved.permission.network.expect("network defaults");
        assert_eq!(
            network.internet,
            Some(crate::permission::PermissionMode::Ask)
        );
        assert_eq!(
            network.private,
            Some(crate::permission::PermissionMode::Ask)
        );
        assert_eq!(
            network.loopback,
            Some(crate::permission::PermissionMode::Ask)
        );

        let tools = resolved.permission.tools.expect("tool defaults");
        assert_eq!(tools.default, Some(crate::permission::PermissionMode::Ask));
        assert_eq!(
            tools.tags.get("filesystem_read").copied(),
            Some(crate::permission::PermissionMode::Allow)
        );
    }

    #[test]
    fn resolve_config_merges_explicit_permission_over_global_defaults() {
        let resolved = resolve_config(json!({
            "permission": {
                "path": {
                    "external": {
                        "read": "allow"
                    }
                },
                "tools": {
                    "names": {
                        "shell": "allow"
                    }
                }
            }
        }))
        .expect("partial permission config should resolve");

        let path = resolved.permission.path.expect("path defaults");
        assert_eq!(
            path.workspace.and_then(|modes| modes.read),
            Some(crate::permission::PermissionMode::Allow)
        );
        let external = path.external.expect("external path defaults");
        assert_eq!(
            external.read,
            Some(crate::permission::PermissionMode::Allow)
        );
        assert_eq!(external.write, Some(crate::permission::PermissionMode::Ask));

        let tools = resolved.permission.tools.expect("tool defaults");
        assert_eq!(tools.default, Some(crate::permission::PermissionMode::Ask));
        assert_eq!(
            tools.tags.get("filesystem_read").copied(),
            Some(crate::permission::PermissionMode::Allow)
        );
        assert_eq!(
            tools.names.get("shell").copied(),
            Some(crate::permission::PermissionMode::Allow)
        );
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
                            "runtime": {
                                "token_store": {
                                    "enabled": true
                                }
                            },
                            "servers": {
                                "docs": {
                                    "transport": "http",
                                    "endpoint": {
                                        "url": "https://example.com/mcp"
                                    }
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
    fn provider_hosted_file_search_without_vector_stores_is_ignored_in_bindings() {
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
                            "enabled": true,
                            "models": {
                                "gpt-5": {
                                    "native_tools": {
                                        "enabled": true,
                                        "routes": {
                                            "web_search": "provider_hosted",
                                            "file_search": "provider_hosted"
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

        let provider = resolved.providers.get("openai").expect("provider");
        let model = provider
            .models
            .get("openai/gpt-5")
            .expect("configured model");
        let bindings = model.native_tool_bindings();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].tool, ProviderNativeToolKind::WebSearch);
    }

    #[test]
    fn chatgpt_codex_native_tools_legacy_defaults_are_normalized() {
        let resolved = resolve_config(json!({
            "providers": {
                "default": "openai_chatgpt",
                "openai_chatgpt": {
                    "defaults": {
                        "adapter": "openai",
                        "model": "gpt-5.5"
                    },
                    "auth": {
                        "mode": "credential",
                        "issuer": "openai_chatgpt"
                    },
                    "adapters": {
                        "openai": {
                            "enabled": true,
                            "backend": "chatgpt_codex",
                            "models": {
                                "gpt-5.5": {
                                    "native_tools": {
                                        "enabled": true,
                                        "routes": {
                                            "web_search": "provider_hosted",
                                            "file_search": "provider_hosted",
                                            "code_execution": "provider_hosted"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }))
        .expect("legacy chatgpt codex native tools should resolve");

        let provider = resolved
            .providers
            .get("openai_chatgpt")
            .expect("provider should resolve");
        let model = provider
            .models
            .get("openai/gpt-5.5")
            .expect("model should resolve");

        assert!(model.native_tools.enabled);
        assert_eq!(
            model.native_tools.routes.web_search,
            Some(ProviderNativeToolRoute::ProviderHosted)
        );
        assert_eq!(model.native_tools.routes.file_search, None);
        assert_eq!(model.native_tools.routes.code_execution, None);
        assert_eq!(
            model.native_tools.routes.image_generation,
            Some(ProviderNativeToolRoute::ProviderHosted)
        );
    }

    #[test]
    fn resolves_default_agent_name_from_external_registry() {
        let resolved = resolve_config(json!({
            "agents": {
                "default": "build"
            }
        }))
        .expect("agents.default should allow default or disk-discovered agent profiles");

        assert_eq!(resolved.default_agent.as_deref(), Some("build"));
    }

    #[test]
    fn rejects_disabled_config_agent_as_default() {
        let error = resolve_config(json!({
            "agents": {
                "default": "planner",
                "planner": {
                    "disabled": true,
                    "prompt": "Plan the next steps."
                }
            }
        }))
        .expect_err("disabled config-backed agents must not be selectable defaults");

        assert!(
            error
                .to_string()
                .contains("agents.default `planner` references disabled agent"),
            "unexpected error: {error}"
        );
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

    #[test]
    fn gemini_adapter_defaults_to_x_goog_api_key_header() {
        let resolved = resolve_config(json!({
            "providers": {
                "default": "google",
                "google": {
                    "defaults": {
                        "adapter": "gemini",
                        "model": "gemini-2.5-pro"
                    },
                    "auth": {
                        "mode": "api",
                        "base_url": "https://generativelanguage.googleapis.com",
                        "api_key": "test"
                    },
                    "adapters": {
                        "gemini": {
                            "enabled": true
                        }
                    }
                }
            }
        }))
        .expect("gemini provider should resolve");

        let provider = resolved.providers.get("google").expect("provider");
        let adapter = provider.adapters.get("gemini").expect("gemini adapter");
        let ProviderAdapterDefinition::Gemini(config) = &adapter.definition else {
            panic!("expected gemini adapter");
        };
        assert_eq!(
            config.options.auth_header.as_deref(),
            Some("x-goog-api-key")
        );
        assert_eq!(config.options.auth_scheme, None);
    }
}
