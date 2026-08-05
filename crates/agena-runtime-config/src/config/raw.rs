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
    ConfigEnvironment, ConfigError, HarnessViewportConfig, HarnessesConfig,
    HttpProviderAdapterConfig, ProviderAdapterDefinition, ProviderApiAuthConfig,
    ProviderAuthConfig, ProviderDefaultsConfig, ResolvedConfig, ResolvedProviderAdapterConfig,
    ResolvedProviderConfig, RuntimeConfig, RuntimeProvidersConfig, SessionCompactionConfig,
    SessionConfig, TuiColorSchemeConfig, TuiGraphicsModeConfig, TuiUiConfig, UiConfig,
};
use agena_domain::ModelSelectionConfig;

pub use crate::merge_optional_config as merge_option;
pub use crate::normalize_config_optional as normalize_optional;
pub use crate::normalize_config_optional as normalize_optional_string;

mod raw_provider;

use raw_provider::ProviderOverlayExt;

const DEFAULT_LOG_FILTER: &str = "info";
const DEFAULT_DATABASE_LOG_LEVEL: &str = "error";

#[derive(Debug, Clone)]
pub struct RawConfigFile {
    pub config: RawConfig,
    pub found: bool,
    pub merge_keys: RawProjectMergeKeys,
}

impl RawConfigFile {
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        match crate::read_config_json(path)? {
            Some(mut value) => {
                strip_legacy_provider_default_variants(&mut value);
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
            None => Ok(Self {
                config: RawConfig::default(),
                found: false,
                merge_keys: RawProjectMergeKeys::default(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RawProjectMergeKeys {
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

pub fn validate_config_text(
    path: &Path,
    text: &str,
    env: &dyn ConfigEnvironment,
) -> Result<(), ConfigError> {
    let (config, _) = parse_raw_config_text(path, text)?;
    config.resolve_with_env(env)?;
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
    merged.resolve_with_env(env)?;
    Ok(())
}

fn parse_raw_config_text(
    path: &Path,
    text: &str,
) -> Result<(RawConfig, RawProjectMergeKeys), ConfigError> {
    let mut value = crate::parse_config_json(path, text)?;
    strip_legacy_provider_default_variants(&mut value);
    reject_unsupported_fields_value(&value)?;
    let merge_keys = RawProjectMergeKeys::from_value(&value);
    let config =
        serde_json::from_value::<RawConfig>(value).map_err(|source| ConfigError::ParseFile {
            path: path.to_path_buf(),
            source,
        })?;
    Ok((config, merge_keys))
}

/// Provider defaults used to carry request variants such as thinking mode,
/// verbosity, and parallel tool calls. Those values now belong to a model
/// capability or an explicit session/run option. Silently discard the legacy
/// keys while loading so an existing user configuration keeps working, but do
/// not let the removed fields enter the typed configuration or effective
/// configuration output.
fn strip_legacy_provider_default_variants(value: &mut Value) {
    let Some(providers) = value.get_mut("providers").and_then(Value::as_object_mut) else {
        return;
    };
    for provider in providers.values_mut() {
        let Some(defaults) = provider
            .as_object_mut()
            .and_then(|provider| provider.get_mut("defaults"))
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        for field in [
            "thinking_mode",
            "speed_mode",
            "verbosity",
            "parallel_tool_calls",
        ] {
            defaults.remove(field);
        }
    }
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
                || provider.contains_key("thinking_variants")
                || provider.contains_key("thinking_modes")
                || provider.contains_key("speed_modes")
            {
                return Err(ConfigError::Validation(format!(
                    "provider `{provider_id}` model modes must be configured under `providers.{provider_id}.adapters.<adapter-id>.models.\"<model-id>\".thinking_modes` or `.speed_modes`; provider-level modes are not supported"
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
                            for field in [
                                "target_model",
                                "default_thinking_mode",
                                "thinking_variants",
                                "default_thinking_variant",
                            ] {
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
pub struct RawProvidersConfig {
    #[merge(strategy = option_override)]
    pub default: Option<String>,
    /// Explicit global model route and variants selected in the settings
    /// model picker. `default` remains the legacy provider-only selector.
    #[merge(strategy = option_override)]
    pub default_selection: Option<ModelSelectionConfig>,
    #[serde(flatten)]
    #[merge(strategy = map_extend)]
    pub providers: BTreeMap<String, ProviderOverlay>,
}

impl RawProvidersConfig {
    fn is_empty(&self) -> bool {
        self.default.is_none() && self.default_selection.is_none() && self.providers.is_empty()
    }

    fn merge_project_from(&mut self, overlay: Self) {
        merge_option(&mut self.default, overlay.default);
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
        merge_option(&mut self.default, overlay.default);
        merge_option(&mut self.default_selection, overlay.default_selection);
        merge_map(&mut self.providers, overlay.providers);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RawConfig {
    pub tracing: Option<RawTracingConfig>,
    pub ui: Option<RawUiConfig>,
    pub runtime: Option<RawRuntimeConfig>,
    pub session: Option<RawSessionConfig>,
    pub permission: Option<agena_domain::PermissionConfig>,
    pub plugins: Option<PluginConfig>,
    pub harnesses: Option<HarnessesConfig>,
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
        merge_option_struct(&mut self.harnesses, overlay.harnesses);
        self.providers.merge_from(overlay.providers);
    }

    /// Merge a project/workspace config layer.
    ///
    /// Project config is partial, but keyed entities use their natural key as
    /// the conflict boundary: `plugins.list.<id>` replaces
    /// the lower-priority entry, while provider `defaults` and `auth` replace
    /// as whole selection/auth tuples. This keeps project overrides from
    /// inheriting unrelated nested fields by accident.
    pub fn merge_project_from_with_keys(&mut self, overlay: Self, merge_keys: RawProjectMergeKeys) {
        merge_option_struct(&mut self.tracing, overlay.tracing);
        merge_option_struct(&mut self.ui, overlay.ui);
        merge_option_struct(&mut self.runtime, overlay.runtime);
        merge_option_struct(&mut self.session, overlay.session);
        merge_option_struct(&mut self.permission, overlay.permission);
        merge_project_plugins(&mut self.plugins, overlay.plugins, merge_keys);
        merge_option_struct(&mut self.harnesses, overlay.harnesses);
        self.providers.merge_project_from(overlay.providers);
    }

    pub fn is_empty(&self) -> bool {
        self.tracing.is_none()
            && self.ui.is_none()
            && self.runtime.is_none()
            && self.session.is_none()
            && self.permission.is_none()
            && self.plugins.is_none()
            && self.harnesses.is_none()
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
        let tui_color_scheme = env
            .var("AGENA_TUI_COLOR_SCHEME")
            .map(|value| {
                value
                    .parse::<TuiColorSchemeConfig>()
                    .map_err(ConfigError::Validation)
            })
            .transpose()?;
        let tui_theme = env.var("AGENA_TUI_THEME");
        let tui_graphics = env
            .var("AGENA_TUI_GRAPHICS")
            .map(|value| {
                value
                    .parse::<TuiGraphicsModeConfig>()
                    .map_err(ConfigError::Validation)
            })
            .transpose()?;
        let ui = (locale.is_some()
            || tui_color_scheme.is_some()
            || tui_graphics.is_some()
            || tui_theme.is_some())
        .then_some(RawUiConfig {
            locale,
            tui: (tui_color_scheme.is_some() || tui_graphics.is_some() || tui_theme.is_some())
                .then_some(RawTuiUiConfig {
                    color_scheme: tui_color_scheme,
                    graphics: tui_graphics,
                    theme: tui_theme,
                }),
        });

        let codex = env.var("AGENA_CODEX_CLIENT_VERSION");
        let claude = env.var("AGENA_CLAUDE_CLIENT_VERSION");
        let gemini = env.var("AGENA_GEMINI_CLIENT_VERSION");
        let client_versions = (codex.is_some() || claude.is_some() || gemini.is_some()).then_some(
            RawProviderClientVersionSettings {
                codex,
                claude,
                gemini,
            },
        );
        let runtime = client_versions.map(|client_versions| RawRuntimeConfig {
            providers: Some(RawRuntimeProvidersConfig {
                client_versions: Some(client_versions),
            }),
        });

        let compaction_auto = env
            .var("AGENA_SESSION_COMPACTION_AUTO")
            .map(|value| crate::parse_config_bool("AGENA_SESSION_COMPACTION_AUTO", value.as_str()))
            .transpose()?;
        let mut compaction_reserved_tokens = None;
        crate::apply_config_env_number(env, "AGENA_SESSION_COMPACTION_RESERVED_TOKENS", |value| {
            compaction_reserved_tokens = Some(value)
        })?;
        let session = (compaction_auto.is_some() || compaction_reserved_tokens.is_some())
            .then_some(RawSessionConfig {
                compaction: Some(RawSessionCompactionConfig {
                    auto: compaction_auto,
                    reserved_tokens: compaction_reserved_tokens,
                }),
            });

        Ok(Self {
            tracing,
            ui,
            runtime,
            session,
            permission: None,
            plugins: None,
            harnesses: None,
            providers: RawProvidersConfig::default(),
        })
    }

    pub fn resolve_with_env(
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
        let raw_tui = raw_ui.tui.unwrap_or_default();
        let ui = UiConfig {
            locale: raw_ui
                .locale
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            tui: TuiUiConfig {
                color_scheme: raw_tui.color_scheme.unwrap_or_default(),
                graphics: raw_tui.graphics.unwrap_or_default(),
                theme: raw_tui
                    .theme
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
            },
        };
        let runtime = RuntimeConfig::from_raw(self.runtime.unwrap_or_default())?;
        let session = SessionConfig::from_raw(self.session.unwrap_or_default());
        let mut permission = agena_domain::PermissionConfig::global_default();
        if let Some(overlay) = self.permission {
            permission.merge_from(overlay);
        }
        let plugins: PluginConfig = self.plugins.unwrap_or_default();
        let mcp = crate::mcp_config_from_plugins(&plugins).map_err(ConfigError::Validation)?;
        let harnesses: HarnessesConfig = self.harnesses.unwrap_or_default();
        let providers_default = self.providers.default.clone();
        let explicit_default_selection = self.providers.default_selection.clone();

        validate_harnesses(&harnesses)?;

        let providers = self
            .providers
            .providers
            .into_iter()
            .map(|(provider_id, raw)| raw.resolve(provider_id, env, &harnesses, &mcp))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let default_selection = resolve_default_selection(
            providers_default.as_deref(),
            explicit_default_selection.as_ref(),
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
            harnesses,
            providers,
        })
    }
}

fn resolve_default_selection(
    explicit_provider: Option<&str>,
    explicit_selection: Option<&agena_domain::ModelSelectionConfig>,
    providers: &BTreeMap<String, ResolvedProviderConfig>,
) -> Result<agena_domain::ExecutionSelection, ConfigError> {
    let selected_provider = explicit_selection
        .and_then(|selection| selection.provider.as_deref())
        .or(explicit_provider);
    let provider_id = if let Some(explicit_provider) = selected_provider
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
        return Ok(agena_domain::ExecutionSelection::default());
    };
    let provider = providers.get(provider_id.as_str()).ok_or_else(|| {
        ConfigError::Validation(format!(
            "providers.default `{provider_id}` references unknown provider"
        ))
    })?;

    // A `default_selection` can pin the adapter independently of the
    // provider's `defaults.adapter`. When that adapter is not enabled the
    // selection would point at a route the runtime never builds, so the
    // effective model reference resolves to a "provider has no enabled
    // adapter" failure at run time instead of a load-time error. Reject the
    // reference here so config validation surfaces it (the provider-level
    // `defaults.adapter` check in `raw_provider.rs` cannot see it).
    if let Some(selection_adapter) = explicit_selection
        .and_then(|selection| selection.adapter.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let resolved_adapter = provider
            .adapters
            .get(selection_adapter)
            .ok_or_else(|| {
                ConfigError::Validation(format!(
                    "providers.default_selection adapter `{selection_adapter}` references unknown adapter for provider `{provider_id}`"
                ))
            })?;
        if !resolved_adapter.enabled {
            return Err(ConfigError::Validation(format!(
                "providers.default_selection adapter `{selection_adapter}` references disabled adapter for provider `{provider_id}`"
            )));
        }
    }

    Ok(agena_domain::ExecutionSelection {
        provider: Some(provider_id),
        adapter: explicit_selection
            .and_then(|selection| selection.adapter.clone())
            .or_else(|| provider.defaults.adapter.clone()),
        model: explicit_selection
            .and_then(|selection| selection.model.clone())
            .or_else(|| provider.defaults.model.clone()),
        thinking_mode: explicit_selection.and_then(|selection| selection.thinking_mode.clone()),
        speed_mode: explicit_selection.and_then(|selection| selection.speed_mode.clone()),
        verbosity: explicit_selection.and_then(|selection| selection.verbosity.clone()),
        parallel_tool_calls: explicit_selection.and_then(|selection| selection.parallel_tool_calls),
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
    if merge_keys.plugins_policy {
        merge_project_plugin_policy(&mut base.policy, overlay.policy, merge_keys);
    }
    base.list.extend(overlay.list);
}

fn merge_project_plugin_policy(
    base: &mut agena_plugin_host::PluginPolicyConfig,
    overlay: agena_plugin_host::PluginPolicyConfig,
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
    base: &mut agena_plugin_host::ToolPresentationConfig,
    overlay: agena_plugin_host::ToolPresentationConfig,
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
    base: &mut agena_plugin_host::UiPresentationConfig,
    overlay: agena_plugin_host::UiPresentationConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub struct RawUiConfig {
    #[merge(strategy = option_override)]
    pub locale: Option<String>,
    #[merge(strategy = option_struct_merge)]
    pub tui: Option<RawTuiUiConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub struct RawTuiUiConfig {
    #[merge(strategy = option_override)]
    pub color_scheme: Option<TuiColorSchemeConfig>,
    #[merge(strategy = option_override)]
    pub graphics: Option<TuiGraphicsModeConfig>,
    #[merge(strategy = option_override)]
    pub theme: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub struct RawRuntimeConfig {
    #[merge(strategy = option_struct_merge)]
    pub providers: Option<RawRuntimeProvidersConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub struct RawRuntimeProvidersConfig {
    #[merge(strategy = option_struct_merge)]
    pub client_versions: Option<RawProviderClientVersionSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub struct RawProviderClientVersionSettings {
    #[merge(strategy = option_override)]
    pub codex: Option<String>,
    #[merge(strategy = option_override)]
    pub claude: Option<String>,
    #[merge(strategy = option_override)]
    pub gemini: Option<String>,
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
pub struct RawSessionConfig {
    #[merge(strategy = option_struct_merge)]
    pub compaction: Option<RawSessionCompactionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, DeriveMerge)]
#[serde(default, deny_unknown_fields)]
pub struct RawSessionCompactionConfig {
    #[merge(strategy = option_override)]
    pub auto: Option<bool>,
    #[merge(strategy = option_override)]
    pub reserved_tokens: Option<u32>,
}

impl SessionConfig {
    pub fn from_raw(raw: RawSessionConfig) -> Self {
        let compaction = raw.compaction.unwrap_or_default();
        Self {
            compaction: SessionCompactionConfig {
                auto: compaction.auto.unwrap_or(true),
                reserved_tokens: compaction.reserved_tokens,
            },
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
pub enum ProviderKind {
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "openai_responses")]
    OpenAiResponses,
    #[serde(rename = "openai_chat_completions")]
    OpenAiChatCompletions,
    #[serde(rename = "openai_realtime")]
    OpenAiRealtime,
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
            "openai_realtime" => Ok(Self::OpenAiRealtime),
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
    RawTuiUiConfig,
    RawRuntimeConfig,
    RawRuntimeProvidersConfig,
    RawProviderClientVersionSettings,
    RawSessionConfig,
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

fn validate_permission_config(
    label: &str,
    permission: &agena_domain::PermissionConfig,
) -> Result<(), ConfigError> {
    agena_runtime_contracts::authorization::validate_permission_config(permission)
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

#[cfg(test)]
mod openai_protocol_adapter_tests {
    use super::{
        RawConfig, reject_unsupported_fields_value, validate_config_text, validate_configured_modes,
    };
    use crate::ProcessEnvironment;
    use agena_provider::ConfiguredModelDefinition;
    use std::{fs, path::Path};

    fn config_with_adapter(adapter_id: &str, adapter_fields: &str) -> String {
        format!(
            r#"{{
                "providers": {{
                    "default": "test",
                    "test": {{
                        "defaults": {{ "adapter": "{adapter_id}", "model": "gpt-test" }},
                        "auth": {{
                            "mode": "api",
                            "subtype": "custom",
                            "base_url": "https://api.openai.com",
                            "api_key": {{ "kind": "inline", "value": "test-key" }}
                        }},
                        "adapters": {{
                            "{adapter_id}": {{
                                "enabled": true,
                                {adapter_fields}
                                "models": {{ "gpt-test": {{}} }}
                            }}
                        }}
                    }}
                }}
            }}"#,
        )
    }

    #[test]
    fn distinct_openai_protocol_adapters_are_accepted() {
        for adapter_id in [
            "openai_responses",
            "openai_chat_completions",
            "openai_realtime",
        ] {
            validate_config_text(
                Path::new("agena.json"),
                config_with_adapter(adapter_id, "").as_str(),
                &ProcessEnvironment,
            )
            .unwrap_or_else(|error| panic!("{adapter_id} should resolve: {error}"));
        }
    }

    #[test]
    fn legacy_openai_adapter_id_is_rejected() {
        let error = validate_config_text(
            Path::new("agena.json"),
            config_with_adapter("openai", "").as_str(),
            &ProcessEnvironment,
        )
        .expect_err("legacy adapter id must be rejected");
        assert!(error.to_string().contains("unknown provider kind `openai`"));
    }

    #[test]
    fn removed_agent_configuration_is_rejected() {
        for value in [
            serde_json::json!({ "agents": { "default": "build" } }),
            serde_json::json!({ "agent_profile": "build" }),
        ] {
            let error = serde_json::from_value::<RawConfig>(value)
                .expect_err("removed agent configuration must not be silently accepted");
            assert!(error.to_string().contains("unknown field"));
        }
    }

    #[test]
    fn legacy_api_mode_field_is_rejected() {
        let error = validate_config_text(
            Path::new("agena.json"),
            config_with_adapter("openai_responses", r#""api_mode": "auto","#).as_str(),
            &ProcessEnvironment,
        )
        .expect_err("legacy api_mode must be rejected");
        assert!(error.to_string().contains("unknown field `api_mode`"));
    }

        #[test]
    fn prompt_envelope_tool_mode_is_rejected() {
        let config = config_with_adapter("openai_chat_completions", "").replace(
            r#""gpt-test": {}"#,
            r#""gpt-test": {
                "agena_tools": { "mode": "prompt_envelope" }
            }"#,
        );
        validate_config_text(
            Path::new("agena.json"),
            config.as_str(),
            &ProcessEnvironment,
        )
        .expect_err("prompt-envelope mode was removed and must be rejected");
    }

    #[test]
    fn provider_native_tools_are_rejected_for_every_mode() {
        for mode in ["provider_protocol", "disabled"] {
            let config = config_with_adapter("openai_chat_completions", "").replace(
                r#""gpt-test": {}"#,
                format!(
                    r#""gpt-test": {{
                "agena_tools": {{
                    "mode": "{mode}",
                    "provider_native": {{
                        "routes": {{ "web_search": "provider_hosted" }}
                    }}
                }}
            }}"#
                )
                .as_str(),
            );
            let error = validate_config_text(
                Path::new("agena.json"),
                config.as_str(),
                &ProcessEnvironment,
            )
            .expect_err("provider-native model-route configuration must be rejected");
            assert!(
                error
                    .to_string()
                    .contains("unknown field `agena_tools.provider_native`")
            );
        }
    }

    #[test]
    fn direct_tool_policy_is_rejected_for_every_mode() {
        for mode in ["provider_protocol", "disabled"] {
            let config = config_with_adapter("openai_chat_completions", "").replace(
                r#""gpt-test": {}"#,
                format!(
                    r#""gpt-test": {{
                "agena_tools": {{
                    "mode": "{mode}",
                    "direct": {{ "max_tools": 3 }}
                }}
            }}"#
                )
                .as_str(),
            );
            let error = validate_config_text(
                Path::new("agena.json"),
                config.as_str(),
                &ProcessEnvironment,
            )
            .expect_err("direct model-route configuration must be rejected");
            assert!(
                error
                    .to_string()
                    .contains("unknown field `agena_tools.direct`")
            );
        }
    }

    #[test]
    fn legacy_transport_key_is_rejected() {
        let error = serde_json::from_value::<agena_provider::ResolvedProviderModelConfig>(
            serde_json::json!({ "agena_tools": { "transport": "prompt_envelope" } }),
        )
        .expect_err("legacy transport key must be rejected");
        assert!(error.to_string().contains("unknown field `transport`"));
    }

    #[test]
    fn legacy_top_level_native_tool_keys_are_rejected() {
        for key in ["provider_tools", "provider_native_tools", "native_tools"] {
            let mut object = serde_json::Map::new();
            object.insert(key.to_owned(), serde_json::json!({ "enabled": true }));
            let error = serde_json::from_value::<agena_provider::ResolvedProviderModelConfig>(
                serde_json::Value::Object(object),
            )
            .expect_err("legacy top-level native tool key must be rejected");
            assert!(error.to_string().contains("unknown field"), "key: {key}");
        }
    }

    #[test]
    fn removed_tool_declarations_are_not_serialized() {
        let model: agena_provider::ResolvedProviderModelConfig =
            serde_json::from_value(serde_json::json!({
                "agena_tools": {
                    "mode": "provider_protocol"
                }
            }))
            .expect("gateway-only model config should deserialize");

        let serialized = serde_json::to_value(model).expect("model config should serialize");
        assert!(serialized["agena_tools"].get("direct").is_none());
        assert!(serialized["agena_tools"].get("provider_native").is_none());
        assert!(serialized.get("provider_tools").is_none());
        assert!(serialized.get("provider_native_tools").is_none());
        assert!(serialized.get("native_tools").is_none());
    }

    #[test]
    fn model_native_compaction_defaults_enabled_and_serializes_only_when_disabled() {
        let defaulted: agena_provider::ResolvedProviderModelConfig =
            serde_json::from_value(serde_json::json!({}))
                .expect("empty model config should use execution defaults");
        assert!(defaulted.native_compaction);
        let serialized =
            serde_json::to_value(defaulted).expect("default model config should serialize");
        assert!(serialized.get("native_compaction").is_none());

        let disabled: agena_provider::ResolvedProviderModelConfig =
            serde_json::from_value(serde_json::json!({ "native_compaction": false }))
                .expect("native compaction should be configurable per model route");
        assert!(!disabled.native_compaction);
        let serialized =
            serde_json::to_value(disabled).expect("disabled model config should serialize");
        assert_eq!(serialized["native_compaction"], serde_json::json!(false));
    }

    #[test]
    fn legacy_provider_native_enabled_switch_is_rejected() {
        let error = serde_json::from_value::<agena_provider::ResolvedProviderModelConfig>(
            serde_json::json!({
                "agena_tools": {
                    "mode": "provider_protocol",
                    "provider_native": { "enabled": true }
                }
            }),
        )
        .expect_err("provider-native configuration was removed");
        assert!(
            error
                .to_string()
                .contains("unknown field `agena_tools.provider_native`")
        );
    }

    #[test]
    fn full_example_uses_only_resolvable_protocol_adapters() {
        let fixture = workspace_fixture("config.full.json");
        validate_config_text(Path::new("config.full.json"), &fixture, &ProcessEnvironment)
            .expect("config.full.json should remain a valid canonical configuration");
    }

    #[test]
    fn minimal_example_is_a_valid_canonical_configuration() {
        let fixture = workspace_fixture("config.example.json");
        validate_config_text(
            Path::new("config.example.json"),
            &fixture,
            &ProcessEnvironment,
        )
        .expect("config.example.json should remain a valid canonical configuration");
    }

    fn workspace_fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(name);
        fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read fixture {}: {error}", path.display()))
    }

    #[test]
    fn legacy_model_default_thinking_mode_field_is_rejected() {
        let error = reject_unsupported_fields_value(&serde_json::json!({
            "providers": {
                "test": {
                    "adapters": {
                        "openai_responses": {
                            "models": {
                                "gpt-test": {
                                    "default_thinking_mode": "medium"
                                }
                            }
                        }
                    }
                }
            }
        }))
        .expect_err("the detached string default was removed");

        assert!(error.to_string().contains("default_thinking_mode"));
    }

    #[test]
    fn provider_default_model_variants_are_removed_while_loading() {
        let mut value = serde_json::json!({
            "providers": {
                "test": {
                    "defaults": {
                        "adapter": "openai_responses",
                        "model": "gpt-test",
                        "thinking_mode": "high",
                        "speed_mode": "fast",
                        "verbosity": "low",
                        "parallel_tool_calls": true
                    },
                    "adapters": {}
                }
            }
        });
        super::strip_legacy_provider_default_variants(&mut value);
        let defaults = &value["providers"]["test"]["defaults"];
        assert_eq!(
            defaults,
            &serde_json::json!({
                "adapter": "openai_responses",
                "model": "gpt-test"
            })
        );
    }

    #[test]
    fn explicit_default_selection_resolves_model_variants() {
        let value = serde_json::json!({
            "providers": {
                "default": "test",
                "default_selection": {
                    "provider": "test",
                    "adapter": "openai_responses",
                    "model": "gpt-test",
                    "thinking_mode": "high",
                    "speed_mode": "fast",
                    "verbosity": "high",
                    "parallel_tool_calls": false
                },
                "test": {
                    "defaults": { "adapter": "openai_responses" },
                    "auth": {
                        "mode": "api",
                        "subtype": "custom",
                        "base_url": "https://api.openai.com",
                        "api_key": { "kind": "inline", "value": "test-key" }
                    },
                    "adapters": {
                        "openai_responses": {
                            "enabled": true,
                            "models": { "gpt-test": {} }
                        }
                    }
                }
            }
        });
        let raw = serde_json::from_value::<RawConfig>(value).expect("config should parse");
        let resolved = raw
            .resolve_with_env(&crate::ProcessEnvironment)
            .expect("config should resolve");

        assert_eq!(resolved.default_selection.provider.as_deref(), Some("test"));
        assert_eq!(
            resolved.default_selection.adapter.as_deref(),
            Some("openai_responses")
        );
        assert_eq!(
            resolved.default_selection.model.as_deref(),
            Some("gpt-test")
        );
        assert_eq!(
            resolved.default_selection.thinking_mode.as_deref(),
            Some("high")
        );
        assert_eq!(
            resolved.default_selection.speed_mode.as_deref(),
            Some("fast")
        );
        assert_eq!(
            resolved.default_selection.verbosity.as_deref(),
            Some("high")
        );
        assert_eq!(resolved.default_selection.parallel_tool_calls, Some(false));
    }

    #[test]
    fn thinking_mode_name_does_not_infer_its_strategy() {
        let definition: ConfiguredModelDefinition = serde_json::from_value(serde_json::json!({
            "thinking_modes": { "low": {} }
        }))
        .unwrap();

        let error = validate_configured_modes(
            "test",
            "model `gpt-test` thinking modes",
            &definition.thinking_modes,
            "model `gpt-test` speed modes",
            &definition.speed_modes,
        )
        .expect_err("mode names must not imply strategy or effort");

        assert!(error.to_string().contains("needs an explicit strategy"));
    }

    #[test]
    fn default_selection_adapter_referencing_a_disabled_adapter_is_rejected() {
        // `providers.default_selection` pins an adapter that is disabled in
        // the provider. Unlike the provider-level `defaults.adapter` check,
        // resolution must reject this too, otherwise the runtime resolves a
        // model through an adapter that is never built and fails per request
        // with a generic "no enabled adapter" configuration error.
        let value = serde_json::json!({
            "providers": {
                "default": "test",
                "default_selection": {
                    "provider": "test",
                    "adapter": "openai_chat_completions",
                    "model": "deepseek-v4-flash",
                    "thinking_mode": "max"
                },
                "test": {
                    "defaults": { "adapter": "openai_responses" },
                    "auth": {
                        "mode": "api",
                        "subtype": "custom",
                        "base_url": "https://api.openai.com",
                        "api_key": { "kind": "inline", "value": "test-key" }
                    },
                    "adapters": {
                        "openai_chat_completions": { "enabled": false },
                        "openai_responses": { "enabled": true }
                    }
                }
            }
        });
        let raw = serde_json::from_value::<RawConfig>(value).expect("config should parse");
        let error = raw.resolve_with_env(&crate::ProcessEnvironment).expect_err(
            "default_selection adapter referencing a disabled adapter must be rejected",
        );
        assert!(
            error.to_string().contains("references disabled adapter"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn default_selection_adapter_referencing_an_unknown_adapter_is_rejected() {
        let value = serde_json::json!({
            "providers": {
                "default": "test",
                "default_selection": {
                    "provider": "test",
                    "adapter": "does_not_exist",
                    "model": "deepseek-v4-flash"
                },
                "test": {
                    "defaults": { "adapter": "openai_responses" },
                    "auth": {
                        "mode": "api",
                        "subtype": "custom",
                        "base_url": "https://api.openai.com",
                        "api_key": { "kind": "inline", "value": "test-key" }
                    },
                    "adapters": {
                        "openai_responses": { "enabled": true }
                    }
                }
            }
        });
        let raw = serde_json::from_value::<RawConfig>(value).expect("config should parse");
        let error = raw.resolve_with_env(&crate::ProcessEnvironment).expect_err(
            "default_selection adapter referencing an unknown adapter must be rejected",
        );
        assert!(
            error.to_string().contains("references unknown adapter"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn deleted_provider_with_dangling_default_selection_is_rejected() {
        // A provider was removed from the file but `default_selection` still
        // references it: resolution must reject the dangling reference so the
        // settings layer refuses to persist the invalid document.
        let value = serde_json::json!({
            "providers": {
                "default": "opencode",
                "default_selection": {
                    "provider": "opencode",
                    "adapter": "openai_responses",
                    "model": "deepseek-v4-flash",
                    "thinking_mode": "max"
                },
                "chatgpt": {
                    "defaults": { "adapter": "openai_responses" },
                    "auth": {
                        "mode": "credential",
                        "issuer": "openai_chatgpt"
                    },
                    "adapters": {
                        "openai_responses": {
                            "enabled": true,
                            "backend": "chatgpt_codex",
                            "models": { "gpt-5": {} }
                        }
                    }
                }
            }
        });
        let raw = serde_json::from_value::<RawConfig>(value).expect("config should parse");
        let error = raw
            .resolve_with_env(&crate::ProcessEnvironment)
            .expect_err("dangling default_selection must be rejected");
        assert!(
            error.to_string().contains("references unknown provider"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn provider_default_adapter_may_not_point_at_a_disabled_adapter() {
        // Provider Studio unchecking an adapter keeps the adapter entry with
        // `enabled: false`; a stale `defaults.adapter` pointing at it must be
        // rejected during validation, which is exactly the failure the studio
        // hit when it tried to save such a selection.
        let value = serde_json::json!({
            "providers": {
                "test": {
                    "defaults": { "adapter": "openai_chat_completions" },
                    "auth": {
                        "mode": "api",
                        "subtype": "custom",
                        "base_url": "https://api.openai.com",
                        "api_key": { "kind": "inline", "value": "test-key" }
                    },
                    "adapters": {
                        "openai_chat_completions": { "enabled": false },
                        "anthropic": { "enabled": true }
                    }
                }
            }
        });
        let raw = serde_json::from_value::<RawConfig>(value).expect("config should parse");
        let error = raw
            .resolve_with_env(&crate::ProcessEnvironment)
            .expect_err("defaults.adapter referencing a disabled adapter must be rejected");
        assert!(
            error.to_string().contains("references disabled adapter"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn deleting_a_provider_and_clearing_its_references_validates() {
        // This is the document produced by `delete_provider` after its atomic
        // patch removes `providers.<id>`, `providers.default`, and
        // `providers.default_selection` (which referenced the deleted provider).
        // The remaining providers must still resolve.
        let value = serde_json::json!({
            "providers": {
                "chatgpt": {
                    "defaults": { "adapter": "openai_responses", "model": "gpt-5" },
                    "auth": {
                        "mode": "credential",
                        "issuer": "openai_chatgpt"
                    },
                    "adapters": {
                        "openai_responses": {
                            "enabled": true,
                            "backend": "chatgpt_codex",
                            "models": { "gpt-5": {} }
                        }
                    }
                }
            }
        });
        let raw = serde_json::from_value::<RawConfig>(value).expect("config should parse");
        let resolved = raw
            .resolve_with_env(&crate::ProcessEnvironment)
            .expect("config with provider references cleared must resolve");
        assert_eq!(
            resolved.default_selection.provider.as_deref(),
            Some("chatgpt"),
            "default selection falls back to the remaining enabled provider"
        );
    }
}
