//! Settings reads and mutations shared by all application transports.
use super::wire;
use crate::{Application, ApplicationError};
use agena_api::management::{SettingsCommand, SettingsQuery, settings::*};
use agena_api::resource::{ConfigSettingsEditResponse, ConfigSettingsReloadResponse};
use serde_json::Value;

#[derive(serde::Serialize)]
struct SettingSources {
    effective: ConfigSettingsReadResponse,
    file: ConfigSettingsReadResponse,
    global: ConfigSettingsReadResponse,
    workspace: ConfigSettingsReadResponse,
}

impl Application {
    pub async fn query_settings(&self, query: SettingsQuery) -> Result<Value, ApplicationError> {
        self.run_blocking(move |app| match query {
            SettingsQuery::Get(input) => {
                let path = input.target.path.clone();
                let response = match input.source {
                    ConfigSettingsSource::File => app
                        .runtime_config_settings()
                        .read_file_settings(input)
                        .map_err(settings_error)?,
                    ConfigSettingsSource::Effective => {
                        let configuration = app.config_json_sources()?;
                        let value =
                            agena_domain::get_json_path(&configuration.effective, path.as_deref())
                                .map_err(|error| {
                                    ApplicationError::bad_request_with_diagnostic(
                                        "The settings path is invalid.",
                                        error,
                                    )
                                })?;
                        ConfigSettingsReadResponse {
                            revision: None,
                            config_path: configuration.config_path,
                            config_found: configuration.config_found,
                            source: ConfigSettingsSource::Effective,
                            path,
                            value,
                        }
                    }
                };
                wire(response)
            }
            SettingsQuery::GetLayer { layer, mut input } => {
                input.source = ConfigSettingsSource::File;
                wire(
                    match layer {
                        ConfigSettingsLayer::Global => {
                            app.runtime_config_settings().read_file_settings(input)
                        }
                        ConfigSettingsLayer::Workspace => app
                            .runtime_config_settings()
                            .read_project_file_settings(input),
                    }
                    .map_err(settings_error)?,
                )
            }
            SettingsQuery::List(input) => {
                let path = input.target.path.clone();
                let response = match input.source {
                    ConfigSettingsSource::File => app
                        .runtime_config_settings()
                        .list_file_settings(input)
                        .map_err(settings_error)?,
                    ConfigSettingsSource::Effective => {
                        let configuration = app.config_json_sources()?;
                        let items = agena_runtime::list_json_path(
                            &configuration.effective,
                            path.as_deref(),
                            input.recursive,
                        )
                        .map_err(settings_error)?;
                        ConfigSettingsListResponse {
                            revision: None,
                            config_path: configuration.config_path,
                            config_found: configuration.config_found,
                            source: ConfigSettingsSource::Effective,
                            path,
                            items,
                        }
                    }
                };
                wire(response)
            }
            SettingsQuery::Resolved => wire(app.resolved_configuration_document()?),
            SettingsQuery::Validate(input) => wire(
                app.runtime_config_settings()
                    .validate_file_settings(input)
                    .map_err(settings_error)?,
            ),
            SettingsQuery::Sources { paths } => {
                validate_source_paths(&paths)?;
                let configuration = app.config_json_sources()?;
                let effective = ConfigSettingsReadResponse {
                    revision: None,
                    config_path: configuration.config_path,
                    config_found: configuration.config_found,
                    source: ConfigSettingsSource::Effective,
                    path: None,
                    value: configuration.effective,
                };
                let input = ConfigSettingsGetInput {
                    source: ConfigSettingsSource::File,
                    ..Default::default()
                };
                let global = app
                    .runtime_config_settings()
                    .read_file_settings(input.clone())
                    .map_err(settings_error)?;
                let workspace = app
                    .runtime_config_settings()
                    .read_project_file_settings(input)
                    .map_err(settings_error)?;
                let mut result = std::collections::BTreeMap::new();
                for path in paths {
                    if result.contains_key(&path) {
                        continue;
                    }
                    let global = settings_leaf(&global, &path)?;
                    result.insert(
                        path.clone(),
                        SettingSources {
                            effective: settings_leaf(&effective, &path)?,
                            file: global.clone(),
                            global,
                            workspace: settings_leaf(&workspace, &path)?,
                        },
                    );
                }
                wire(result)
            }
        })
        .await?
    }

    pub async fn command_settings(
        &self,
        command: SettingsCommand,
    ) -> Result<Value, ApplicationError> {
        let mut response: ConfigSettingsEditResponse = self
            .run_blocking(move |app| {
                match command {
                    SettingsCommand::Set(input) => {
                        app.runtime_config_settings().set_file_setting(input)
                    }
                    SettingsCommand::SetLayer { layer, input } => match layer {
                        ConfigSettingsLayer::Global => {
                            app.runtime_config_settings().set_file_setting(input)
                        }
                        ConfigSettingsLayer::Workspace => app
                            .runtime_config_settings()
                            .set_project_file_setting(input),
                    },
                    SettingsCommand::Patch(input) => {
                        app.runtime_config_settings().patch_file_settings(input)
                    }
                    SettingsCommand::Delete(input) => {
                        app.runtime_config_settings().delete_file_setting(input)
                    }
                    SettingsCommand::DeleteLayer { layer, input } => match layer {
                        ConfigSettingsLayer::Global => {
                            app.runtime_config_settings().delete_file_setting(input)
                        }
                        ConfigSettingsLayer::Workspace => app
                            .runtime_config_settings()
                            .delete_project_file_setting(input),
                    },
                }
                .map_err(settings_error)
            })
            .await??;
        if response.reload_required {
            let report = self
                .runtime_control()
                .reload()
                .await
                .map_err(ApplicationError::from)?;
            response.reload = Some(ConfigSettingsReloadResponse {
                previous_generation: report.previous_generation,
                generation: report.generation,
                loaded_at: report.loaded_at.to_rfc3339(),
                deferred_task_id: report.deferred_task_id,
            });
        }
        wire(response)
    }
}

fn settings_error(error: agena_runtime::RuntimeConfigSettingsError) -> ApplicationError {
    ApplicationError::from_failure_with_diagnostic(error.failure().clone(), error.diagnostic())
}

fn settings_leaf(
    root: &ConfigSettingsReadResponse,
    path: &str,
) -> Result<ConfigSettingsReadResponse, ApplicationError> {
    Ok(ConfigSettingsReadResponse {
        revision: root.revision.clone(),
        config_path: root.config_path.clone(),
        config_found: root.config_found,
        source: root.source,
        path: Some(path.to_owned()),
        value: agena_domain::get_json_path(&root.value, Some(path)).map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("The settings path is invalid.", error)
        })?,
    })
}

fn validate_source_paths(paths: &[String]) -> Result<(), ApplicationError> {
    if paths.is_empty()
        || paths.len() > 64
        || paths
            .iter()
            .any(|path| path.trim().is_empty() || path.len() > 512)
    {
        return Err(ApplicationError::bad_request(
            "The settings batch needs 1 to 64 non-empty paths.",
        ));
    }
    if serde_json::to_vec(paths)
        .map_err(|error| ApplicationError::internal_error(&error))?
        .len()
        > 16_384
    {
        return Err(ApplicationError::bad_request(
            "The settings path batch is too large.",
        ));
    }
    for path in paths {
        agena_domain::parse_json_path(path).map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("The settings path is invalid.", error)
        })?;
    }
    Ok(())
}
