use std::{collections::BTreeMap, path::Path};

use merge::Merge as DeriveMerge;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::RuntimeTracingConfiguration;
use agena_plugin_host::PluginsConfig as PluginConfig;
use agena_provider::{
    ConfiguredModelSpeedMode, ConfiguredModelThinkingMode, CredentialIssuer,
    HostedCodeExecutionContainerOverlay, ProviderAdapterOverlay,
    ProviderHostedCodeExecutionOverlay, ProviderHostedFileSearchOverlay,
    ProviderHostedImageGenerationOverlay, ProviderHostedToolsOverlay,
    ProviderHostedUrlContextOverlay, ProviderHostedWebSearchOverlay,
    ProviderNativeToolConnectorOverlay, ProviderNativeToolHarnessBindingsOverlay,
    ProviderNativeToolHarnessRefOverlay, ProviderNativeToolRoutesOverlay,
    ProviderNativeToolUserLocationOverlay, ProviderNativeToolsOverlay, ProviderOverlay,
};

use crate::{
    ConfigEnvironment, ConfigError, HttpProviderAdapterConfig, ProviderAdapterDefinition,
    ProviderApiAuthConfig, ProviderAuthConfig, ResolvedConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, RuntimeConfig, RuntimeProvidersConfig, SessionCompactionConfig,
    SessionConfig, UiConfig,
};

pub use crate::merge_optional_config as merge_option;
pub use crate::normalize_config_optional as normalize_optional;
pub use crate::normalize_config_optional as normalize_optional_string;

mod raw_provider;

use raw_provider::ProviderOverlayExt;

const DEFAULT_LOG_FILTER: &str = "info";
const DEFAULT_DATABASE_LOG_LEVEL: &str = "error";

#[derive(Debug, Clone)]
/// A raw config file together with whether it was found and its merge keys.
pub struct RawConfigFile {
    pub config: RawConfig,
    pub found: bool,
    pub merge_keys: RawProjectMergeKeys,
}

