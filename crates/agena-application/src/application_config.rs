//! Config-setting operations, migrated from
//! `agena-tui-backend/src/backend_workspace.rs` (config subset) using the JSON
//! helpers from `backend_config.rs` (now in `provider_studio::save`).

use agena_runtime::{
    ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsEditResponse,
    ConfigSettingsSetInput,
};

use crate::provider_studio::catalog::quoted_settings_segment;
use crate::provider_studio::save::{
    normalize_plugin_record_for_config_edit, plugin_config_setting_target,
    plugin_record_for_config_edit, remove_nested_json_value, set_nested_json_value,
};
use crate::{Application, ApplicationError};

impl Application {
    pub async fn set_config_setting(
        &self,
        path: &str,
        value: serde_json::Value,
    ) -> Result<ConfigSettingsEditResponse, ApplicationError> {
        if let Some((plugin_id, config_segments)) = plugin_config_setting_target(path)
            .map_err(ApplicationError::internal)?
        {
            return self
                .set_plugin_config_setting(plugin_id.as_str(), config_segments.as_slice(), value)
                .await;
        }
        self.set_config_setting_direct(path, value).await
    }

    pub async fn delete_config_setting(
        &self,
        path: &str,
    ) -> Result<ConfigSettingsEditResponse, ApplicationError> {
        if let Some((plugin_id, config_segments)) = plugin_config_setting_target(path)
            .map_err(ApplicationError::internal)?
        {
            return self
                .delete_plugin_config_setting(plugin_id.as_str(), config_segments.as_slice())
                .await;
        }
        let response = self
            .runtime_config_settings()
            .delete_file_setting(ConfigSettingsDeleteInput {
                path: path.trim().to_owned(),
                options: ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            })
            .map_err(|error| {
                ApplicationError::internal(format!("failed to delete config setting: {error}"))
            })?;

        if response.reload_required {
            self.runtime_control()
                .reload()
                .await
                .map_err(|error| {
                    ApplicationError::internal(format!(
                        "failed to reload runtime after config change: {error}"
                    ))
                })?;
        }
        Ok(response)
    }

    pub async fn set_plugin_config_setting(
        &self,
        plugin_id: &str,
        config_segments: &[String],
        value: serde_json::Value,
    ) -> Result<ConfigSettingsEditResponse, ApplicationError> {
        let sources = self.config_json_sources()?;
        let mut record = plugin_record_for_config_edit(&sources, plugin_id);
        let config = normalize_plugin_record_for_config_edit(&mut record)
            .map_err(ApplicationError::internal)?;
        set_nested_json_value(config, config_segments, value);
        let path = format!("plugins.list.{}", quoted_settings_segment(plugin_id));
        self.set_config_setting_direct(path.as_str(), record).await
    }

    pub async fn delete_plugin_config_setting(
        &self,
        plugin_id: &str,
        config_segments: &[String],
    ) -> Result<ConfigSettingsEditResponse, ApplicationError> {
        let sources = self.config_json_sources()?;
        let mut record = plugin_record_for_config_edit(&sources, plugin_id);
        let config = normalize_plugin_record_for_config_edit(&mut record)
            .map_err(ApplicationError::internal)?;
        remove_nested_json_value(config, config_segments);
        let path = format!("plugins.list.{}", quoted_settings_segment(plugin_id));
        self.set_config_setting_direct(path.as_str(), record).await
    }

    async fn set_config_setting_direct(
        &self,
        path: &str,
        value: serde_json::Value,
    ) -> Result<ConfigSettingsEditResponse, ApplicationError> {
        let response = self
            .runtime_config_settings()
            .set_file_setting(ConfigSettingsSetInput {
                path: path.trim().to_owned(),
                value,
                options: ConfigSettingsEditOptions {
                    dry_run: false,
                    validate: true,
                    reload: true,
                },
            })
            .map_err(|error| {
                ApplicationError::internal(format!("failed to set config setting: {error}"))
            })?;

        if response.reload_required {
            self.runtime_control()
                .reload()
                .await
                .map_err(|error| {
                    ApplicationError::internal(format!(
                        "failed to reload runtime after config change: {error}"
                    ))
                })?;
        }
        Ok(response)
    }
}
