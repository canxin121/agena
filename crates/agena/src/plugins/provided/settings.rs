//! `agena.settings` plugin: read and edit Agena's active `config.toml`.

use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::config::{
    ConfigError, ConfigSettingsDeleteInput, ConfigSettingsGetInput, ConfigSettingsListInput,
    ConfigSettingsPatchInput, ConfigSettingsReadResponse, ConfigSettingsSetInput,
    ConfigSettingsSource, ConfigSettingsValidateInput, ConfigSettingsValidateResponse,
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum SettingsToolInput {
    Get(ConfigSettingsGetInput),
    List(ConfigSettingsListInput),
    Validate(ConfigSettingsValidateInput),
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum SettingsEditToolInput {
    Set(ConfigSettingsSetInput),
    Delete(ConfigSettingsDeleteInput),
    Patch(ConfigSettingsPatchInput),
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

    async fn get(&self, input: ConfigSettingsGetInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, config_found) = self.config_meta().await?;
        let response = match input.source {
            ConfigSettingsSource::File => read_file_setting(config_path, input).map_err(map_err)?,
            ConfigSettingsSource::Effective => ConfigSettingsReadResponse {
                value: self.effective_config_value(input.path.as_deref()).await?,
                config_path,
                config_found,
                source: ConfigSettingsSource::Effective,
                path: input.path,
            },
        };
        Ok(output("Settings value", "Read settings value.", &response)?)
    }

    async fn list(&self, input: ConfigSettingsListInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, config_found) = self.config_meta().await?;
        let response = match input.source {
            ConfigSettingsSource::File => {
                list_file_settings(config_path, input).map_err(map_err)?
            }
            ConfigSettingsSource::Effective => {
                let value = self.host()?.read_config(None).await?;
                let entries = list_json_path(
                    &value,
                    effective_host_path(input.path.as_deref()).as_deref(),
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
        Ok(output(
            "Settings entries",
            format!(
                "Listed {count} settings entr{}.",
                if count == 1 { "y" } else { "ies" }
            ),
            &response,
        )?)
    }

    async fn set(&self, input: ConfigSettingsSetInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, _) = self.config_meta().await?;
        let reload = input.reload;
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
        let reload = input.reload;
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
        let reload = input.reload;
        let response = patch_file_settings(config_path, input).map_err(map_err)?;
        self.edit_output("Settings patched", "Patched settings.", response, reload)
            .await
    }

    async fn validate(&self, _input: ConfigSettingsValidateInput) -> SdkResult<ToolInvokeOutput> {
        let (config_path, _) = self.config_meta().await?;
        let response: ConfigSettingsValidateResponse =
            validate_file_settings(config_path).map_err(map_err)?;
        Ok(output(
            "Settings valid",
            "Settings file is valid.",
            &response,
        )?)
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
            .description("Read and edit Agena runtime settings in config.toml.")
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
            "settings" => match serde_json::from_value::<SettingsToolInput>(input.input)? {
                SettingsToolInput::Get(args) => self.get(args).await,
                SettingsToolInput::List(args) => self.list(args).await,
                SettingsToolInput::Validate(args) => self.validate(args).await,
            },
            "settings_edit" => {
                match serde_json::from_value::<SettingsEditToolInput>(input.input)? {
                    SettingsEditToolInput::Set(args) => self.set(args).await,
                    SettingsEditToolInput::Delete(args) => self.delete(args).await,
                    SettingsEditToolInput::Patch(args) => self.patch(args).await,
                }
            }
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
            "settings" => Ok(vec![PathRequest::read(path)]),
            _ => Ok(Vec::new()),
        }
    }
}

fn entries() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "settings",
            crate::entry::definition::json_schema_for::<SettingsToolInput>(),
        )
        .description(
            "Settings read command. Set command to get, list, or validate; pass that command's payload in args.",
        )
        .tags([ToolTag::ReadOnly, ToolTag::Discovery, settings_tag()])
        .host_capability(crate::plugin::sdk::HostCapability::ReadConfig)
        .concurrency_safe(true)
        .always_load(),
        PluginToolDecl::new(
            "settings_edit",
            crate::entry::definition::json_schema_for::<SettingsEditToolInput>(),
        )
        .description(
            "Settings edit command. Set command to set, delete, or patch; pass that command's payload in args. Edits validate config.toml and reload by default.",
        )
        .tags([ToolTag::Mutating, ToolTag::FilesystemWrite, settings_tag()])
        .host_capabilities([
            crate::plugin::sdk::HostCapability::ReadConfig,
            crate::plugin::sdk::HostCapability::ReloadConfig,
        ])
        .concurrency_safe(false)
        .deferred_load(),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_exposes_settings_command_tools() {
        let plugin = SettingsPlugin::new();
        let manifest = plugin.manifest();
        let names = manifest
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["settings", "settings_edit"]);
        let set = manifest
            .entries
            .iter()
            .find(|entry| entry.name == "settings_edit")
            .expect("settings_edit should exist");
        assert!(set.has_tag(ToolTag::Mutating));
        assert!(
            set.host_capabilities
                .contains(&crate::plugin::sdk::HostCapability::ReloadConfig)
        );
    }

    #[test]
    fn effective_host_path_defaults_to_resolved_config() {
        assert_eq!(effective_host_path(None).as_deref(), Some("config"));
        assert_eq!(
            effective_host_path(Some("default.agent")).as_deref(),
            Some("config.default.agent")
        );
        assert_eq!(
            effective_host_path(Some("meta.config_path")).as_deref(),
            Some("meta.config_path")
        );
    }
}
