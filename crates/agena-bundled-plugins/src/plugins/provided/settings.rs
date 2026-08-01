//! `agena.settings` plugin: inspect and edit Agena's layered `agena.json` files.

use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, RwLock},
};

use agena_macros::ToolInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::config::{
    ConfigError, ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsGetInput,
    ConfigSettingsLayer, ConfigSettingsListInput, ConfigSettingsPatchInput,
    ConfigSettingsPathInput, ConfigSettingsReadResponse, ConfigSettingsSetInput,
    ConfigSettingsSource, ConfigSettingsValidateResponse, delete_layered_file_setting,
    list_file_settings, list_json_path, patch_layered_file_settings, read_file_setting,
    set_layered_file_setting, validate_layered_file_settings,
};
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{HostClient, HostConfigReloadResponse};
use agena_plugin_host::sdk::{PathRequest, Result as SdkResult, ToolInvokeOutput, ToolTag};

pub(crate) const SETTINGS_PLUGIN_ID: &str = "agena.settings";

pub(crate) struct SettingsPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
    config: OnceLock<SettingsPluginConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SettingsScope {
    Config,
    Meta,
}

/// Selects the persisted config file to read or mutate. `global` is the
/// user-level `~/agena/agena.json`; `workspace` is
/// `<workspace_root>/.agena/agena.json`.
type SettingsLayer = ConfigSettingsLayer;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct SettingsPluginConfig {
    reads: SettingsReadDefaultsConfig,
    edits: SettingsEditDefaultsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct SettingsReadDefaultsConfig {
    source: ConfigSettingsSource,
    file_layer: SettingsLayer,
    effective_scope: SettingsScope,
    list_recursive: bool,
}

impl Default for SettingsReadDefaultsConfig {
    fn default() -> Self {
        Self {
            source: ConfigSettingsSource::Effective,
            file_layer: SettingsLayer::Global,
            effective_scope: SettingsScope::Config,
            list_recursive: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct SettingsEditDefaultsConfig {
    file_layer: SettingsLayer,
    validate_by_default: bool,
    reload_after_write: bool,
}

impl Default for SettingsEditDefaultsConfig {
    fn default() -> Self {
        Self {
            file_layer: SettingsLayer::Global,
            validate_by_default: true,
            reload_after_write: true,
        }
    }
}

fn settings_config_schema() -> JsonValue {
    let mut schema = agena_runtime_tools::tool::definition::json_schema_for_default(
        SettingsPluginConfig::default(),
    );
    for (pointer, title, description) in [
        (
            "",
            "Settings Plugin Config",
            "Default read and edit behavior for the agena.settings plugin.",
        ),
        (
            "/properties/reads",
            "Reads",
            "Defaults applied when reading or listing settings through settings.",
        ),
        (
            "/properties/reads/properties/source",
            "Default Source",
            "Which settings source is used when get/list calls omit source.",
        ),
        (
            "/properties/reads/properties/file_layer",
            "Default Read Layer",
            "Which persisted config file is read when source=file and layer is omitted.",
        ),
        (
            "/properties/reads/properties/effective_scope",
            "Effective Scope",
            "Which branch of the effective settings snapshot is read when source=effective and scope is omitted.",
        ),
        (
            "/properties/reads/properties/list_recursive",
            "List Recursively",
            "Whether list calls recurse into nested structures when recursive is omitted.",
        ),
        (
            "/properties/edits",
            "Edits",
            "Defaults applied when mutating agena.json through set, delete, or patch.",
        ),
        (
            "/properties/edits/properties/file_layer",
            "Default Edit Layer",
            "Which persisted config file is edited when layer is omitted.",
        ),
        (
            "/properties/edits/properties/validate_by_default",
            "Validate by Default",
            "Runs config validation unless the caller explicitly disables it.",
        ),
        (
            "/properties/edits/properties/reload_after_write",
            "Reload After Write",
            "Reloads the active runtime configuration after a successful write unless the caller overrides it.",
        ),
    ] {
        agena_runtime_tools::tool::definition::set_schema_metadata(
            &mut schema,
            pointer,
            Some(title),
            Some(description),
        );
    }
    schema
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[input(trim("path"))]
#[serde(default, deny_unknown_fields)]
struct SettingsGetToolInput {
    path: Option<String>,
    scope: Option<SettingsScope>,
    source: Option<ConfigSettingsSource>,
    layer: Option<SettingsLayer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[input(trim("path"))]
#[serde(default, deny_unknown_fields)]
struct SettingsListToolInput {
    path: Option<String>,
    scope: Option<SettingsScope>,
    source: Option<ConfigSettingsSource>,
    layer: Option<SettingsLayer>,
    recursive: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[input(trim("path"))]
#[serde(default, deny_unknown_fields)]
struct SettingsInspectToolInput {
    path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[serde(default, deny_unknown_fields)]
struct SettingsValidateToolInput {
    layer: Option<SettingsLayer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("path"), non_empty("path"))]
#[serde(deny_unknown_fields)]
struct SettingsSetToolInput {
    path: String,
    value: JsonValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<SettingsLayer>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reload: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("path"), non_empty("path"))]
#[serde(deny_unknown_fields)]
struct SettingsDeleteToolInput {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<SettingsLayer>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reload: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("path"))]
#[serde(deny_unknown_fields)]
struct SettingsPatchToolInput {
    #[serde(default)]
    path: Option<String>,
    changes: JsonValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<SettingsLayer>,
    #[serde(default)]
    dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reload: Option<bool>,
}

#[derive(Debug, Clone)]
struct SettingsConfigMeta {
    global_path: PathBuf,
    global_found: bool,
    workspace_path: PathBuf,
    workspace_found: bool,
    applied_layers: JsonValue,
}

impl SettingsConfigMeta {
    fn from_value(meta: &JsonValue) -> SdkResult<Self> {
        Ok(Self {
            global_path: required_meta_path(meta, "config_path")?,
            global_found: meta
                .get("config_found")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            workspace_path: required_meta_path(meta, "project_config_path")?,
            workspace_found: meta
                .get("project_config_found")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            applied_layers: meta
                .get("applied_layers")
                .cloned()
                .unwrap_or_else(|| JsonValue::Array(Vec::new())),
        })
    }

    fn file(&self, layer: SettingsLayer) -> (&PathBuf, bool) {
        match layer {
            SettingsLayer::Global => (&self.global_path, self.global_found),
            SettingsLayer::Workspace => (&self.workspace_path, self.workspace_found),
        }
    }

    fn read_requests(
        &self,
        source: ConfigSettingsSource,
        layer: SettingsLayer,
    ) -> Vec<PathRequest> {
        match source {
            ConfigSettingsSource::File => {
                vec![PathRequest::read(self.file(layer).0.display().to_string())]
            }
            // Effective settings can contain values from both persisted files.
            // Declare both sources so the existing path policy remains the
            // security boundary for resolved config reads as well.
            ConfigSettingsSource::Effective => self.all_read_requests(),
        }
    }

    fn all_read_requests(&self) -> Vec<PathRequest> {
        let global = self.global_path.display().to_string();
        let workspace = self.workspace_path.display().to_string();
        if global == workspace {
            vec![PathRequest::read(global)]
        } else {
            vec![PathRequest::read(global), PathRequest::read(workspace)]
        }
    }

    fn edit_requests(&self, layer: SettingsLayer, dry_run: bool) -> Vec<PathRequest> {
        let target = self.file(layer).0.display().to_string();
        let other = self
            .file(match layer {
                SettingsLayer::Global => SettingsLayer::Workspace,
                SettingsLayer::Workspace => SettingsLayer::Global,
            })
            .0
            .display()
            .to_string();
        let mut requests = Vec::new();
        if dry_run {
            requests.push(PathRequest::read(target.clone()));
        } else {
            requests.push(PathRequest::write(target.clone()));
        }
        if other != target {
            requests.push(PathRequest::read(other));
        }
        requests
    }
}

#[derive(Debug, Serialize)]
struct SettingsInspectFileValue {
    layer: SettingsLayer,
    config_path: PathBuf,
    config_found: bool,
    path: Option<String>,
    defined: bool,
    value: JsonValue,
}

#[derive(Debug, Serialize)]
struct SettingsInspectResponse {
    path: Option<String>,
    global: SettingsInspectFileValue,
    workspace: SettingsInspectFileValue,
    effective: JsonValue,
    applied_layers: JsonValue,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "settings",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Inspect and edit Agena's global and workspace agena.json settings.",
    config_schema = settings_config_schema(),
    display = brief
)]
impl SettingsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
            config: OnceLock::new(),
        }
    }

    #[hook(init)]
    async fn init(
        &self,
        ctx: agena_plugin_host::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<agena_plugin_host::sdk::InitOutcome> {
        let config = agena_plugin_host::sdk::macro_support::parse_defaulted_config(
            ctx.config,
            "invalid settings plugin config",
        )?;
        self.config
            .set(config)
            .map_err(|_| PluginError::internal("settings plugin config already initialized"))?;
        *self
            .host
            .write()
            .map_err(|_| PluginError::internal("settings plugin host lock poisoned"))? = Some(host);
        Ok(agena_plugin_host::sdk::InitOutcome::ack(
            agena_plugin_host::sdk::Plugin::manifest(self),
        ))
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::internal("settings plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::internal("settings plugin invoked before init"))
    }

    fn config(&self) -> SdkResult<&SettingsPluginConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::internal("settings plugin invoked before init"))
    }

    fn read_source(
        &self,
        requested: Option<ConfigSettingsSource>,
    ) -> SdkResult<ConfigSettingsSource> {
        Ok(requested.unwrap_or(self.config()?.reads.source))
    }

    fn read_layer(&self, requested: Option<SettingsLayer>) -> SdkResult<SettingsLayer> {
        Ok(requested.unwrap_or(self.config()?.reads.file_layer))
    }

    fn edit_layer(&self, requested: Option<SettingsLayer>) -> SdkResult<SettingsLayer> {
        Ok(requested.unwrap_or(self.config()?.edits.file_layer))
    }

    fn effective_scope(
        &self,
        requested: Option<SettingsScope>,
        source: ConfigSettingsSource,
    ) -> SdkResult<Option<SettingsScope>> {
        Ok(match source {
            ConfigSettingsSource::Effective => {
                Some(requested.unwrap_or(self.config()?.reads.effective_scope))
            }
            ConfigSettingsSource::File => requested,
        })
    }

    fn edit_options(
        &self,
        dry_run: bool,
        validate: Option<bool>,
        reload: Option<bool>,
    ) -> SdkResult<ConfigSettingsEditOptions> {
        let config = self.config()?;
        Ok(ConfigSettingsEditOptions {
            dry_run,
            validate: validate.unwrap_or(config.edits.validate_by_default),
            reload: reload.unwrap_or(config.edits.reload_after_write),
        })
    }

    async fn config_meta(&self) -> SdkResult<SettingsConfigMeta> {
        let host = self.host()?;
        let meta = host.read_config(Some("meta".to_string())).await?;
        SettingsConfigMeta::from_value(&meta)
    }

    async fn effective_config_value(&self, path: Option<&str>) -> SdkResult<JsonValue> {
        let host = self.host()?;
        host.read_config(effective_host_path(path)).await
    }

    async fn read_permission_paths(
        &self,
        source: Option<ConfigSettingsSource>,
        layer: Option<SettingsLayer>,
    ) -> SdkResult<Vec<PathRequest>> {
        let source = self.read_source(source)?;
        let layer = self.read_layer(layer)?;
        Ok(self.config_meta().await?.read_requests(source, layer))
    }

    async fn inspect_permission_paths(&self) -> SdkResult<Vec<PathRequest>> {
        Ok(self.config_meta().await?.all_read_requests())
    }

    async fn edit_permission_paths(
        &self,
        layer: Option<SettingsLayer>,
        dry_run: bool,
    ) -> SdkResult<Vec<PathRequest>> {
        let layer = self.edit_layer(layer)?;
        Ok(self.config_meta().await?.edit_requests(layer, dry_run))
    }

    async fn validate_permission_paths(
        &self,
        _layer: Option<SettingsLayer>,
    ) -> SdkResult<Vec<PathRequest>> {
        Ok(self.config_meta().await?.all_read_requests())
    }

    #[tool(
        summary = "Read one settings path.",
        help = "Use `source=file` with `layer=global|workspace` for persisted values. Effective reads merge both files plus environment and CLI layers; prefer explicit `scope=config|meta` with a relative path.",
        display = brief,
        tags(
            ToolTag::ReadOnly,
            ToolTag::Discovery,
            settings_tag(),
            settings_read_tag(),
            ToolTag::FilesystemRead
        ),
        capabilities(agena_plugin_host::sdk::HostCapability::ReadConfig),
        path(requests = self.read_permission_paths(input.source, input.layer).await?),
        concurrency_safe
    )]
    async fn get(&self, input: SettingsGetToolInput) -> SdkResult<ToolInvokeOutput> {
        let meta = self.config_meta().await?;
        let source = self.read_source(input.source)?;
        let layer = self.read_layer(input.layer)?;
        let scope = self.effective_scope(input.scope, source)?;
        let response = match source {
            ConfigSettingsSource::File => {
                if scope == Some(SettingsScope::Meta) {
                    return Err(PluginError::invalid_params(
                        "settings get with source=file does not support scope=meta",
                    ));
                }
                let (config_path, _) = meta.file(layer);
                read_file_setting(
                    config_path.clone(),
                    ConfigSettingsGetInput {
                        target: ConfigSettingsPathInput {
                            path: input.path.clone(),
                        },
                        source,
                    },
                )
                .map_err(map_err)?
            }
            ConfigSettingsSource::Effective => {
                let (config_path, config_found) = meta.file(SettingsLayer::Global);
                ConfigSettingsReadResponse {
                    value: self
                        .effective_config_value(
                            resolve_effective_settings_path(scope, input.path.as_deref())?
                                .as_deref(),
                        )
                        .await?,
                    config_path: config_path.clone(),
                    config_found,
                    source,
                    path: input.path,
                }
            }
        };
        output_with_layer(
            "Settings value",
            "Read settings value.",
            &response,
            (source == ConfigSettingsSource::File).then_some(layer),
        )
    }

    #[tool(
        summary = "List settings paths.",
        display = brief,
        tags(
            ToolTag::ReadOnly,
            ToolTag::Discovery,
            settings_tag(),
            settings_read_tag(),
            ToolTag::FilesystemRead
        ),
        capabilities(agena_plugin_host::sdk::HostCapability::ReadConfig),
        path(requests = self.read_permission_paths(input.source, input.layer).await?),
        concurrency_safe
    )]
    async fn list(&self, input: SettingsListToolInput) -> SdkResult<ToolInvokeOutput> {
        let meta = self.config_meta().await?;
        let source = self.read_source(input.source)?;
        let layer = self.read_layer(input.layer)?;
        let scope = self.effective_scope(input.scope, source)?;
        let recursive = input
            .recursive
            .unwrap_or(self.config()?.reads.list_recursive);
        let response = match source {
            ConfigSettingsSource::File => {
                if scope == Some(SettingsScope::Meta) {
                    return Err(PluginError::invalid_params(
                        "settings list with source=file does not support scope=meta",
                    ));
                }
                let (config_path, _) = meta.file(layer);
                list_file_settings(
                    config_path.clone(),
                    ConfigSettingsListInput {
                        target: ConfigSettingsPathInput {
                            path: input.path.clone(),
                        },
                        source,
                        recursive,
                    },
                )
                .map_err(map_err)?
            }
            ConfigSettingsSource::Effective => {
                let value = self.host()?.read_config(None).await?;
                let items = list_json_path(
                    &value,
                    resolve_effective_settings_path(scope, input.path.as_deref())?.as_deref(),
                    recursive,
                )
                .map_err(map_err)?;
                let (config_path, config_found) = meta.file(SettingsLayer::Global);
                crate::config::ConfigSettingsListResponse {
                    config_path: config_path.clone(),
                    config_found,
                    source,
                    path: input.path,
                    items,
                }
            }
        };
        let count = response.items.len();
        output_with_layer(
            "Settings items",
            format!(
                "Listed {count} settings item{}.",
                if count == 1 { "" } else { "s" }
            ),
            &response,
            (source == ConfigSettingsSource::File).then_some(layer),
        )
    }

    #[tool(
        summary = "Inspect a setting across every config layer.",
        help = "Returns the persisted global value, persisted workspace value, effective merged value, source file paths, and applied-layer metadata. Secret values are always redacted.",
        display = brief,
        tags(
            ToolTag::ReadOnly,
            ToolTag::Discovery,
            settings_tag(),
            settings_read_tag(),
            ToolTag::FilesystemRead
        ),
        capabilities(agena_plugin_host::sdk::HostCapability::ReadConfig),
        path(requests = self.inspect_permission_paths().await?),
        concurrency_safe
    )]
    async fn inspect(&self, input: SettingsInspectToolInput) -> SdkResult<ToolInvokeOutput> {
        let meta = self.config_meta().await?;
        let global = inspect_file_value(&meta, SettingsLayer::Global, input.path.clone())?;
        let workspace = inspect_file_value(&meta, SettingsLayer::Workspace, input.path.clone())?;
        let effective_path =
            resolve_effective_settings_path(Some(SettingsScope::Config), input.path.as_deref())?;
        let effective = self
            .effective_config_value(effective_path.as_deref())
            .await?;
        let response = SettingsInspectResponse {
            path: input.path,
            global,
            workspace,
            effective,
            applied_layers: meta.applied_layers,
        };
        output(
            "Settings inspection",
            "Inspected global, workspace, and effective settings values.",
            &response,
        )
    }

    #[tool(
        summary = "Set one settings value.",
        help = "Writes the global or workspace config selected by `layer` and validates the combined layered configuration. Use `dry_run=true` to preview without writing; dry runs request read permission for both config files instead of write permission.",
        display = brief,
        tags(
            ToolTag::Mutating,
            ToolTag::FilesystemWrite,
            settings_tag(),
            settings_write_tag()
        ),
        capabilities(
            agena_plugin_host::sdk::HostCapability::ReadConfig,
            agena_plugin_host::sdk::HostCapability::ReloadConfig
        ),
        path(requests = self.edit_permission_paths(input.layer, input.dry_run).await?)
    )]
    async fn set(&self, input: SettingsSetToolInput) -> SdkResult<ToolInvokeOutput> {
        let layer = self.edit_layer(input.layer)?;
        let meta = self.config_meta().await?;
        let options = self.edit_options(input.dry_run, input.validate, input.reload)?;
        let reload = options.reload;
        let response = set_layered_file_setting(
            meta.global_path.clone(),
            meta.workspace_path.clone(),
            layer,
            ConfigSettingsSetInput {
                path: input.path,
                value: input.value,
                options,
            },
        )
        .map_err(map_err)?;
        self.edit_output(
            "Settings updated",
            "Updated settings value.",
            response,
            reload,
            layer,
        )
        .await
    }

    #[tool(
        summary = "Delete one settings value.",
        help = "Deletes from the global or workspace config selected by `layer` and validates the combined layered configuration. Use `dry_run=true` to preview without writing.",
        display = brief,
        tags(
            ToolTag::Mutating,
            ToolTag::FilesystemWrite,
            settings_tag(),
            settings_write_tag()
        ),
        capabilities(
            agena_plugin_host::sdk::HostCapability::ReadConfig,
            agena_plugin_host::sdk::HostCapability::ReloadConfig
        ),
        path(requests = self.edit_permission_paths(input.layer, input.dry_run).await?)
    )]
    async fn delete(&self, input: SettingsDeleteToolInput) -> SdkResult<ToolInvokeOutput> {
        let layer = self.edit_layer(input.layer)?;
        let meta = self.config_meta().await?;
        let options = self.edit_options(input.dry_run, input.validate, input.reload)?;
        let reload = options.reload;
        let response = delete_layered_file_setting(
            meta.global_path.clone(),
            meta.workspace_path.clone(),
            layer,
            ConfigSettingsDeleteInput {
                path: input.path,
                options,
            },
        )
        .map_err(map_err)?;
        self.edit_output(
            "Settings deleted",
            "Deleted settings value.",
            response,
            reload,
            layer,
        )
        .await
    }

    #[tool(
        summary = "Patch settings in agena.json.",
        help = "Deep-merges a JSON object into the global or workspace config selected by `layer`, then validates the combined layered configuration; null object entries delete keys. Use `dry_run=true` to preview without writing.",
        display = brief,
        tags(
            ToolTag::Mutating,
            ToolTag::FilesystemWrite,
            settings_tag(),
            settings_write_tag()
        ),
        capabilities(
            agena_plugin_host::sdk::HostCapability::ReadConfig,
            agena_plugin_host::sdk::HostCapability::ReloadConfig
        ),
        path(requests = self.edit_permission_paths(input.layer, input.dry_run).await?)
    )]
    async fn patch(&self, input: SettingsPatchToolInput) -> SdkResult<ToolInvokeOutput> {
        let layer = self.edit_layer(input.layer)?;
        let meta = self.config_meta().await?;
        let options = self.edit_options(input.dry_run, input.validate, input.reload)?;
        let reload = options.reload;
        let response = patch_layered_file_settings(
            meta.global_path.clone(),
            meta.workspace_path.clone(),
            layer,
            ConfigSettingsPatchInput {
                target: ConfigSettingsPathInput { path: input.path },
                changes: input.changes,
                options,
            },
        )
        .map_err(map_err)?;
        self.edit_output(
            "Settings patched",
            "Patched settings.",
            response,
            reload,
            layer,
        )
        .await
    }

    #[tool(
        summary = "Validate layered agena.json settings.",
        display = brief,
        tags(
            ToolTag::ReadOnly,
            settings_tag(),
            settings_read_tag(),
            ToolTag::FilesystemRead
        ),
        capabilities(agena_plugin_host::sdk::HostCapability::ReadConfig),
        path(requests = self.validate_permission_paths(input.layer).await?),
        concurrency_safe
    )]
    async fn validate(&self, input: SettingsValidateToolInput) -> SdkResult<ToolInvokeOutput> {
        let layer = self.read_layer(input.layer)?;
        let meta = self.config_meta().await?;
        let response: ConfigSettingsValidateResponse = validate_layered_file_settings(
            meta.global_path.clone(),
            meta.workspace_path.clone(),
            layer,
        )
        .map_err(map_err)?;
        output_with_layer(
            "Settings valid",
            "Settings file is valid.",
            &response,
            Some(layer),
        )
    }

    async fn edit_output<T>(
        &self,
        title: &str,
        text: &str,
        response: T,
        reload: bool,
        layer: SettingsLayer,
    ) -> SdkResult<ToolInvokeOutput>
    where
        T: Serialize,
    {
        let mut payload = serde_json::to_value(&response)
            .map_err(|err| PluginError::internal(err.to_string()))?;
        let reload_report = match payload
            .get("reload_required")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
            && reload
        {
            true => Some(self.host()?.reload_config().await?),
            false => None,
        };
        if let Some(report) = reload_report {
            insert_reload_report(&mut payload, report);
        }
        insert_settings_layer(&mut payload, layer);
        redact_settings_payload(&mut payload);
        Ok(ToolInvokeOutput::from_parts(
            title,
            text,
            Some(payload),
            std::collections::BTreeMap::from([("agena.effect".to_string(), "config".to_string())]),
            Vec::new(),
        ))
    }
}

