use std::{collections::BTreeMap, fs, path::Path};

use merge::Merge as DeriveMerge;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::{
    ConfiguredModelSpeedMode, ConfiguredModelThinkingMode, ProviderRequestRetryConfig,
    ProviderStreamReplayConfig, auth::CredentialIssuer,
};

use super::{
    AgentConfig, ConfigEnvironment, ConfigError, HarnessViewportConfig, HarnessesConfig,
    HttpProviderAdapterConfig, OpenAiApiModeConfig, PluginConfig, ProviderAdapterDefinition,
    ProviderAdapterOverlay, ProviderApiAuthConfig, ProviderApiSubtype, ProviderAuthConfig,
    ProviderAuthMode, ProviderAuthOverlay, ProviderCapabilityFamilyConfig,
    ProviderCredentialAuthConfig, ProviderDefaultsConfig, ProviderGitlabApiAccessConfig,
    ProviderGitlabApiAccessOverlay, ProviderGitlabCredentialAuthConfig, ProviderHostedToolConfigs,
    ProviderHttpCredentialAuthConfig, ProviderInlineCredentialAuthConfig,
    ProviderModelDiscoveryConfig, ProviderModelOverlay, ProviderNativeToolKind,
    ProviderNativeToolRoute, ProviderNativeToolsConfig, ProviderOverlay,
    ProviderProtocolPathsConfig, ProviderProtocolPathsOverlay,
    ProviderSapAiCoreCredentialAuthConfig, ProviderSecretSourceConfig, ProviderSecretSourceOverlay,
    ResolvedConfig, ResolvedProviderAdapterConfig, ResolvedProviderConfig,
    ResolvedProviderModelConfig, RuntimeConfig, RuntimeGcConfig, RuntimeModelCatalogConfig,
    RuntimeProvidersConfig, RuntimeSessionConfig, SessionCacheConfig, SessionCompactionConfig,
    SessionConfig, StreamTransportMode, TracingConfig, TuiColorSchemeConfig, TuiUiConfig, UiConfig,
};

