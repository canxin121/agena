//! `agena.settings` plugin: read and edit Agena's active `config.json`.

use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};

use agena_macros::StaticToolSurface;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::config::{
    ConfigError, ConfigSettingsDeleteInput, ConfigSettingsGetInput, ConfigSettingsListInput,
    ConfigSettingsPatchInput, ConfigSettingsPathInput, ConfigSettingsReadResponse,
    ConfigSettingsSetInput, ConfigSettingsSource, ConfigSettingsValidateResponse,
    delete_file_setting, list_file_settings, list_json_path, patch_file_settings,
    read_file_setting, set_file_setting, validate_file_settings,
};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostConfigReloadResponse};
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, PathRequest, Plugin, PluginManifest,
    PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput, ToolTag,
};

pub(crate) const SETTINGS_PLUGIN_ID: &str = "agena.settings";

pub(crate) struct SettingsPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SettingsScope {
    Config,
    Meta,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
struct SettingsGetToolInput {
    path: Option<String>,
    scope: Option<SettingsScope>,
    source: ConfigSettingsSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
struct SettingsListToolInput {
    path: Option<String>,
    scope: Option<SettingsScope>,
    source: ConfigSettingsSource,
    recursive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default)]
struct SettingsValidateToolInput {}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "settings",
    description = "Settings command. Set action to get, list, validate, set, delete, or patch. Edits validate config.json and reload by default.",
    summary = "Read, validate, or edit runtime settings.",
    help = "Use action `get` to inspect one setting path, `list` to enumerate settings, `validate` to validate config text without applying it, and `set`, `delete`, or `patch` to mutate config.json. For effective reads, prefer explicit `scope = config|meta` with a relative `path` instead of relying on prefixed paths like `config.foo`.",
    tags(
        ToolTag::ReadOnly,
        ToolTag::Mutating,
        ToolTag::FilesystemWrite,
        ToolTag::Discovery,
        settings_tag()
    ),
    host_capabilities(
        crate::plugin::sdk::HostCapability::ReadConfig,
        crate::plugin::sdk::HostCapability::ReloadConfig
    ),
    concurrency_safe = false
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SettingsToolInput {
    #[tool(exec = "get")]
    Get {
        #[serde(flatten)]
        args: SettingsGetToolInput,
    },
    #[tool(exec = "list")]
    List {
        #[serde(flatten)]
        args: SettingsListToolInput,
    },
    #[tool(exec = "validate")]
    Validate {
        #[serde(flatten)]
        args: SettingsValidateToolInput,
    },
    #[tool(exec = "set")]
    Set {
        #[serde(flatten)]
        args: ConfigSettingsSetInput,
    },
    #[tool(exec = "delete")]
    Delete {
        #[serde(flatten)]
        args: ConfigSettingsDeleteInput,
    },
    #[tool(exec = "patch")]
    Patch {
        #[serde(flatten)]
        args: ConfigSettingsPatchInput,
    },
}

impl SettingsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("settings plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("settings plugin invoked before init"))
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

    async fn get(&self, input: SettingsGetToolInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, config_found) = self.config_meta().await?;
        let response = match input.source {
            ConfigSettingsSource::File => {
                if input.scope == Some(SettingsScope::Meta) {
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
                        source: input.source,
                    },
                )
                .map_err(map_err)?
            }
            ConfigSettingsSource::Effective => ConfigSettingsReadResponse {
                value: self
                    .effective_config_value(
                        resolve_effective_settings_path(input.scope, input.path.as_deref())?
                            .as_deref(),
                    )
                    .await?,
                config_path,
                config_found,
                source: ConfigSettingsSource::Effective,
                path: input.path,
            },
        };
        output("Settings value", "Read settings value.", &response)
    }

    async fn list(&self, input: SettingsListToolInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, config_found) = self.config_meta().await?;
        let response = match input.source {
            ConfigSettingsSource::File => {
                if input.scope == Some(SettingsScope::Meta) {
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
                        source: input.source,
                        recursive: input.recursive,
                    },
                )
                .map_err(map_err)?
            }
            ConfigSettingsSource::Effective => {
                let value = self.host()?.read_config(None).await?;
                let entries = list_json_path(
                    &value,
                    resolve_effective_settings_path(input.scope, input.path.as_deref())?.as_deref(),
                    input.recursive,
                )
                .map_err(map_err)?;
                crate::config::ConfigSettingsListResponse {
                    config_path,
                    config_found,
                    source: ConfigSettingsSource::Effective,
                    path: input.path,
                    entries,
                }
            }
        };
        let count = response.entries.len();
        output(
            "Settings entries",
            format!(
                "Listed {count} settings entr{}.",
                if count == 1 { "y" } else { "ies" }
            ),
            &response,
        )
    }

    async fn set(&self, input: ConfigSettingsSetInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, _) = self.config_meta().await?;
        let reload = input.options.reload;
        let response = set_file_setting(config_path, input).map_err(map_err)?;
        self.edit_output(
            "Settings updated",
            "Updated settings value.",
            response,
            reload,
        )
        .await
    }

    async fn delete(&self, input: ConfigSettingsDeleteInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, _) = self.config_meta().await?;
        let reload = input.options.reload;
        let response = delete_file_setting(config_path, input).map_err(map_err)?;
        self.edit_output(
            "Settings deleted",
            "Deleted settings value.",
            response,
            reload,
        )
        .await
    }

    async fn patch(&self, input: ConfigSettingsPatchInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, _) = self.config_meta().await?;
        let reload = input.options.reload;
        let response = patch_file_settings(config_path, input).map_err(map_err)?;
        self.edit_output("Settings patched", "Patched settings.", response, reload)
            .await
    }

    async fn validate(&self, _input: SettingsValidateToolInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, _) = self.config_meta().await?;
        let response: ConfigSettingsValidateResponse =
            validate_file_settings(config_path).map_err(map_err)?;
        output("Settings valid", "Settings file is valid.", &response)
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
        Ok(ToolInvokeOutput::text(text)
            .with_title(title)
            .with_payload(payload)
            .with_metadata("agena.effect", "settings"))
    }
}