fn settings_tag() -> ToolTag {
    ToolTag::custom("settings").expect("settings tag is valid")
}

fn settings_read_tag() -> ToolTag {
    ToolTag::custom("settings_read").expect("settings_read tag is valid")
}

fn settings_write_tag() -> ToolTag {
    ToolTag::custom("settings_write").expect("settings_write tag is valid")
}

fn required_meta_path(meta: &JsonValue, field: &str) -> SdkResult<PathBuf> {
    meta.get(field)
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| PluginError::internal(format!("host config meta is missing {field}")))
}

fn inspect_file_value(
    meta: &SettingsConfigMeta,
    layer: SettingsLayer,
    path: Option<String>,
) -> SdkResult<SettingsInspectFileValue> {
    let (config_path, config_found) = meta.file(layer);
    let response = read_file_setting(
        config_path.clone(),
        ConfigSettingsGetInput {
            target: ConfigSettingsPathInput { path: path.clone() },
            source: ConfigSettingsSource::File,
        },
    )
    .map_err(map_err)?;
    let defined = match path.as_deref() {
        Some(_) => !response.value.is_null(),
        None => config_found,
    };
    Ok(SettingsInspectFileValue {
        layer,
        config_path: config_path.clone(),
        config_found,
        path,
        defined,
        value: response.value,
    })
}

