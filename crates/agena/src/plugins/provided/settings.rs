//! `agena.settings` plugin: read and edit Agena's active `agena.json`.

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
    ConfigSettingsListInput, ConfigSettingsPatchInput, ConfigSettingsPathInput,
    ConfigSettingsReadResponse, ConfigSettingsSetInput, ConfigSettingsSource,
    ConfigSettingsValidateResponse, delete_file_setting, list_file_settings, list_json_path,
    patch_file_settings, read_file_setting, set_file_setting, validate_file_settings,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostConfigReloadResponse};
use crate::plugin::sdk::{PathRequest, Result as SdkResult, ToolInvokeOutput, ToolTag};

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
    effective_scope: SettingsScope,
    list_recursive: bool,
}

impl Default for SettingsReadDefaultsConfig {
    fn default() -> Self {
        Self {
            source: ConfigSettingsSource::Effective,
            effective_scope: SettingsScope::Config,
            list_recursive: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct SettingsEditDefaultsConfig {
    validate_by_default: bool,
    reload_after_write: bool,
}

impl Default for SettingsEditDefaultsConfig {
    fn default() -> Self {
        Self {
            validate_by_default: true,
            reload_after_write: true,
        }
    }
}

fn settings_config_schema() -> JsonValue {
    let mut schema =
        crate::tool::definition::json_schema_for_default(SettingsPluginConfig::default());
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
        crate::tool::definition::set_schema_metadata(
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
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[input(trim("path"))]
#[serde(default, deny_unknown_fields)]
struct SettingsListToolInput {
    path: Option<String>,
    scope: Option<SettingsScope>,
    source: Option<ConfigSettingsSource>,
    recursive: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[serde(default, deny_unknown_fields)]
struct SettingsValidateToolInput {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("path"), non_empty("path"))]
#[serde(deny_unknown_fields)]
struct SettingsSetToolInput {
    path: String,
    value: JsonValue,
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
    #[serde(default)]
    dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reload: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput, Default)]
#[input(trim("path"))]
#[serde(default, deny_unknown_fields)]
struct SettingsPatchToolInput {
    path: Option<String>,
    changes: JsonValue,
    #[serde(default)]
    dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validate: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reload: Option<bool>,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "settings",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Read and edit Agena runtime settings in agena.json.",
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

    #[hook]
    async fn init(
        &self,
        ctx: crate::plugin::sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<crate::plugin::sdk::InitOutcome> {
        let config = crate::plugin::sdk::macro_support::parse_defaulted_config(
            ctx.config,
            "invalid settings plugin config",
        )?;
        self.config
            .set(config)
            .map_err(|_| PluginError::new("settings plugin config already initialized"))?;
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("settings plugin host lock poisoned"))? = Some(host);
        Ok(crate::plugin::sdk::InitOutcome::ack(
            crate::plugin::sdk::Plugin::manifest(self),
        ))
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("settings plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("settings plugin invoked before init"))
    }

    fn config(&self) -> SdkResult<&SettingsPluginConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::new("settings plugin invoked before init"))
    }

    fn read_source(
        &self,
        requested: Option<ConfigSettingsSource>,
    ) -> SdkResult<ConfigSettingsSource> {
        Ok(requested.unwrap_or(self.config()?.reads.source))
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

    async fn config_meta(&self) -> SdkResult<(PathBuf, bool)> {
        let host = self.host()?;
        let meta = host.read_config(Some("meta".to_string())).await?;
        let path = meta
            .get("config_path")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| PluginError::new("host config meta is missing config_path"))?;
        let found = meta
            .get("config_found")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        Ok((PathBuf::from(path), found))
    }

    async fn effective_config_value(&self, path: Option<&str>) -> SdkResult<JsonValue> {
        let host = self.host()?;
        host.read_config(effective_host_path(path)).await
    }

    #[tool(
        summary = "Read one settings path.",
        help = "For effective reads, prefer explicit `scope = config|meta` with a relative `path` instead of relying on prefixed paths like `config.foo`.",
        display = brief,
        tags(ToolTag::ReadOnly, ToolTag::Discovery, settings_tag()),
        capabilities(
            crate::plugin::sdk::HostCapability::ReadConfig,
            crate::plugin::sdk::HostCapability::ReloadConfig
        ),
        permission(paths = permission_get),
        concurrency_safe
    )]
    async fn get(&self, input: SettingsGetToolInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, config_found) = self.config_meta().await?;
        let source = self.read_source(input.source)?;
        let scope = self.effective_scope(input.scope, source)?;
        let response = match source {
            ConfigSettingsSource::File => {
                if scope == Some(SettingsScope::Meta) {
                    return Err(PluginError::invalid_params(
                        "settings get with source=file does not support scope=meta",
                    ));
                }
                read_file_setting(
                    config_path,
                    ConfigSettingsGetInput {
                        target: ConfigSettingsPathInput {
                            path: input.path.clone(),
                        },
                        source,
                    },
                )
                .map_err(map_err)?
            }
            ConfigSettingsSource::Effective => ConfigSettingsReadResponse {
                value: self
                    .effective_config_value(
                        resolve_effective_settings_path(scope, input.path.as_deref())?.as_deref(),
                    )
                    .await?,
                config_path,
                config_found,
                source,
                path: input.path,
            },
        };
        output("Settings value", "Read settings value.", &response)
    }