impl RawConfigFile {
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        match crate::read_config_json(path)? {
            Some(value) => {
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
            None => Ok(Self {
                config: RawConfig::default(),
                found: false,
                merge_keys: RawProjectMergeKeys::default(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
/// Which top-level project config sections participate in merging.
pub struct RawProjectMergeKeys {
    plugins_host: bool,
    plugins_host_timeouts: bool,
    plugins_host_default_quota: bool,
    plugins_host_quotas: bool,
    plugins_host_trusted_keys: bool,
}

impl RawProjectMergeKeys {
    fn from_value(value: &Value) -> Self {
        let plugins = value.get("plugins").and_then(Value::as_object);
        let plugins_host = plugins
            .and_then(|table| table.get("host"))
            .and_then(Value::as_object);
        Self {
            plugins_host: plugins.is_some_and(|table| table.contains_key("host")),
            plugins_host_timeouts: plugins_host.is_some_and(|table| table.contains_key("timeouts")),
            plugins_host_default_quota: plugins_host
                .is_some_and(|table| table.contains_key("default_quota")),
            plugins_host_quotas: plugins_host.is_some_and(|table| table.contains_key("quotas")),
            plugins_host_trusted_keys: plugins_host
                .is_some_and(|table| table.contains_key("trusted_keys")),
        }
    }
}

pub fn validate_config_text(path: &Path, text: &str) -> Result<(), ConfigError> {
    let (config, _) = parse_raw_config_text(path, text)?;
    config.resolve()?;
    Ok(())
}

pub fn validate_layered_config_text(
    global_path: &Path,
    workspace_path: &Path,
    edited_layer: super::ConfigSettingsLayer,
    edited_text: &str,
    env: &dyn ConfigEnvironment,
) -> Result<(), ConfigError> {
    let edited_path = match edited_layer {
        super::ConfigSettingsLayer::Global => global_path,
        super::ConfigSettingsLayer::Workspace => workspace_path,
    };
    let (edited, edited_merge_keys) = parse_raw_config_text(edited_path, edited_text)?;
    let (global, workspace, workspace_merge_keys) = match edited_layer {
        super::ConfigSettingsLayer::Global => {
            let file = RawConfigFile::read(workspace_path)?;
            (edited, file.config, file.merge_keys)
        }
        super::ConfigSettingsLayer::Workspace => (
            RawConfigFile::read(global_path)?.config,
            edited,
            edited_merge_keys,
        ),
    };

    let mut merged = global;
    merged.merge_project_from_with_keys(workspace, workspace_merge_keys);
    merged.merge_from(RawConfig::from_env(env)?);
    merged.resolve()?;
    Ok(())
}

fn parse_raw_config_text(
    path: &Path,
    text: &str,
) -> Result<(RawConfig, RawProjectMergeKeys), ConfigError> {
    let value = crate::parse_config_json(path, text)?;
    let merge_keys = RawProjectMergeKeys::from_value(&value);
    let config =
        serde_json::from_value::<RawConfig>(value).map_err(|source| ConfigError::ParseFile {
            path: path.to_path_buf(),
            source,
        })?;
    Ok((config, merge_keys))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
/// Raw provider configuration with one global model selection and per-provider overlays.
pub struct RawProvidersConfig {
    #[merge(strategy = option_override)]
    pub default_selection: Option<agena_domain::ModelSelectionConfig>,
    #[serde(flatten)]
    #[merge(strategy = map_extend)]
    pub providers: BTreeMap<String, ProviderOverlay>,
}

impl RawProvidersConfig {
    fn is_empty(&self) -> bool {
        self.default_selection.is_none() && self.providers.is_empty()
    }

    fn merge_project_from(&mut self, overlay: Self) {
        merge_option(&mut self.default_selection, overlay.default_selection);
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
        merge_option(&mut self.default_selection, overlay.default_selection);
        merge_map(&mut self.providers, overlay.providers);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Raw, unvalidated configuration as parsed from file or environment.
pub struct RawConfig {
    pub tracing: Option<RawTracingConfig>,
    pub ui: Option<RawUiConfig>,
    pub runtime: Option<RawRuntimeConfig>,
    pub session: Option<RawSessionConfig>,
    pub permission: Option<agena_domain::PermissionConfig>,
    pub plugins: Option<PluginConfig>,
    pub providers: RawProvidersConfig,
}

impl RawConfig {
    pub fn merge_from(&mut self, overlay: Self) {
        merge_option_struct(&mut self.tracing, overlay.tracing);
        merge_option_struct(&mut self.ui, overlay.ui);
        merge_option_struct(&mut self.runtime, overlay.runtime);
        merge_option_struct(&mut self.session, overlay.session);
        merge_option_struct(&mut self.permission, overlay.permission);
        merge_option_struct(&mut self.plugins, overlay.plugins);
        self.providers.merge_from(overlay.providers);
    }

    /// Merge a project/workspace config layer.
    ///
    /// Project config is partial, but keyed entities use their natural key as
    /// the conflict boundary: `plugins.list.<id>` replaces
    /// the lower-priority entry, while provider `auth` replaces as a whole
    /// authentication tuple. This keeps project overrides from inheriting
    /// unrelated nested fields by accident.
    pub fn merge_project_from_with_keys(&mut self, overlay: Self, merge_keys: RawProjectMergeKeys) {
        merge_option_struct(&mut self.tracing, overlay.tracing);
        merge_option_struct(&mut self.ui, overlay.ui);
        merge_option_struct(&mut self.runtime, overlay.runtime);
        merge_option_struct(&mut self.session, overlay.session);
        merge_option_struct(&mut self.permission, overlay.permission);
        merge_project_plugins(&mut self.plugins, overlay.plugins, merge_keys);
        self.providers.merge_project_from(overlay.providers);
    }

    pub fn is_empty(&self) -> bool {
        self.tracing.is_none()
            && self.ui.is_none()
            && self.runtime.is_none()
            && self.session.is_none()
            && self.permission.is_none()
            && self.plugins.is_none()
            && self.providers.is_empty()
    }

    pub fn from_env(env: &dyn ConfigEnvironment) -> Result<Self, ConfigError> {
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
        let ui = locale.map(|locale| RawUiConfig {
            locale: Some(locale),
            ..Default::default()
        });

        let codex = env.var("AGENA_CODEX_CLIENT_VERSION");
        let claude = env.var("AGENA_CLAUDE_CLIENT_VERSION");
        let gemini = env.var("AGENA_GEMINI_CLIENT_VERSION");
        let client_versions = (codex.is_some() || claude.is_some() || gemini.is_some()).then_some(
            RawProviderClientVersionSettings {
                codex,
                claude,
                gemini,
                auto_update: None,
            },
        );
        let runtime = client_versions.map(|client_versions| RawRuntimeConfig {
            providers: Some(RawRuntimeProvidersConfig {
                client_versions: Some(client_versions),
            }),
            ..Default::default()
        });

        let compaction_auto = env
            .var("AGENA_SESSION_COMPACTION_AUTO")
            .map(|value| crate::parse_config_bool("AGENA_SESSION_COMPACTION_AUTO", value.as_str()))
            .transpose()?;
        let mut compaction_reserved_tokens = None;
        crate::apply_config_env_number(env, "AGENA_SESSION_COMPACTION_RESERVED_TOKENS", |value| {
            compaction_reserved_tokens = Some(value)
        })?;
        let mut session_max_turns = None;
        crate::apply_config_env_number(env, "AGENA_SESSION_MAX_TURNS", |value| {
            session_max_turns = Some(value)
        })?;
        let session = (compaction_auto.is_some()
            || compaction_reserved_tokens.is_some()
            || session_max_turns.is_some())
        .then_some(RawSessionConfig {
            compaction: Some(RawSessionCompactionConfig {
                auto: compaction_auto,
                reserved_tokens: compaction_reserved_tokens,
            }),
            max_turns: session_max_turns,
            ..Default::default()
        });

        Ok(Self {
            tracing,
            ui,
            runtime,
            session,
            permission: None,
            plugins: None,
            providers: RawProvidersConfig::default(),
        })
    }

    pub fn resolve(self) -> Result<ResolvedConfig, ConfigError> {
        let raw_tracing = self.tracing.unwrap_or_default();
        let database = raw_tracing
            .database
            .unwrap_or_else(|| DEFAULT_DATABASE_LOG_LEVEL.to_owned());
        validate_database_log_level(database.as_str())?;
        let adapter = raw_tracing
            .adapter
            .unwrap_or_else(|| RuntimeTracingConfiguration::default().adapter);
        validate_tracing_level("tracing.adapter", adapter.as_str())?;
        let tracing = RuntimeTracingConfiguration {
            filter: raw_tracing
                .filter
                .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_owned()),
            database,
            adapter,
        };
        let raw_ui = self.ui.unwrap_or_default();
        let ui = UiConfig {
            locale: raw_ui
                .locale
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            preferences: raw_ui.preferences,
        };
        let runtime = RuntimeConfig::from_raw(self.runtime.unwrap_or_default())?;
        let session = SessionConfig::from_raw(self.session.unwrap_or_default());
        if session.auto_title.message_interval == 0 {
            return Err(ConfigError::Validation(
                "session.auto_title.message_interval must be greater than zero".into(),
            ));
        }
        let mut permission = agena_domain::PermissionConfig::global_default();
        if let Some(overlay) = self.permission {
            permission.merge_from(overlay);
        }
        let plugins: PluginConfig = self.plugins.unwrap_or_default();
        crate::mcp_config_from_plugins(&plugins).map_err(ConfigError::Validation)?;

        let explicit_default_selection = self.providers.default_selection.clone();
        let providers = self
            .providers
            .providers
            .into_iter()
            .map(|(provider_id, raw)| raw.resolve(provider_id))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let default_selection = resolve_model_selection(
            "providers.default_selection",
            explicit_default_selection.as_ref(),
            &providers,
        )?;
        resolve_model_selection(
            "runtime.auxiliary_model",
            runtime.auxiliary_model.as_ref(),
            &providers,
        )?;
        validate_permission_config("permission", &permission)?;

        Ok(ResolvedConfig {
            default_selection,
            tracing,
            ui,
            runtime,
            session,
            permission,
            plugins,
            providers,
        })
    }
}

fn resolve_model_selection(
    path: &str,
    explicit_selection: Option<&agena_domain::ModelSelectionConfig>,
    providers: &BTreeMap<String, ResolvedProviderConfig>,
) -> Result<agena_domain::ExecutionSelection, ConfigError> {
    let Some(selection) = explicit_selection else {
        return Ok(agena_domain::ExecutionSelection::default());
    };

    if selection.is_empty() {
        return Ok(agena_domain::ExecutionSelection::default());
    }

    let provider_id = selection
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ConfigError::Validation(format!("`{path}` must specify a provider and model"))
        })?;
    let provider = providers.get(provider_id).ok_or_else(|| {
        ConfigError::Validation(format!(
            "{path} `{provider_id}` references unknown provider"
        ))
    })?;
    if !provider.enabled {
        return Err(ConfigError::Validation(format!(
            "{path} `{provider_id}` references disabled provider"
        )));
    }

    if let Some(adapter_id) = selection
        .adapter
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let adapter = provider.adapters.get(adapter_id).ok_or_else(|| {
            ConfigError::Validation(format!(
                "{path} adapter `{adapter_id}` references unknown adapter for provider `{provider_id}`"
            ))
        })?;
        if !adapter.enabled {
            return Err(ConfigError::Validation(format!(
                "{path} adapter `{adapter_id}` references disabled adapter for provider `{provider_id}`"
            )));
        }
    }

    let model = selection
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ConfigError::Validation(format!("`{path}` must specify a provider and model"))
        })?;

    Ok(agena_domain::ExecutionSelection {
        provider: Some(provider_id.to_owned()),
        adapter: selection.adapter.clone(),
        model: Some(model.to_owned()),
        thinking_mode: selection.thinking_mode.clone(),
        speed_mode: selection.speed_mode.clone(),
        verbosity: selection.verbosity.clone(),
        parallel_tool_calls: selection.parallel_tool_calls,
        ..Default::default()
    })
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
    base.list.extend(overlay.list);
}

fn merge_project_plugin_host(
    base: &mut agena_plugin_host::PluginHostConfig,
    overlay: agena_plugin_host::PluginHostConfig,
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

fn merge_timeouts(
    base: &mut agena_plugin_host::TimeoutsConfig,
    overlay: agena_plugin_host::TimeoutsConfig,
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
    if overlay.chat.is_some() {
        base.chat = overlay.chat;
    }
    if overlay.fast.is_some() {
        base.fast = overlay.fast;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Raw tracing configuration.
pub struct RawTracingConfig {
    #[merge(strategy = option_override)]
    pub filter: Option<String>,
    #[merge(strategy = option_override)]
    pub database: Option<String>,
    #[merge(strategy = option_override)]
    pub adapter: Option<String>,
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

/// Opaque client preference entries with generic recursive overlay semantics.
#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default)]
pub struct RawUiConfig {
    #[merge(strategy = option_override)]
    pub locale: Option<String>,
    #[serde(flatten)]
    #[merge(strategy = preference_map_merge)]
    pub preferences: BTreeMap<String, serde_json::Value>,
}

fn preference_map_merge(
    base: &mut BTreeMap<String, serde_json::Value>,
    overlay: BTreeMap<String, serde_json::Value>,
) {
    fn merge_value(base: &mut serde_json::Value, overlay: serde_json::Value) {
        match (base, overlay) {
            (serde_json::Value::Object(base), serde_json::Value::Object(overlay)) => {
                for (key, value) in overlay {
                    if let Some(existing) = base.get_mut(&key) {
                        merge_value(existing, value);
                    } else {
                        base.insert(key, value);
                    }
                }
            }
            (base, overlay) => *base = overlay,
        }
    }
    for (key, value) in overlay {
        if let Some(existing) = base.get_mut(&key) {
            merge_value(existing, value);
        } else {
            base.insert(key, value);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Raw runtime configuration.
pub struct RawRuntimeConfig {
    #[merge(strategy = option_struct_merge)]
    pub providers: Option<RawRuntimeProvidersConfig>,
    #[merge(strategy = option_override)]
    pub auxiliary_model: Option<agena_domain::ModelSelectionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Raw runtime provider settings.
pub struct RawRuntimeProvidersConfig {
    #[merge(strategy = option_struct_merge)]
    pub client_versions: Option<RawProviderClientVersionSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Raw provider client version settings.
pub struct RawProviderClientVersionSettings {
    #[merge(strategy = option_override)]
    pub codex: Option<String>,
    #[merge(strategy = option_override)]
    pub claude: Option<String>,
    #[merge(strategy = option_override)]
    pub gemini: Option<String>,
    #[merge(strategy = option_override)]
    pub auto_update: Option<bool>,
}

impl RuntimeConfig {
    pub fn from_raw(raw: RawRuntimeConfig) -> Result<Self, ConfigError> {
        let client_versions = raw
            .providers
            .unwrap_or_default()
            .client_versions
            .unwrap_or_default();
        let defaults = crate::ProviderClientVersionSettings::default();
        Ok(Self {
            auxiliary_model: raw
                .auxiliary_model
                .filter(|selection| !selection.is_empty()),
            providers: RuntimeProvidersConfig {
                client_versions: crate::ProviderClientVersionSettings {
                    codex: normalize_provider_client_version(
                        "runtime.providers.client_versions.codex",
                        client_versions.codex,
                        defaults.codex.as_str(),
                    )?,
                    claude: normalize_provider_client_version(
                        "runtime.providers.client_versions.claude",
                        client_versions.claude,
                        defaults.claude.as_str(),
                    )?,
                    gemini: normalize_provider_client_version(
                        "runtime.providers.client_versions.gemini",
                        client_versions.gemini,
                        defaults.gemini.as_str(),
                    )?,
                    auto_update: client_versions.auto_update.unwrap_or(true),
                },
            },
        })
    }
}

fn normalize_provider_client_version(
    path: &str,
    value: Option<String>,
    default: &str,
) -> Result<String, ConfigError> {
    let Some(value) = value else {
        return Ok(default.to_owned());
    };
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-+_".contains(character))
    {
        return Err(ConfigError::Validation(format!(
            "{path} must be a version containing only ASCII letters, numbers, dot, dash, plus, or underscore"
        )));
    }
    Ok(value.to_owned())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Raw session configuration.
pub struct RawSessionConfig {
    #[merge(strategy = option_struct_merge)]
    pub compaction: Option<RawSessionCompactionConfig>,
    #[merge(strategy = option_struct_merge)]
    pub auto_title: Option<RawSessionAutoTitleConfig>,
    /// Cap on model turns within one stable run. `0` means unlimited;
    /// `None` falls back to `DEFAULT_MAX_MODEL_TURNS` (500).
    #[merge(strategy = option_override)]
    pub max_turns: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub struct RawSessionAutoTitleConfig {
    #[merge(strategy = option_override)]
    pub enabled: Option<bool>,
    #[merge(strategy = option_override)]
    pub message_interval: Option<u64>,
    #[merge(strategy = option_override)]
    pub cooldown_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
/// Raw session compaction configuration.
pub struct RawSessionCompactionConfig {
    #[merge(strategy = option_override)]
    pub auto: Option<bool>,
    #[merge(strategy = option_override)]
    pub reserved_tokens: Option<u32>,
}

impl SessionConfig {
    pub fn from_raw(raw: RawSessionConfig) -> Self {
        let compaction = raw.compaction.unwrap_or_default();
        let title = raw.auto_title.unwrap_or_default();
        let defaults = crate::SessionAutoTitleConfig::default();
        Self {
            auto_title: crate::SessionAutoTitleConfig {
                enabled: title.enabled.unwrap_or(defaults.enabled),
                message_interval: title.message_interval.unwrap_or(defaults.message_interval),
                cooldown_seconds: title.cooldown_seconds.unwrap_or(defaults.cooldown_seconds),
            },
            compaction: SessionCompactionConfig {
                auto: compaction.auto.unwrap_or(true),
                reserved_tokens: compaction.reserved_tokens,
            },
            max_turns: raw.max_turns,
        }
    }
}

impl Merge for agena_domain::PermissionConfig {
    fn merge_from(&mut self, overlay: Self) {
        self.merge_from(overlay);
    }
}

impl Merge for agena_domain::PermissionMode {
    fn merge_from(&mut self, overlay: Self) {
        *self = overlay;
    }
}

// PluginConfig (alias for agena_plugin_host::PluginsConfig) is parsed
// directly via serde; no `from_raw` adapter needed.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// Provider protocol family used by an adapter.
pub enum ProviderKind {
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "openai_responses")]
    OpenAiResponses,
    #[serde(rename = "openai_chat_completions")]
    OpenAiChatCompletions,
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
            "openai_responses" => Ok(Self::OpenAiResponses),
            "openai_chat_completions" => Ok(Self::OpenAiChatCompletions),
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

/// Merges an overlay value into a base value.
pub trait Merge {
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
    RawProviderClientVersionSettings,
    RawSessionConfig,
    RawSessionAutoTitleConfig,
    RawSessionCompactionConfig,
    ProviderNativeToolRoutesOverlay,
    ProviderNativeToolUserLocationOverlay,
    ProviderHostedWebSearchOverlay,
    ProviderHostedFileSearchOverlay,
    HostedCodeExecutionContainerOverlay,
    ProviderHostedCodeExecutionOverlay,
    ProviderHostedImageGenerationOverlay,
    ProviderHostedUrlContextOverlay,
    ProviderHostedToolsOverlay,
    ProviderNativeToolHarnessRefOverlay,
    ProviderNativeToolHarnessBindingsOverlay,
    ProviderNativeToolConnectorOverlay,
    ProviderNativeToolsOverlay,
    ProviderAdapterOverlay,
    ProviderOverlay,
);

pub fn option_override<T>(base: &mut Option<T>, overlay: Option<T>) {
    merge_option(base, overlay);
}

pub fn merge_option_struct<T>(base: &mut Option<T>, overlay: Option<T>)
where
    T: Merge,
{
    match (base.as_mut(), overlay) {
        (Some(base), Some(overlay)) => base.merge_from(overlay),
        (None, Some(overlay)) => *base = Some(overlay),
        _ => {}
    }
}

pub fn option_struct_merge<T>(base: &mut Option<T>, overlay: Option<T>)
where
    T: Merge,
{
    merge_option_struct(base, overlay);
}

pub fn merge_map<T>(base: &mut BTreeMap<String, T>, overlay: BTreeMap<String, T>)
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

pub fn map_extend<K, V>(base: &mut BTreeMap<K, V>, overlay: BTreeMap<K, V>)
where
    K: Ord,
{
    base.extend(overlay);
}

pub fn option_map_extend<K, V>(base: &mut Option<BTreeMap<K, V>>, overlay: Option<BTreeMap<K, V>>)
where
    K: Ord,
{
    match (base.as_mut(), overlay) {
        (Some(base), Some(overlay)) => base.extend(overlay),
        (None, Some(overlay)) => *base = Some(overlay),
        _ => {}
    }
}

fn validate_permission_config(
    label: &str,
    permission: &agena_domain::PermissionConfig,
) -> Result<(), ConfigError> {
    agena_runtime_contracts::authorization::validate_permission_config(permission)
        .map(|_| ())
        .map_err(|err| ConfigError::Validation(format!("{label} is invalid: {err}")))
}

fn validate_configured_models(
    provider_id: &str,
    scope: &str,
    models: &BTreeMap<String, agena_provider::ResolvedProviderModelConfig>,
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
    thinking_modes: &agena_provider::ConfiguredModelModeMap<ConfiguredModelThinkingMode>,
    speed_scope: &str,
    speed_modes: &agena_provider::ConfiguredModelModeMap<ConfiguredModelSpeedMode>,
) -> Result<(), ConfigError> {
    for (name, mode) in thinking_modes.iter() {
        if name.trim().is_empty() || name == "default" {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {thinking_scope} contains an invalid or reserved mode name"
            )));
        }
        let resolved = agena_provider::configured_thinking_mode_to_model(name, mode);
        if resolved.thinking.is_none()
            && !matches!(
                mode.strategy,
                Some(agena_provider::ConfiguredThinkingStrategy::RequestOnly)
            )
        {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {thinking_scope} mode `{name}` needs an explicit strategy and all fields required by that strategy"
            )));
        }
        if matches!(
            mode.strategy,
            Some(agena_provider::ConfiguredThinkingStrategy::RequestOnly)
        ) && mode.request_override.is_empty()
            && mode.adapter_overrides.is_empty()
        {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {thinking_scope} request-only mode `{name}` needs a request override"
            )));
        }
    }
    if let Some(name) = thinking_modes.default.mode() {
        let mode = thinking_modes.get(name).ok_or_else(|| {
            ConfigError::Validation(format!(
                "provider `{provider_id}` {thinking_scope} default references missing mode `{name}`"
            ))
        })?;
        if mode.disabled {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {thinking_scope} default mode `{name}` is disabled"
            )));
        }
    }
    for name in speed_modes.keys() {
        if name.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {speed_scope} mode name cannot be empty"
            )));
        }
    }
    if let Some(name) = speed_modes.default.mode() {
        let mode = speed_modes.get(name).ok_or_else(|| {
            ConfigError::Validation(format!(
                "provider `{provider_id}` {speed_scope} default references missing mode `{name}`"
            ))
        })?;
        if mode.disabled {
            return Err(ConfigError::Validation(format!(
                "provider `{provider_id}` {speed_scope} default mode `{name}` is disabled"
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

pub fn parse_adapter_model_ref(
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