fn effective_host_path(path: Option<&str>) -> Option<String> {
    match path.map(str::trim).filter(|path| !path.is_empty()) {
        None => Some("config".to_string()),
        Some(path) if path == "config" || path == "meta" => Some(path.to_string()),
        Some(path) if path.starts_with("config.") || path.starts_with("meta.") => {
            Some(path.to_string())
        }
        Some(path) => Some(format!("config.{path}")),
    }
}

fn resolve_effective_settings_path(
    scope: Option<SettingsScope>,
    path: Option<&str>,
) -> SdkResult<Option<String>> {
    match scope {
        None => Ok(effective_host_path(path)),
        Some(scope) => {
            let trimmed = path.map(str::trim).filter(|value| !value.is_empty());
            if let Some(path) = trimmed
                && (path == "config"
                    || path == "meta"
                    || path.starts_with("config.")
                    || path.starts_with("meta."))
            {
                return Err(PluginError::invalid_params(
                    "explicit settings scope expects a relative path without `config.` or `meta.` prefix",
                ));
            }
            let root = match scope {
                SettingsScope::Config => "config",
                SettingsScope::Meta => "meta",
            };
            Ok(match trimmed {
                None => Some(root.to_string()),
                Some(path) => Some(format!("{root}.{path}")),
            })
        }
    }
}