    #[tool(
        summary = "List settings paths.",
        display = brief,
        tags(ToolTag::ReadOnly, ToolTag::Discovery, settings_tag()),
        capabilities(
            crate::plugin::sdk::HostCapability::ReadConfig,
            crate::plugin::sdk::HostCapability::ReloadConfig
        ),
        permission(paths = permission_list),
        concurrency_safe
    )]
    async fn list(&self, input: SettingsListToolInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, config_found) = self.config_meta().await?;
        let source = self.read_source(input.source)?;
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
                list_file_settings(
                    config_path,
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
                crate::config::ConfigSettingsListResponse {
                    config_path,
                    config_found,
                    source,
                    path: input.path,
                    items,
                }
            }
        };
        let count = response.items.len();
        output(
            "Settings items",
            format!(
                "Listed {count} settings item{}.",
                if count == 1 { "" } else { "s" }
            ),
            &response,
        )
    }

    #[tool(
        summary = "Set one settings value.",
        display = brief,
        tags(ToolTag::Mutating, ToolTag::FilesystemWrite, settings_tag()),
        capabilities(
            crate::plugin::sdk::HostCapability::ReadConfig,
            crate::plugin::sdk::HostCapability::ReloadConfig
        ),
        permission(paths = permission_set)
    )]
    async fn set(&self, input: SettingsSetToolInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, _) = self.config_meta().await?;
        let options = self.edit_options(input.dry_run, input.validate, input.reload)?;
        let reload = options.reload;
        let response = set_file_setting(
            config_path,
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
        )
        .await
    }

    #[tool(
        summary = "Delete one settings value.",
        display = brief,
        tags(ToolTag::Mutating, ToolTag::FilesystemWrite, settings_tag()),
        capabilities(
            crate::plugin::sdk::HostCapability::ReadConfig,
            crate::plugin::sdk::HostCapability::ReloadConfig
        ),
        permission(paths = permission_delete)
    )]
    async fn delete(&self, input: SettingsDeleteToolInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, _) = self.config_meta().await?;
        let options = self.edit_options(input.dry_run, input.validate, input.reload)?;
        let reload = options.reload;
        let response = delete_file_setting(
            config_path,
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
        )
        .await
    }

    #[tool(
        summary = "Patch settings in agena.json.",
        display = brief,
        tags(ToolTag::Mutating, ToolTag::FilesystemWrite, settings_tag()),
        capabilities(
            crate::plugin::sdk::HostCapability::ReadConfig,
            crate::plugin::sdk::HostCapability::ReloadConfig
        ),
        permission(paths = permission_patch)
    )]
    async fn patch(&self, input: SettingsPatchToolInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, _) = self.config_meta().await?;
        let options = self.edit_options(input.dry_run, input.validate, input.reload)?;
        let reload = options.reload;
        let response = patch_file_settings(
            config_path,
            ConfigSettingsPatchInput {
                target: ConfigSettingsPathInput { path: input.path },
                changes: input.changes,
                options,
            },
        )
        .map_err(map_err)?;
        self.edit_output("Settings patched", "Patched settings.", response, reload)
            .await
    }

    #[tool(
        summary = "Validate agena.json.",
        display = brief,
        tags(ToolTag::ReadOnly, settings_tag()),
        capabilities(
            crate::plugin::sdk::HostCapability::ReadConfig,
            crate::plugin::sdk::HostCapability::ReloadConfig
        ),
        permission(paths = permission_validate),
        concurrency_safe
    )]
    async fn validate(&self, _input: SettingsValidateToolInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, _) = self.config_meta().await?;
        let response: ConfigSettingsValidateResponse =
            validate_file_settings(config_path).map_err(map_err)?;
        output("Settings valid", "Settings file is valid.", &response)
    }

    async fn config_read_permission(&self) -> SdkResult<Vec<PathRequest>> {
        let (config_path, _) = self.config_meta().await?;
        Ok(vec![PathRequest::read(config_path.display().to_string())])
    }

    async fn config_write_permission(&self) -> SdkResult<Vec<PathRequest>> {
        let (config_path, _) = self.config_meta().await?;
        Ok(vec![PathRequest::write(config_path.display().to_string())])
    }

    async fn permission_get(&self, _input: SettingsGetToolInput) -> SdkResult<Vec<PathRequest>> {
        self.config_read_permission().await
    }

    async fn permission_list(&self, _input: SettingsListToolInput) -> SdkResult<Vec<PathRequest>> {
        self.config_read_permission().await
    }

    async fn permission_validate(
        &self,
        _input: SettingsValidateToolInput,
    ) -> SdkResult<Vec<PathRequest>> {
        self.config_read_permission().await
    }

    async fn permission_set(&self, _input: SettingsSetToolInput) -> SdkResult<Vec<PathRequest>> {
        self.config_write_permission().await
    }

    async fn permission_delete(
        &self,
        _input: SettingsDeleteToolInput,
    ) -> SdkResult<Vec<PathRequest>> {
        self.config_write_permission().await
    }

    async fn permission_patch(
        &self,
        _input: SettingsPatchToolInput,
    ) -> SdkResult<Vec<PathRequest>> {
        self.config_write_permission().await
    }

    async fn edit_output<T>(
        &self,
        title: &str,
        text: &str,
        response: T,
        reload: bool,
    ) -> SdkResult<ToolInvokeOutput>
    where
        T: Serialize,
    {
        let mut payload =
            serde_json::to_value(&response).map_err(|err| PluginError::new(err.to_string()))?;
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

fn output<T>(title: &str, text: impl Into<String>, payload: &T) -> SdkResult<ToolInvokeOutput>
where
    T: Serialize,
{
    Ok(ToolInvokeOutput::from_parts(
        title,
        text,
        Some(serde_json::to_value(payload).map_err(|err| PluginError::new(err.to_string()))?),
        std::collections::BTreeMap::from([("agena.effect".to_string(), "settings".to_string())]),
        Vec::new(),
    ))
}

fn map_err(error: ConfigError) -> PluginError {
    match error {
        ConfigError::Validation(_) | ConfigError::ParseFile { .. } => {
            PluginError::invalid_params(error.to_string())
        }
        other => PluginError::new(other.to_string()),
    }
}
