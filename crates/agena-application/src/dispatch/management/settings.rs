//! Settings reads and mutations shared by all application transports.
use super::wire;
use crate::{Application, ApplicationError};
use agena_api::management::{SettingsCommand, SettingsQuery, settings::*};
use agena_api::resource::{ConfigSettingsEditResponse, ConfigSettingsReloadResponse};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(serde::Serialize)]
struct SettingSources {
    effective: ConfigSettingsReadResponse,
    file: ConfigSettingsReadResponse,
    global: ConfigSettingsReadResponse,
    workspace: ConfigSettingsReadResponse,
}

#[derive(serde::Serialize)]
struct WorkspaceSettings {
    #[serde(flatten)]
    settings: ConfigSettingsReadResponse,
    workspace_id: Option<i64>,
    active_workspace: bool,
}

impl Application {
    pub async fn query_settings(&self, query: SettingsQuery) -> Result<Value, ApplicationError> {
        let (workspace_id, workspace_root) = match &query {
            SettingsQuery::GetLayer {
                layer,
                workspace_id,
                input,
            } => {
                let path = input.target.path.as_deref();
                let root = self
                    .settings_workspace_root(*layer, *workspace_id, path)
                    .await?;
                let id = if *layer == ConfigSettingsLayer::Workspace
                    && workspace_security_setting(path)
                    && workspace_id.is_none()
                {
                    self.service().server_workspace_id().await?
                } else {
                    *workspace_id
                };
                (id, root)
            }
            _ => (None, None),
        };
        self.run_blocking(move |app| match query {
            SettingsQuery::Get(input) => {
                let path = input.target.path.clone();
                let response = match input.source {
                    ConfigSettingsSource::File => app
                        .runtime_config_settings()
                        .read_file_settings(input)
                        .map_err(settings_error)?,
                    ConfigSettingsSource::Effective => {
                        let configuration = app
                            .runtime_configuration()
                            .runtime_configuration()
                            .map_err(|error| ApplicationError::internal_error(&error))?;
                        let value = agena_domain::get_json_path(
                            &configuration.effective_config,
                            path.as_deref(),
                        )
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
                            has_override: None,
                            override_path: None,
                            requires_group_reset: None,
                            default_value: None,
                            layer_value: None,
                            inherited_value: None,
                        }
                    }
                };
                wire(response)
            }
            SettingsQuery::GetLayer {
                layer, mut input, ..
            } => {
                input.source = ConfigSettingsSource::File;
                let workspace_root = app.alternate_settings_workspace(workspace_root.as_deref());
                let settings = match layer {
                    ConfigSettingsLayer::Global => {
                        app.runtime_config_settings().read_file_settings(input)
                    }
                    ConfigSettingsLayer::Workspace => app
                        .runtime_config_settings()
                        .read_project_file_settings(input, workspace_root),
                }
                .map_err(settings_error)?;
                if layer == ConfigSettingsLayer::Workspace
                    && workspace_security_setting(settings.path.as_deref())
                {
                    wire(WorkspaceSettings {
                        settings,
                        workspace_id,
                        active_workspace: workspace_root.is_none(),
                    })
                } else {
                    wire(settings)
                }
            }
            SettingsQuery::List(input) => {
                let path = input.target.path.clone();
                let response = match input.source {
                    ConfigSettingsSource::File => app
                        .runtime_config_settings()
                        .list_file_settings(input)
                        .map_err(settings_error)?,
                    ConfigSettingsSource::Effective => {
                        let configuration = app
                            .runtime_configuration()
                            .runtime_configuration()
                            .map_err(|error| ApplicationError::internal_error(&error))?;
                        let items = agena_runtime::list_json_path(
                            &configuration.effective_config,
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
                // Resolve each editable root once below, then slice all paths.
                // The effective snapshot does not need another pair of file reads.
                let configuration = app
                    .runtime_configuration()
                    .runtime_configuration()
                    .map_err(|error| ApplicationError::internal_error(&error))?;
                let effective = ConfigSettingsReadResponse {
                    revision: None,
                    config_path: configuration.config_path,
                    config_found: configuration.config_found,
                    source: ConfigSettingsSource::Effective,
                    path: None,
                    value: configuration.effective_config,
                    has_override: None,
                    override_path: None,
                    requires_group_reset: None,
                    default_value: None,
                    layer_value: None,
                    inherited_value: None,
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
                    .read_project_file_settings(input, None)
                    .map_err(settings_error)?;
                let mut result = std::collections::BTreeMap::new();
                for path in paths {
                    if result.contains_key(&path) {
                        continue;
                    }
                    let global = settings_leaf(&global, ConfigSettingsLayer::Global, &path)?;
                    result.insert(
                        path.clone(),
                        SettingSources {
                            effective: settings_leaf(
                                &effective,
                                ConfigSettingsLayer::Global,
                                &path,
                            )?,
                            file: global.clone(),
                            global,
                            workspace: settings_leaf(
                                &workspace,
                                ConfigSettingsLayer::Workspace,
                                &path,
                            )?,
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
        let workspace_root = match &command {
            SettingsCommand::SetLayer {
                layer,
                workspace_id,
                input,
            } => {
                self.settings_workspace_root(*layer, *workspace_id, Some(&input.path))
                    .await?
            }
            SettingsCommand::DeleteLayer {
                layer,
                workspace_id,
                input,
            } => {
                self.settings_workspace_root(*layer, *workspace_id, Some(&input.path))
                    .await?
            }
            _ => None,
        };
        let mut response: ConfigSettingsEditResponse = self
            .run_blocking(move |app| {
                let workspace_root = app.alternate_settings_workspace(workspace_root.as_deref());
                match command {
                    SettingsCommand::Set(input) => {
                        app.runtime_config_settings().set_file_setting(input)
                    }
                    SettingsCommand::SetLayer { layer, input, .. } => match layer {
                        ConfigSettingsLayer::Global => {
                            app.runtime_config_settings().set_file_setting(input)
                        }
                        ConfigSettingsLayer::Workspace => app
                            .runtime_config_settings()
                            .set_project_file_setting(input, workspace_root),
                    },
                    SettingsCommand::Patch(input) => {
                        app.runtime_config_settings().patch_file_settings(input)
                    }
                    SettingsCommand::Delete(input) => {
                        app.runtime_config_settings().delete_file_setting(input)
                    }
                    SettingsCommand::DeleteLayer { layer, input, .. } => match layer {
                        ConfigSettingsLayer::Global => {
                            app.runtime_config_settings().delete_file_setting(input)
                        }
                        ConfigSettingsLayer::Workspace => app
                            .runtime_config_settings()
                            .delete_project_file_setting(input, workspace_root),
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

    async fn settings_workspace_root(
        &self,
        layer: ConfigSettingsLayer,
        workspace_id: Option<i64>,
        path: Option<&str>,
    ) -> Result<Option<PathBuf>, ApplicationError> {
        let Some(id) = workspace_id else {
            return Ok(None);
        };
        if layer != ConfigSettingsLayer::Workspace || !workspace_security_setting(path) {
            return Err(ApplicationError::bad_request(
                "workspace_id is available for workspace permission, sandbox, and resource preflight settings",
            ));
        }
        if id <= 0 {
            return Err(ApplicationError::bad_request(
                "workspace_id must be positive",
            ));
        }
        self.service()
            .registered_workspace_path(id)
            .await
            .map(|path| Some(path.into()))
    }

    fn alternate_settings_workspace<'a>(&self, root: Option<&'a Path>) -> Option<&'a Path> {
        let active = self.workspace_root();
        root.filter(|root| {
            *root != active
                && !root
                    .canonicalize()
                    .ok()
                    .zip(active.canonicalize().ok())
                    .is_some_and(|(left, right)| left == right)
        })
    }
}

fn workspace_security_setting(path: Option<&str>) -> bool {
    matches!(path, Some("permission" | "sandbox" | "resource_guard"))
}

fn settings_error(error: agena_runtime::RuntimeConfigSettingsError) -> ApplicationError {
    ApplicationError::from_failure_with_diagnostic(error.failure().clone(), error.diagnostic())
}

fn settings_leaf(
    root: &ConfigSettingsReadResponse,
    layer: ConfigSettingsLayer,
    path: &str,
) -> Result<ConfigSettingsReadResponse, ApplicationError> {
    let override_path = if root.source == ConfigSettingsSource::File && root.config_found {
        let boundary = match layer {
            ConfigSettingsLayer::Global => {
                agena_domain::config_setting_override_path(&root.value, Some(path))
            }
            ConfigSettingsLayer::Workspace => {
                agena_domain::config_setting_inheritance_override_path(&root.value, Some(path))
            }
        };
        boundary.map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("The settings path is invalid.", error)
        })?
    } else {
        None
    };
    let requires_group_reset = if root.source == ConfigSettingsSource::File {
        Some(
            agena_domain::config_setting_requires_group_reset(
                &root.value,
                Some(path),
                override_path.as_deref(),
            )
            .map_err(|error| {
                ApplicationError::bad_request_with_diagnostic(
                    "The settings path is invalid.",
                    error,
                )
            })?,
        )
    } else {
        None
    };
    Ok(ConfigSettingsReadResponse {
        revision: root.revision.clone(),
        config_path: root.config_path.clone(),
        config_found: root.config_found,
        source: root.source,
        path: Some(path.to_owned()),
        value: agena_domain::get_json_path(&root.value, Some(path)).map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("The settings path is invalid.", error)
        })?,
        has_override: (root.source == ConfigSettingsSource::File)
            .then_some(override_path.is_some()),
        override_path,
        requires_group_reset,
        default_value: settings_projection_leaf(root.default_value.as_ref(), path)?,
        layer_value: settings_projection_leaf(root.layer_value.as_ref(), path)?,
        inherited_value: settings_projection_leaf(root.inherited_value.as_ref(), path)?,
    })
}

fn settings_projection_leaf(
    document: Option<&Value>,
    path: &str,
) -> Result<Option<Value>, ApplicationError> {
    document
        .map(|value| agena_domain::get_json_path(value, Some(path)))
        .transpose()
        .map_err(|error| {
            ApplicationError::bad_request_with_diagnostic("The settings path is invalid.", error)
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

#[cfg(test)]
mod layer_source_tests {
    use super::*;

    #[test]
    fn batch_leaves_preserve_non_permission_parent_values_and_revisions() {
        let root = ConfigSettingsReadResponse {
            revision: Some("selected-revision".into()),
            config_path: "/fixture/workspace.json".into(),
            config_found: true,
            source: ConfigSettingsSource::File,
            path: None,
            value: serde_json::json!({"sandbox":true,"ui":{"tui":{"width":40}}}),
            has_override: Some(true),
            override_path: Some(String::new()),
            requires_group_reset: None,
            default_value: Some(serde_json::json!({"sandbox":true,"ui":{}})),
            layer_value: Some(
                serde_json::json!({"sandbox":true,"ui":{"tui":{"width":40,"theme":""}}}),
            ),
            inherited_value: Some(
                serde_json::json!({"sandbox":false,"ui":{"tui":{"width":0,"theme":""}}}),
            ),
        };
        let sandbox = settings_leaf(&root, ConfigSettingsLayer::Workspace, "sandbox").unwrap();
        assert_eq!(sandbox.value, true);
        assert_eq!(sandbox.has_override, Some(true));
        assert_eq!(sandbox.inherited_value, Some(serde_json::json!(false)));
        assert_eq!(sandbox.default_value, Some(serde_json::json!(true)));
        assert_eq!(sandbox.layer_value, Some(serde_json::json!(true)));
        assert_eq!(sandbox.revision, root.revision);
        let width = settings_leaf(&root, ConfigSettingsLayer::Workspace, "ui.tui.width").unwrap();
        assert_eq!(width.value, 40);
        assert_eq!(width.inherited_value, Some(serde_json::json!(0)));
        assert_eq!(width.default_value, Some(Value::Null));
        assert_eq!(width.layer_value, Some(serde_json::json!(40)));
        let theme = settings_leaf(&root, ConfigSettingsLayer::Workspace, "ui.tui.theme").unwrap();
        assert!(theme.value.is_null());
        assert_eq!(theme.has_override, Some(false));
        assert_eq!(theme.inherited_value, Some(serde_json::json!("")));
    }

    #[test]
    fn batch_leaves_keep_unavailable_parent_distinct_from_an_unset_path() {
        let mut root = ConfigSettingsReadResponse {
            revision: None,
            config_path: "/fixture/global.json".into(),
            config_found: false,
            source: ConfigSettingsSource::File,
            path: None,
            value: serde_json::json!({}),
            has_override: Some(false),
            override_path: None,
            requires_group_reset: None,
            default_value: Some(serde_json::json!({"sandbox":true})),
            layer_value: None,
            inherited_value: None,
        };
        assert_eq!(
            settings_leaf(&root, ConfigSettingsLayer::Global, "ui.tui.theme")
                .unwrap()
                .inherited_value,
            None
        );
        root.inherited_value = Some(serde_json::json!({}));
        assert_eq!(
            settings_leaf(&root, ConfigSettingsLayer::Global, "ui.tui.theme")
                .unwrap()
                .inherited_value,
            Some(Value::Null)
        );
    }

    #[test]
    fn batch_global_defaults_and_resolved_null_do_not_invent_a_parent() {
        let root = ConfigSettingsReadResponse {
            revision: Some("global-revision".into()),
            config_path: "/fixture/global.json".into(),
            config_found: true,
            source: ConfigSettingsSource::File,
            path: None,
            value: serde_json::json!({"sandbox":false,"ui":{"tui":{"theme":null}}}),
            has_override: Some(true),
            override_path: Some(String::new()),
            requires_group_reset: None,
            default_value: Some(serde_json::json!({"sandbox":true,"ui":{}})),
            layer_value: Some(serde_json::json!({"sandbox":false,"ui":{"tui":{"theme":null}}})),
            inherited_value: None,
        };
        let sandbox = settings_leaf(&root, ConfigSettingsLayer::Global, "sandbox").unwrap();
        assert_eq!(sandbox.value, false);
        assert_eq!(sandbox.default_value, Some(serde_json::json!(true)));
        assert_eq!(sandbox.layer_value, Some(serde_json::json!(false)));
        assert_eq!(sandbox.inherited_value, None);
        let theme = settings_leaf(&root, ConfigSettingsLayer::Global, "ui.tui.theme").unwrap();
        assert_eq!(theme.default_value, Some(Value::Null));
        assert_eq!(theme.layer_value, Some(Value::Null));
        let wire = serde_json::to_value(theme).unwrap();
        assert!(wire.get("default_value").is_some_and(Value::is_null));
        assert!(wire.get("layer_value").is_some_and(Value::is_null));
        assert!(wire.get("inherited_value").is_none());
    }

    #[test]
    fn batch_atomic_boundaries_distinguish_global_defaults_from_workspace_inheritance() {
        let root = ConfigSettingsReadResponse {
            revision: Some("atomic-revision".into()),
            config_path: "/fixture/config.json".into(),
            config_found: true,
            source: ConfigSettingsSource::File,
            path: None,
            value: serde_json::json!({
                "providers":{"default_selection":{"provider":"local","model":"main"}},
                "permission":{"tools":{"rules":{"shell.*":{"routine":"allow"}}}},
                "ui":{"tui":null}
            }),
            has_override: Some(true),
            override_path: Some(String::new()),
            requires_group_reset: Some(false),
            default_value: None,
            layer_value: None,
            inherited_value: None,
        };
        let mode = "providers.default_selection.thinking_mode";
        let global = settings_leaf(&root, ConfigSettingsLayer::Global, mode).unwrap();
        assert_eq!(global.has_override, Some(false));
        assert_eq!(global.override_path, None);
        let workspace = settings_leaf(&root, ConfigSettingsLayer::Workspace, mode).unwrap();
        assert_eq!(workspace.has_override, Some(true));
        assert_eq!(
            workspace.override_path.as_deref(),
            Some("providers.default_selection")
        );
        assert_eq!(workspace.requires_group_reset, Some(false));
        for layer in [ConfigSettingsLayer::Global, ConfigSettingsLayer::Workspace] {
            let rule = settings_leaf(&root, layer, "permission.tools.rules.\"shell.*\".dangerous")
                .unwrap();
            assert_eq!(
                rule.override_path.as_deref(),
                Some("permission.tools.rules.\"shell.*\"")
            );
            assert_eq!(rule.requires_group_reset, Some(false));
            let opaque = settings_leaf(&root, layer, "ui.tui.theme").unwrap();
            assert_eq!(opaque.override_path.as_deref(), Some("ui.tui"));
            assert_eq!(opaque.requires_group_reset, Some(true));
        }
    }

    #[test]
    fn batch_leaves_distinguish_local_null_clears_from_missing_and_optional_nulls() {
        let mut root = ConfigSettingsReadResponse {
            revision: Some("raw-revision".into()),
            config_path: "/fixture/workspace.json".into(),
            config_found: true,
            source: ConfigSettingsSource::File,
            path: None,
            value: serde_json::json!({"sandbox":null,"ui":{"locale":null,"tui":{"theme":null}}}),
            has_override: Some(true),
            override_path: Some(String::new()),
            requires_group_reset: None,
            default_value: Some(serde_json::json!({"sandbox":true,"ui":{}})),
            layer_value: Some(
                serde_json::json!({"sandbox":false,"ui":{"locale":"en-US","tui":{"theme":null,"width":40}}}),
            ),
            inherited_value: Some(
                serde_json::json!({"sandbox":false,"ui":{"locale":"en-US","tui":{"theme":"dark","width":40}}}),
            ),
        };
        for (path, overridden) in [
            ("sandbox", false),
            ("ui.locale", false),
            ("ui.tui.theme", true),
            ("ui.tui.width", false),
        ] {
            let leaf = settings_leaf(&root, ConfigSettingsLayer::Workspace, path).unwrap();
            assert!(leaf.value.is_null());
            assert_eq!(leaf.has_override, Some(overridden), "{path}");
            assert_eq!(
                leaf.override_path.as_deref(),
                overridden.then_some(path),
                "{path}"
            );
            assert_eq!(leaf.revision, root.revision);
            assert_eq!(
                serde_json::to_value(leaf).unwrap()["has_override"],
                overridden
            );
        }
        root.value["ui"]["tui"] = Value::Null;
        let subtree_clear =
            settings_leaf(&root, ConfigSettingsLayer::Workspace, "ui.tui.theme").unwrap();
        assert_eq!(subtree_clear.has_override, Some(true));
        assert_eq!(subtree_clear.override_path.as_deref(), Some("ui.tui"));
        assert!(subtree_clear.value.is_null());
        root.source = ConfigSettingsSource::Effective;
        let effective =
            settings_leaf(&root, ConfigSettingsLayer::Workspace, "ui.tui.theme").unwrap();
        assert_eq!(effective.has_override, None);
        assert_eq!(effective.override_path, None);
        assert!(
            serde_json::to_value(effective)
                .unwrap()
                .get("has_override")
                .is_none()
        );
    }
}