fn insert_reload_report(payload: &mut JsonValue, report: HostConfigReloadResponse) {
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "reload".to_string(),
            serde_json::json!({
                "previous_generation": report.previous_generation,
                "generation": report.generation,
                "loaded_at": report.loaded_at,
            }),
        );
    }
}

fn insert_settings_layer(payload: &mut JsonValue, layer: SettingsLayer) {
    if let Some(object) = payload.as_object_mut()
        && let Ok(layer) = serde_json::to_value(layer)
    {
        object.insert("layer".to_string(), layer);
    }
}

fn output_with_layer<T>(
    title: &str,
    text: impl Into<String>,
    payload: &T,
    layer: Option<SettingsLayer>,
) -> SdkResult<ToolInvokeOutput>
where
    T: Serialize,
{
    let mut payload =
        serde_json::to_value(payload).map_err(|err| PluginError::internal(err.to_string()))?;
    if let Some(layer) = layer {
        insert_settings_layer(&mut payload, layer);
    }
    redact_settings_payload(&mut payload);
    Ok(ToolInvokeOutput::from_parts(
        title,
        text,
        Some(payload),
        std::collections::BTreeMap::from([("agena.effect".to_string(), "settings".to_string())]),
        Vec::new(),
    ))
}

fn output<T>(title: &str, text: impl Into<String>, payload: &T) -> SdkResult<ToolInvokeOutput>
where
    T: Serialize,
{
    output_with_layer(title, text, payload, None)
}