#[async_trait]
impl Plugin for SettingsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-settings", env!("CARGO_PKG_VERSION"))
            .description("Read and edit Agena runtime settings in config.json.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .tools(entries())
            .build()
    }

    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("settings plugin host lock poisoned"))? = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        match input.tool_name.as_str() {
            "settings" | "settings_edit" => match parse_settings_input(input.input)? {
                SettingsToolInput::Get { args } => self.get(args).await,
                SettingsToolInput::List { args } => self.list(args).await,
                SettingsToolInput::Validate { args } => self.validate(args).await,
                SettingsToolInput::Set { args } => self.set(args).await,
                SettingsToolInput::Delete { args } => self.delete(args).await,
                SettingsToolInput::Patch { args } => self.patch(args).await,
            },
            other => Err(PluginError::invalid_params(format!(
                "unknown settings tool '{other}'"
            ))),
        }
    }

    async fn permission_paths(
        &self,
        tool: &str,
        _input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        let (config_path, _) = self.config_meta().await?;
        let path = config_path.display().to_string();
        match tool {
            "settings_edit" => Ok(vec![PathRequest::write(path)]),
            "settings" => match parse_settings_input(_input.clone())? {
                SettingsToolInput::Get { .. }
                | SettingsToolInput::List { .. }
                | SettingsToolInput::Validate { .. } => Ok(vec![PathRequest::read(path)]),
                SettingsToolInput::Set { .. }
                | SettingsToolInput::Delete { .. }
                | SettingsToolInput::Patch { .. } => Ok(vec![PathRequest::write(path)]),
            },
            _ => Ok(Vec::new()),
        }
    }
}

fn entries() -> Vec<PluginToolDecl> {
    vec![SettingsToolInput::tool_decl()]
}

fn settings_tag() -> ToolTag {
    ToolTag::custom("settings").expect("settings tag is valid")
}

fn parse_settings_input(input: JsonValue) -> SdkResult<SettingsToolInput> {
    SettingsToolInput::parse_input(input)
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
    Ok(ToolInvokeOutput::text(text)
        .with_title(title)
        .with_payload(
            serde_json::to_value(payload).map_err(|err| PluginError::new(err.to_string()))?,
        )
        .with_metadata("agena.effect", "settings"))
}

fn map_err(error: ConfigError) -> PluginError {
    match error {
        ConfigError::Validation(_) | ConfigError::ParseFile { .. } => {
            PluginError::invalid_params(error.to_string())
        }
        other => PluginError::new(other.to_string()),
    }
}
