//! `agena.settings` plugin: read and edit Agena's active `config.toml`.

use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use serde::Serialize;
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
            "settings_get" => self.get(serde_json::from_value(input.input)?).await,
            "settings_list" => self.list(serde_json::from_value(input.input)?).await,
            "settings_set" => self.set(serde_json::from_value(input.input)?).await,
            "settings_delete" => self.delete(serde_json::from_value(input.input)?).await,
            "settings_patch" => self.patch(serde_json::from_value(input.input)?).await,
            "settings_validate" => self.validate(serde_json::from_value(input.input)?).await,
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
            "settings_set" | "settings_delete" | "settings_patch" => {
                Ok(vec![PathRequest::write(path)])
            }
            "settings_get" | "settings_list" | "settings_validate" => {
                Ok(vec![PathRequest::read(path)])
            }
            _ => Ok(Vec::new()),
        }
    }
}

fn entries() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "settings_get",
            crate::entry::definition::json_schema_for::<ConfigSettingsGetInput>(),
        )
        .description(
            "Read one Agena setting by dot path. Use source=file for persisted config.toml or source=effective for the resolved runtime value.",
        )
        .tags([ToolTag::ReadOnly, ToolTag::Discovery, settings_tag()])
        .host_capability(crate::plugin::sdk::HostCapability::ReadConfig)
        .concurrency_safe(true)
        .always_load(),
        PluginToolDecl::new(
            "settings_list",
            crate::entry::definition::json_schema_for::<ConfigSettingsListInput>(),
        )
        .description(
            "List child settings under a dot path, optionally recursively. Use source=file for persisted values or source=effective for resolved runtime values.",
        )
        .tags([ToolTag::ReadOnly, ToolTag::Discovery, settings_tag()])
        .host_capability(crate::plugin::sdk::HostCapability::ReadConfig)
        .concurrency_safe(true)
        .always_load(),
        PluginToolDecl::new(
            "settings_set",
            crate::entry::definition::json_schema_for::<ConfigSettingsSetInput>(),
        )
        .description(
            "Create or replace one persisted Agena setting in config.toml, validate it, and reload the runtime by default.",
        )
        .tags([ToolTag::Mutating, ToolTag::FilesystemWrite, settings_tag()])
        .host_capabilities([
            crate::plugin::sdk::HostCapability::ReadConfig,
            crate::plugin::sdk::HostCapability::ReloadConfig,
        ])
        .concurrency_safe(false)
        .deferred_load(),
        PluginToolDecl::new(
            "settings_delete",
            crate::entry::definition::json_schema_for::<ConfigSettingsDeleteInput>(),
        )
        .description("Delete one persisted Agena setting from config.toml and reload by default.")
        .tags([ToolTag::Mutating, ToolTag::FilesystemWrite, settings_tag()])
        .host_capabilities([
            crate::plugin::sdk::HostCapability::ReadConfig,
            crate::plugin::sdk::HostCapability::ReloadConfig,
        ])
        .concurrency_safe(false)
        .deferred_load(),
        PluginToolDecl::new(
            "settings_patch",
            crate::entry::definition::json_schema_for::<ConfigSettingsPatchInput>(),
        )
        .description(
            "Deep-merge a JSON object into persisted config.toml; object entries merge, scalar entries replace, and null entries delete.",
        )
        .tags([ToolTag::Mutating, ToolTag::FilesystemWrite, settings_tag()])
        .host_capabilities([
            crate::plugin::sdk::HostCapability::ReadConfig,
            crate::plugin::sdk::HostCapability::ReloadConfig,
        ])
        .concurrency_safe(false)
        .deferred_load(),
        PluginToolDecl::new(
            "settings_validate",
            crate::entry::definition::json_schema_for::<ConfigSettingsValidateInput>(),
        )
        .description("Validate the active persisted Agena config.toml without changing it.")
        .tags([ToolTag::ReadOnly, settings_tag()])
        .host_capability(crate::plugin::sdk::HostCapability::ReadConfig)
        .concurrency_safe(true)
        .always_load(),
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
    fn manifest_exposes_complete_settings_crud_tools() {
        let plugin = SettingsPlugin::new();
        let manifest = plugin.manifest();
        let names = manifest
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "settings_get",
                "settings_list",
                "settings_set",
                "settings_delete",
                "settings_patch",
                "settings_validate",
            ]
        );
        let set = manifest
            .entries
            .iter()
            .find(|entry| entry.name == "settings_set")
            .expect("settings_set should exist");
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
            effective_host_path(Some("runtime.default_agent")).as_deref(),
            Some("config.runtime.default_agent")
        );
        assert_eq!(
            effective_host_path(Some("meta.config_path")).as_deref(),
            Some("meta.config_path")
        );
    }
}