fn redact_settings_payload(value: &mut JsonValue) {
    redact_settings_value(value, None, false);
}

fn redact_settings_value(value: &mut JsonValue, key_hint: Option<&str>, secret_context: bool) {
    let secret_context = secret_context
        || is_inline_secret_source(value)
        || (key_hint.is_some_and(is_sensitive_container_key)
            && !is_environment_secret_source(value));
    match value {
        JsonValue::Object(object) => {
            let path_hint = object
                .get("path")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned);
            for (key, child) in object.iter_mut() {
                if path_hint.as_deref().is_some_and(|_| {
                    matches!(key.as_str(), "value" | "previous" | "current" | "effective")
                }) {
                    redact_settings_value_for_path(
                        child,
                        path_hint.as_deref().expect("checked above"),
                    );
                    continue;
                }
                let child_context = secret_context
                    || is_inline_secret_source(child)
                    || (is_sensitive_container_key(key) && !is_environment_secret_source(child));
                redact_settings_value(child, Some(key.as_str()), child_context);
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                redact_settings_value(item, key_hint, secret_context);
            }
        }
        JsonValue::Null => {}
        _ if is_sensitive_scalar_key(key_hint, secret_context) => {
            *value = JsonValue::String("<redacted>".to_string());
        }
        _ => {}
    }
}