mod raw_provider;

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
        let tracing = {
            let filter = env.var("AGENA_LOG");
            let database = env.var("AGENA_DATABASE_LOG");
            let adapter = env.var("AGENA_ADAPTER_LOG");
            (filter.is_some() || database.is_some() || adapter.is_some()).then_some(
                RawTracingConfig {
                    filter,
                    database,
                    adapter,
                },
            )
        };

        let locale = env.var("AGENA_LOCALE");
        let tui_color_scheme = env
            .var("AGENA_TUI_COLOR_SCHEME")
            .map(|value| {
                value
                    .parse::<TuiColorSchemeConfig>()
                    .map_err(ConfigError::Validation)
            })
            .transpose()?;
        let tui_theme = env.var("AGENA_TUI_THEME");
        let ui = (locale.is_some() || tui_color_scheme.is_some() || tui_theme.is_some()).then_some(
            RawUiConfig {
                locale,
                tui: (tui_color_scheme.is_some() || tui_theme.is_some()).then_some(
                    RawTuiUiConfig {
                        color_scheme: tui_color_scheme,
                        theme: tui_theme,
                    },
                ),
            },
        );

        let mut timeout_secs = None;
        let mut connect_timeout_secs = None;
        let mut max_retries = None;
        let mut base_delay_ms = None;
        let mut max_delay_ms = None;
        let mut max_retries_after_output = None;
        let mut max_tracked_events = None;
        let codex_client_version = env.var("AGENA_CODEX_CLIENT_VERSION");
        let claude_client_version = env.var("AGENA_CLAUDE_CLIENT_VERSION");
        let gemini_client_version = env.var("AGENA_GEMINI_CLIENT_VERSION");
        let mut model_catalog_cache_max_age_secs = None;
        let mut session_compaction_auto = None;
        let mut session_compaction_reserved_tokens = None;

        apply_env_number(env, "AGENA_PROVIDER_HTTP_TIMEOUT_SECS", |value| {
            timeout_secs = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_CONNECT_TIMEOUT_SECS", |value| {
            connect_timeout_secs = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_REQUEST_MAX_RETRIES", |value| {
            max_retries = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_RETRY_BASE_DELAY_MS", |value| {
            base_delay_ms = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_RETRY_MAX_DELAY_MS", |value| {
            max_delay_ms = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_STREAM_REPLAY_MAX_RETRIES", |value| {
            max_retries_after_output = Some(value);
        })?;
        apply_env_number(env, "AGENA_PROVIDER_STREAM_REPLAY_MAX_EVENTS", |value| {
            max_tracked_events = Some(value);
        })?;
        apply_env_number(env, "AGENA_MODEL_CATALOG_CACHE_MAX_AGE_SECS", |value| {
            model_catalog_cache_max_age_secs = Some(value);
        })?;
        if let Some(enabled) = env.var("AGENA_SESSION_COMPACTION_AUTO") {
            session_compaction_auto = Some(parse_bool(
                "AGENA_SESSION_COMPACTION_AUTO",
                enabled.as_str(),
            )?);
        }
        apply_env_number(env, "AGENA_SESSION_COMPACTION_RESERVED_TOKENS", |value| {
            session_compaction_reserved_tokens = Some(value);
        })?;

        let http = (timeout_secs.is_some() || connect_timeout_secs.is_some()).then_some(
            RawProviderHttpConfig {
                timeout_secs,
                connect_timeout_secs,
            },
        );
        let retry = (max_retries.is_some() || base_delay_ms.is_some() || max_delay_ms.is_some())
            .then_some(RawRequestRetryConfig {
                max_retries,
                base_delay_ms,
                max_delay_ms,
            });
        let stream_replay = (max_retries_after_output.is_some() || max_tracked_events.is_some())
            .then_some(RawStreamReplayConfig {
                max_retries_after_output,
                max_tracked_events,
            });
        let client_versions = (codex_client_version.is_some()
            || claude_client_version.is_some()
            || gemini_client_version.is_some())
        .then_some(RawProviderClientVersionSettings {
            codex: codex_client_version,
            claude: claude_client_version,
            gemini: gemini_client_version,
        });
        let providers = (client_versions.is_some()
            || http.is_some()
            || retry.is_some()
            || stream_replay.is_some())
        .then_some(RawRuntimeProvidersConfig {
            client_versions,
            http,
            retry,
            stream_replay,
        });
        let model_catalog = model_catalog_cache_max_age_secs.map(|cache_max_age_secs| {
            RawRuntimeModelCatalogConfig {
                cache_max_age_secs: Some(cache_max_age_secs),
            }
        });
        let runtime =
            (providers.is_some() || model_catalog.is_some()).then_some(RawRuntimeConfig {
                providers,
                model_catalog,
                reload: None,
                session: None,
            });

        let session = (session_compaction_auto.is_some()
            || session_compaction_reserved_tokens.is_some())
        .then_some(RawSessionConfig {
            compaction: Some(RawSessionCompactionConfig {
                auto: session_compaction_auto,
                reserved_tokens: session_compaction_reserved_tokens,
            }),
        });

        Ok(Self {
            tracing,
            ui,
            desktop: None,
            runtime,
            session,
            permission: None,
            agents: RawAgentsConfig::default(),
            plugins: None,
            harnesses: None,
            providers: RawProvidersConfig::default(),
        })
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
        let raw_ui = self.ui.unwrap_or_default();
        let raw_tui = raw_ui.tui.unwrap_or_default();
        let ui = UiConfig {
            locale: raw_ui
                .locale
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            tui: TuiUiConfig {
                color_scheme: raw_tui.color_scheme.unwrap_or_default(),
                theme: raw_tui
                    .theme
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
            },
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
    #[merge(strategy = option_struct_merge)]
    pub(crate) tui: Option<RawTuiUiConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawTuiUiConfig {
    #[merge(strategy = option_override)]
    pub(crate) color_scheme: Option<TuiColorSchemeConfig>,
    #[merge(strategy = option_override)]
    pub(crate) theme: Option<String>,
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
        let ui_password = Some(backend.ui_password.unwrap_or_default().trim().to_string());
        Self {
            autostart_on_boot: raw.autostart_on_boot.unwrap_or(true),
            backend: crate::config::types::DesktopBackendConfig {
                host: normalize_host(backend.host.as_deref()),
                port: normalize_desktop_port(backend.port),
                ui_dir: normalize_optional_text(backend.ui_dir),
                cors_origins: normalize_cors_origins(backend.cors_origins.unwrap_or_default()),
                cors_allow_all: backend.cors_allow_all.unwrap_or(false),
                backend_log_level: normalize_log_level(backend.backend_log_level),
                ui_password,
                ui_cookie_samesite: normalize_ui_cookie_samesite(backend.ui_cookie_samesite),
                workspace_root: normalize_optional_text(backend.workspace_root),
                database_path: normalize_optional_text(backend.database_path),
                database_url: normalize_optional_text(backend.database_url),
            },
        }
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
    pub(crate) client_versions: Option<RawProviderClientVersionSettings>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) http: Option<RawProviderHttpConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) retry: Option<RawRequestRetryConfig>,
    #[merge(strategy = option_struct_merge)]
    pub(crate) stream_replay: Option<RawStreamReplayConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RawProviderClientVersionSettings {
    #[merge(strategy = option_override)]
    pub(crate) codex: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) claude: Option<String>,
    #[merge(strategy = option_override)]
    pub(crate) gemini: Option<String>,
}

impl RuntimeConfig {
    pub(crate) fn from_raw(raw: RawRuntimeConfig) -> Result<Self, ConfigError> {
        let providers = raw.providers.unwrap_or_default();
        let client_versions = providers.client_versions.unwrap_or_default();
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
        let client_versions = super::ProviderClientVersionSettings {
            codex: normalize_provider_client_version(
                "runtime.providers.client_versions.codex",
                client_versions.codex,
            )?,
            claude: normalize_provider_client_version(
                "runtime.providers.client_versions.claude",
                client_versions.claude,
            )?,
            gemini: normalize_provider_client_version(
                "runtime.providers.client_versions.gemini",
                client_versions.gemini,
            )?,
        };

        Ok(Self {
            providers: RuntimeProvidersConfig {
                client_versions,
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

fn normalize_provider_client_version(
    path: &str,
    value: Option<String>,
) -> Result<String, ConfigError> {
    let Some(value) = value else {
        return Ok("auto".to_owned());
    };
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto") {
        return Ok("auto".to_owned());
    }
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-+_".contains(character))
    {
        return Err(ConfigError::Validation(format!(
            "{path} must be `auto` or a version containing only ASCII letters, numbers, dot, dash, plus, or underscore"
        )));
    }
    Ok(value.to_owned())
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
    RawTuiUiConfig,
    RawDesktopConfig,
    RawDesktopBackendConfig,
    RawRuntimeConfig,
    RawRuntimeProvidersConfig,
    RawProviderClientVersionSettings,
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
        crate::permission::ToolPermissionPolicy::allow_all(),
    )
    .try_apply_permission_config(permission)
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