fn redact_settings_value_for_path(value: &mut JsonValue, path: &str) {
    let Ok(segments) = crate::config::parse_settings_path(path) else {
        redact_settings_value(value, None, false);
        return;
    };
    let key_hint = segments.last().map(String::as_str);
    let parent_segments = &segments[..segments.len().saturating_sub(1)];
    let secret_context = parent_segments
        .iter()
        .any(|segment| is_sensitive_container_key(segment))
        || (key_hint.is_some_and(|key| normalize_sensitive_key(key) == "value")
            && parent_segments
                .iter()
                .any(|segment| normalize_sensitive_key(segment) == "source")
            && parent_segments.iter().any(|segment| {
                matches!(
                    normalize_sensitive_key(segment).as_str(),
                    "auth" | "access" | "api_key"
                )
            }));
    redact_settings_value(value, key_hint, secret_context);
}

fn is_sensitive_scalar_key(key: Option<&str>, secret_context: bool) -> bool {
    let Some(key) = key else {
        return false;
    };
    let normalized = normalize_sensitive_key(key);
    let compact = normalized.replace('_', "");
    is_sensitive_container_key(&normalized)
        || compact == "accesskeyid"
        || normalized == "authorization"
        || normalized == "proxy_authorization"
        || normalized == "token"
        || normalized.ends_with("_token")
        || normalized == "secret"
        || normalized.ends_with("_secret")
        || normalized.starts_with("secret_")
        || normalized == "password"
        || normalized.ends_with("_password")
        || normalized.contains("cookie")
        || normalized.contains("signature")
        || compact.contains("privatekey")
        || (secret_context
            && matches!(
                normalized.as_str(),
                "value" | "key" | "access" | "refresh" | "data"
            ))
}

fn is_sensitive_container_key(key: &str) -> bool {
    let normalized = normalize_sensitive_key(key);
    let compact = normalized.replace('_', "");
    matches!(compact.as_str(), "apikey" | "xapikey")
        || normalized == "credential"
        || normalized.ends_with("_credential")
}

fn normalize_sensitive_key(key: &str) -> String {
    key.trim().to_ascii_lowercase().replace(['-', '.'], "_")
}

fn is_environment_secret_source(value: &JsonValue) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(JsonValue::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("env"))
}

fn is_inline_secret_source(value: &JsonValue) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("kind"))
        .and_then(JsonValue::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("inline"))
        && value.get("value").is_some()
}

fn map_err(error: ConfigError) -> PluginError {
    match error {
        ConfigError::Validation(detail) => PluginError::invalid_params_with_public_detail(
            format!("config validation failed: {detail}"),
            format!("Invalid settings: {detail}"),
        ),
        ConfigError::ParseFile { path, source } => PluginError::invalid_params_with_public_detail(
            format!("failed to parse config file {}: {source}", path.display()),
            format!("Invalid settings JSON: {source}"),
        ),
        other => PluginError::internal(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::ToolPermissionConfig;
    use crate::permission::ToolPermissionPolicy;
    use agena_domain::PermissionDecision;
    use agena_domain::PermissionMode;
    use agena_plugin_host::sdk::{HostCapability, PathKind, Plugin};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn test_meta() -> SettingsConfigMeta {
        SettingsConfigMeta::from_value(&json!({
            "config_path": "/home/test/agena/agena.json",
            "config_found": true,
            "project_config_path": "/workspace/.agena/agena.json",
            "project_config_found": false,
            "applied_layers": [{"source": "default", "description": "built-in defaults"}]
        }))
        .expect("valid settings metadata")
    }

    #[test]
    fn settings_parse_errors_expose_schema_detail_without_private_config_path() {
        #[allow(dead_code)]
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Root {
            compaction: Option<bool>,
        }

        let source = serde_json::from_str::<Root>(r#"{"tool_trial":true}"#)
            .expect_err("unknown field must fail");
        let error = map_err(ConfigError::ParseFile {
            path: PathBuf::from("/Users/private/agena/agena.json"),
            source,
        });
        assert!(
            error
                .failure
                .user
                .fallback
                .contains("unknown field `tool_trial`")
        );
        assert!(error.failure.user.fallback.contains("compaction"));
        assert!(!error.failure.user.fallback.contains("/Users"));
        assert!(error.diagnostic_message().contains("/Users/private"));
    }

    #[test]
    fn manifest_exposes_layered_settings_tools_and_minimal_read_capabilities() {
        let manifest = SettingsPlugin::new().manifest();
        let names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "get", "list", "inspect", "set", "delete", "patch", "validate"
            ]
        );

        for name in ["get", "list", "inspect", "validate"] {
            let tool = manifest
                .tools
                .iter()
                .find(|tool| tool.name == name)
                .expect("settings read tool exists");
            assert_eq!(tool.capabilities, vec![HostCapability::ReadConfig]);
            assert!(tool.permissions.tags.contains(&settings_read_tag()));
            assert!(tool.permissions.tags.contains(&ToolTag::FilesystemRead));
            assert!(!tool.permissions.tags.contains(&settings_write_tag()));
        }

        let set = manifest
            .tools
            .iter()
            .find(|tool| tool.name == "set")
            .expect("settings set tool exists");
        assert!(set.capabilities.contains(&HostCapability::ReloadConfig));
        assert!(set.permissions.tags.contains(&settings_write_tag()));
    }

    #[test]
    fn layered_permission_requests_follow_source_and_dry_run() {
        let meta = test_meta();
        assert_eq!(
            meta.read_requests(ConfigSettingsSource::File, SettingsLayer::Workspace),
            vec![PathRequest::read("/workspace/.agena/agena.json")]
        );
        assert_eq!(
            meta.read_requests(ConfigSettingsSource::Effective, SettingsLayer::Global),
            vec![
                PathRequest::read("/home/test/agena/agena.json"),
                PathRequest::read("/workspace/.agena/agena.json")
            ]
        );
        assert_eq!(
            meta.edit_requests(SettingsLayer::Global, true),
            vec![
                PathRequest::read("/home/test/agena/agena.json"),
                PathRequest::read("/workspace/.agena/agena.json")
            ]
        );
        assert_eq!(
            meta.edit_requests(SettingsLayer::Global, false),
            vec![
                PathRequest::write("/home/test/agena/agena.json"),
                PathRequest::read("/workspace/.agena/agena.json")
            ]
        );
        assert_eq!(
            meta.edit_requests(SettingsLayer::Workspace, false)[0].kind,
            PathKind::Write
        );
    }

    #[test]
    fn settings_tags_and_exact_names_use_the_existing_tool_policy() {
        let manifest = SettingsPlugin::new().manifest();
        let read_tags = &manifest
            .tools
            .iter()
            .find(|tool| tool.name == "inspect")
            .expect("settings inspect tool exists")
            .permissions
            .tags;
        let write_tags = &manifest
            .tools
            .iter()
            .find(|tool| tool.name == "set")
            .expect("settings set tool exists")
            .permissions
            .tags;
        let config = ToolPermissionConfig {
            default: Some(PermissionMode::Ask),
            tags: BTreeMap::from([
                ("settings_read".to_string(), PermissionMode::Allow),
                ("settings_write".to_string(), PermissionMode::Deny),
            ]),
            names: BTreeMap::from([("agena.settings.set".to_string(), PermissionMode::Allow)]),
            ..Default::default()
        };
        let policy = crate::authorization::apply_tool_permission_config(
            &config,
            ToolPermissionPolicy::new(PermissionMode::Ask),
        )
        .expect("valid settings policy");

        assert!(matches!(
            policy.check_tool("agena.settings.inspect", None, read_tags),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            policy.check_tool("agena.settings.patch", None, write_tags),
            PermissionDecision::Deny { .. }
        ));
        assert!(matches!(
            policy.check_tool("agena.settings.set", None, write_tags),
            PermissionDecision::Allow
        ));
    }

    #[test]
    fn settings_payloads_redact_secrets_without_hiding_env_references() {
        let mut payload = json!({
            "path": "providers.openai.auth.api_key.value",
            "value": "previous-secret",
            "current": "current-secret",
            "config": {
                "tracing": { "filter": "info" },
                "providers": {
                    "openai": {
                        "auth": {
                            "api_key": {"kind": "inline", "value": "inline-secret"},
                            "credential": {
                                "type": "oauth",
                                "access": "access-secret",
                                "refresh": "refresh-secret",
                                "account_id": "acct-1"
                            }
                        }
                    },
                    "anthropic": {
                        "auth": {
                            "api_key": {"kind": "env", "value": "ANTHROPIC_API_KEY"}
                        }
                    },
                    "gitlab": {
                        "auth": {
                            "access": {
                                "kind": "api_key",
                                "source": {"kind": "inline", "value": "gitlab-secret"}
                            }
                        }
                    },
                    "bedrock": {
                        "auth": {
                            "accessKeyId": "AKIAEXAMPLE",
                            "secret_access_key": "bedrock-secret"
                        }
                    }
                }
            }
        });
        redact_settings_payload(&mut payload);

        assert_eq!(payload["value"], "<redacted>");
        assert_eq!(payload["current"], "<redacted>");
        assert_eq!(payload["config"]["tracing"]["filter"], "info");
        assert_eq!(
            payload["config"]["providers"]["openai"]["auth"]["api_key"]["value"],
            "<redacted>"
        );
        assert_eq!(
            payload["config"]["providers"]["openai"]["auth"]["credential"]["access"],
            "<redacted>"
        );
        assert_eq!(
            payload["config"]["providers"]["openai"]["auth"]["credential"]["account_id"],
            "acct-1"
        );
        assert_eq!(
            payload["config"]["providers"]["anthropic"]["auth"]["api_key"]["value"],
            "ANTHROPIC_API_KEY"
        );
        assert_eq!(
            payload["config"]["providers"]["gitlab"]["auth"]["access"]["source"]["value"],
            "<redacted>"
        );
        assert_eq!(
            payload["config"]["providers"]["bedrock"]["auth"]["accessKeyId"],
            "<redacted>"
        );

        let mut listed_secret = json!({
            "path": "providers.gitlab.auth.access.source.value",
            "kind": "string",
            "value": "gitlab-secret"
        });
        redact_settings_payload(&mut listed_secret);
        assert_eq!(listed_secret["value"], "<redacted>");
    }

    #[test]
    fn effective_paths_support_explicit_scopes_and_reject_mixed_forms() {
        assert_eq!(
            resolve_effective_settings_path(Some(SettingsScope::Config), Some("tracing.filter"))
                .expect("relative config path")
                .as_deref(),
            Some("config.tracing.filter")
        );
        assert_eq!(
            resolve_effective_settings_path(Some(SettingsScope::Meta), Some("config_path"))
                .expect("relative meta path")
                .as_deref(),
            Some("meta.config_path")
        );
        assert!(
            resolve_effective_settings_path(Some(SettingsScope::Config), Some("config.runtime"))
                .is_err()
        );
        assert!(
            resolve_effective_settings_path(Some(SettingsScope::Config), Some("meta.config_path"))
                .is_err()
        );
    }
}
