impl App {
    pub(crate) fn settings_model_editable(&self, purpose: Purpose, explicit_local: bool) -> bool {
        if explicit_local {
            return true;
        }
        if purpose == Purpose::RuntimeOverride {
            return self.transcript.session_id.is_none()
                || super::session_sources::session_model_route_is_local(
                    &self.current_raw_model_selection(),
                );
        }
        let Some((workspace, _)) = settings_model_target(purpose) else {
            return false;
        };
        !workspace
            || crate::app_backend::config::config_json_sources(&self.application)
                .ok()
                .and_then(|sources| settings_model_projection(&sources, purpose))
                .is_some_and(|projection| projection.local.is_some())
    }

    pub(crate) fn settings_model_source_prompt(
        &self,
        purpose: Purpose,
        explicit_local: bool,
    ) -> String {
        if purpose == Purpose::RuntimeOverride && !explicit_local {
            return self.session_model_source_prompt();
        }
        let projection = crate::app_backend::config::config_json_sources(&self.application)
            .ok()
            .and_then(|sources| settings_model_projection(&sources, purpose));
        let (source, value) = match projection {
            Some(projection) => (
                if explicit_local {
                    self.i18n.text("settings-layer-local")
                } else if projection.workspace {
                    projection.source_label(&self.i18n)
                } else {
                    self.i18n.text("settings-layer-parent-global")
                },
                projection.row_value(&self.i18n),
            ),
            None => (
                self.i18n.text(if explicit_local {
                    "settings-layer-local"
                } else {
                    "settings-source-unavailable"
                }),
                self.i18n.text(if explicit_local {
                    "settings-source-unset"
                } else {
                    "settings-source-unavailable"
                }),
            ),
        };
        format!(
            "{}: {source} · {value}",
            self.i18n.text("settings-layer-source-title")
        )
    }

    pub(crate) fn open_settings_model_source(&mut self, purpose: Purpose) {
        if purpose == Purpose::RuntimeOverride {
            self.open_session_model_source();
            return;
        }
        let Some((workspace, path)) = settings_model_target(purpose) else {
            return;
        };
        let Ok(sources) = crate::app_backend::config::config_json_sources(&self.application) else {
            return;
        };
        let field = crate::SettingsFieldSpec {
            explicit_local: false,
            workspace,
            path: path.into(),
            section: crate::SettingsStudioSectionId::ModelsProviders,
            kind: crate::SettingsFieldKind::String,
            label_key: "overlay-session-model-title",
            description_key: "settings-layer-group-model",
            label_override: None,
            description_override: None,
        };
        self.open_settings_layer_source(field, Some(purpose), &sources);
    }

    pub(crate) fn current_auxiliary_model_ref(&self, workspace: bool) -> Option<ModelRef> {
        self.current_settings_model_ref(if workspace {
            Purpose::WorkspaceAuxiliary
        } else {
            Purpose::Auxiliary
        })
    }

    pub(crate) fn current_global_default_model_ref(&self) -> Option<ModelRef> {
        self.current_settings_model_ref(Purpose::GlobalDefault)
    }

    fn current_settings_model_ref(&self, purpose: Purpose) -> Option<ModelRef> {
        let sources = crate::app_backend::config::config_json_sources(&self.application).ok()?;
        let projection = settings_model_projection(&sources, purpose)?;
        settings_model_ref(projection.value.as_ref()?)
    }

    pub(crate) fn settings_model_initial_modes(
        &self,
        purpose: Purpose,
        model: &ModelRef,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let Some(sources) = crate::app_backend::config::config_json_sources(&self.application).ok()
        else {
            return (None, None, None);
        };
        settings_model_initial_modes(&sources, purpose, model)
    }

    pub(crate) fn finish_model_selection(
        &mut self,
        purpose: agena_tui::model_chooser::SessionModelChooserPurpose,
        explicit_local: bool,
        model: ModelRef,
        thinking_mode: Option<String>,
        speed_mode: Option<String>,
        verbosity: Option<String>,
    ) -> bool {
        if !self.settings_model_editable(purpose, explicit_local) {
            self.flash_warning(crate::settings_inherited_locked_message(
                &self.i18n,
                purpose == Purpose::RuntimeOverride,
            ));
            return false;
        }
        if purpose == Purpose::RuntimeOverride {
            self.finish_model_selection_persisted(purpose, model, Ok(()));
            return true;
        }
        let sources = match crate::app_backend::config::config_json_sources(&self.application) {
            Ok(sources) => sources,
            Err(error) => {
                self.flash_error(crate::UiFailure::internal(error));
                return false;
            }
        };
        let Some((workspace, path)) = settings_model_target(purpose) else {
            return false;
        };
        let projection = crate::SettingsFieldProjection::at_path(&sources, path, workspace);
        let selection = model_selection_value(
            projection.local.as_ref(),
            &model,
            thinking_mode,
            speed_mode,
            verbosity,
        );
        self.dispatch_backend_operation(
            move |application| async move {
                if workspace {
                    application
                        .set_workspace_config_setting(path, selection)
                        .await
                } else {
                    application.set_config_setting(path, selection).await
                }
            },
            move |app, result| {
                app.finish_model_selection_persisted(purpose, model, result.map(|_| ()))
            },
        );
        false
    }

    fn finish_model_selection_persisted(
        &mut self,
        purpose: agena_tui::model_chooser::SessionModelChooserPurpose,
        model: ModelRef,
        result: UiResult<()>,
    ) {
        match result {
            Ok(()) => {
                self.flash_success(self.i18n.text_args(
                    match purpose {
                        agena_tui::model_chooser::SessionModelChooserPurpose::Auxiliary
                        | agena_tui::model_chooser::SessionModelChooserPurpose::WorkspaceAuxiliary => {
                            "flash-auxiliary-model-updated"
                        }
                        agena_tui::model_chooser::SessionModelChooserPurpose::PermissionApproval => {
                            "flash-permission-approval-model-updated"
                        }
                        agena_tui::model_chooser::SessionModelChooserPurpose::WorkspacePermissionApproval => {
                            "flash-workspace-approval-model-updated"
                        }
                        agena_tui::model_chooser::SessionModelChooserPurpose::GlobalDefault => {
                            "flash-global-default-model-updated"
                        }
                        agena_tui::model_chooser::SessionModelChooserPurpose::WorkspaceDefault => {
                            "flash-workspace-default-model-updated"
                        }
                        agena_tui::model_chooser::SessionModelChooserPurpose::RuntimeOverride => {
                            "flash-model-selected"
                        }
                    },
                    &agena_tui::fl_args!(
                        "provider" => model.provider_id.to_string(),
                        "model" => model.model_id.to_string(),
                    ),
                ));
                self.refresh_tui_preferences_from_runtime();
                self.current_route = self
                    .route_stack
                    .pop()
                    .map(|route| self.refresh_restored_route(route))
                    .unwrap_or(crate::Route::Main);
            }
            Err(error) => {
                self.flash_error(error);
            }
        }
    }

    pub(crate) fn current_workspace_default_model_ref(&self) -> Option<ModelRef> {
        self.current_settings_model_ref(Purpose::WorkspaceDefault)
    }

    pub(crate) fn current_workspace_permission_approval_model_ref(&self) -> Option<ModelRef> {
        self.current_settings_model_ref(Purpose::WorkspacePermissionApproval)
    }

    pub(crate) fn current_permission_approval_model_ref(&self) -> Option<ModelRef> {
        self.current_settings_model_ref(Purpose::PermissionApproval)
    }
}

fn settings_model_target(purpose: Purpose) -> Option<(bool, &'static str)> {
    Some(match purpose {
        Purpose::GlobalDefault => (false, "providers.default_selection"),
        Purpose::WorkspaceDefault => (true, "providers.default_selection"),
        Purpose::Auxiliary => (false, "runtime.auxiliary_model"),
        Purpose::WorkspaceAuxiliary => (true, "runtime.auxiliary_model"),
        Purpose::PermissionApproval => (false, "permission.approval_model"),
        Purpose::WorkspacePermissionApproval => (true, "permission.approval_model"),
        Purpose::RuntimeOverride => return None,
    })
}

fn settings_model_projection(
    sources: &crate::ConfigJsonSources,
    purpose: Purpose,
) -> Option<crate::SettingsFieldProjection> {
    let (workspace, path) = settings_model_target(purpose)?;
    Some(crate::SettingsFieldProjection::at_path(
        sources, path, workspace,
    ))
}

fn settings_model_ref(value: &JsonValue) -> Option<ModelRef> {
    serde_json::from_value::<agena_domain::ExecutionSelection>(value.clone())
        .ok()?
        .model_ref()
        .ok()
        .flatten()
}

fn settings_model_initial_modes(
    sources: &crate::ConfigJsonSources,
    purpose: Purpose,
    model: &ModelRef,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(value) =
        settings_model_projection(sources, purpose).and_then(|projection| projection.value)
    else {
        return (None, None, None);
    };
    let Ok(selection) = serde_json::from_value::<agena_domain::ExecutionSelection>(value) else {
        return (None, None, None);
    };
    if selection.model_ref().ok().flatten().as_ref() != Some(model) {
        return (None, None, None);
    }
    (
        selection.thinking_mode,
        selection.speed_mode,
        selection.verbosity,
    )
}

fn model_selection_value(
    local: Option<&JsonValue>,
    model: &ModelRef,
    thinking_mode: Option<String>,
    speed_mode: Option<String>,
    verbosity: Option<String>,
) -> JsonValue {
    // The chooser edits identity and its three model-specific modes. Keep
    // other raw settings (including explicit false and auxiliary permissions)
    // without pinning values from a parent or a later runtime layer.
    let mut selection = local
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    for key in [
        "provider",
        "adapter",
        "model",
        "thinking_mode",
        "speed_mode",
        "verbosity",
    ] {
        selection.remove(key);
    }
    selection.insert(
        "provider".to_owned(),
        JsonValue::String(model.provider_id.to_string()),
    );
    if let Some(adapter_id) = model.adapter_id.as_ref() {
        selection.insert(
            "adapter".to_owned(),
            JsonValue::String(adapter_id.to_string()),
        );
    }
    selection.insert(
        "model".to_owned(),
        JsonValue::String(model.model_id.to_string()),
    );
    insert_optional_selection_value(&mut selection, "thinking_mode", thinking_mode);
    insert_optional_selection_value(&mut selection, "speed_mode", speed_mode);
    insert_optional_selection_value(&mut selection, "verbosity", verbosity);
    JsonValue::Object(selection)
}

fn insert_optional_selection_value(
    selection: &mut JsonMap<String, JsonValue>,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        selection.insert(key.to_owned(), JsonValue::String(value));
    }
}

use crate::{App, JsonMap, JsonValue, ModelRef, UiResult};
use agena_tui::model_chooser::SessionModelChooserPurpose as Purpose;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sources(global: JsonValue, workspace: JsonValue) -> crate::ConfigJsonSources {
        crate::ConfigJsonSources {
            config_path: Default::default(),
            config_found: true,
            project_config_path: Default::default(),
            project_config_found: true,
            applied_layers: vec![],
            file: global.clone(),
            project_file: workspace.clone(),
            defaults: json!({}),
            global_resolved: global.clone(),
            workspace_resolved: workspace,
            global_inherited: JsonValue::Null,
            workspace_inherited: global,
            effective: json!({
                "providers":{"default_selection":{"provider":"runtime","model":"later"}},
                "runtime":{"auxiliary_model":{"provider":"runtime","model":"later"}},
                "permission":{"approval_model":{"provider":"runtime","model":"later"}},
            }),
        }
    }

    #[test]
    fn settings_model_current_values_stay_at_the_edited_layer_for_all_model_groups() {
        let configured = json!({
            "providers":{"default_selection":{"provider":"global","model":"main"}},
            "runtime":{"auxiliary_model":{"provider":"global","model":"aux"}},
            "permission":{"approval_model":{"provider":"global","model":"approve"}},
        });
        let mut state = sources(configured.clone(), json!({}));
        state.workspace_resolved = configured;
        for (global, workspace, expected) in [
            (Purpose::GlobalDefault, Purpose::WorkspaceDefault, "main"),
            (Purpose::Auxiliary, Purpose::WorkspaceAuxiliary, "aux"),
            (
                Purpose::PermissionApproval,
                Purpose::WorkspacePermissionApproval,
                "approve",
            ),
        ] {
            for purpose in [global, workspace] {
                let projection = settings_model_projection(&state, purpose).unwrap();
                let reference = settings_model_ref(projection.value.as_ref().unwrap()).unwrap();
                assert_eq!(reference.provider_id.to_string(), "global");
                assert_eq!(reference.model_id.to_string(), expected);
            }
        }
        state.file = json!({});
        state.global_resolved = json!({});
        for purpose in [
            Purpose::GlobalDefault,
            Purpose::Auxiliary,
            Purpose::PermissionApproval,
        ] {
            assert_eq!(
                settings_model_projection(&state, purpose).unwrap().value,
                Some(JsonValue::Null)
            );
        }
    }

    #[test]
    fn settings_model_partial_groups_do_not_borrow_parent_model_identity() {
        let mut state = sources(
            json!({"providers":{"default_selection":{"provider":"parent","model":"base"}}}),
            json!({"providers":{"default_selection":{"verbosity":"low"}}}),
        );
        let projection = settings_model_projection(&state, Purpose::WorkspaceDefault).unwrap();
        assert!(settings_model_ref(projection.value.as_ref().unwrap()).is_none());
        state.global_resolved = JsonValue::Null;
        state.defaults = JsonValue::Null;
        state.file = json!({});
        assert!(
            settings_model_projection(&state, Purpose::GlobalDefault)
                .unwrap()
                .value
                .is_none()
        );
    }

    #[test]
    fn settings_model_reselect_preserves_current_modes_but_new_identity_starts_fresh() {
        let state = sources(
            json!({"providers":{"default_selection":{
                "provider":"provider","adapter":"adapter","model":"same",
                "thinking_mode":"high","speed_mode":"fast","verbosity":"low",
                "parallel_tool_calls":false
            }}}),
            json!({}),
        );
        let same = ModelRef::try_new_with_adapter("provider", "adapter", "same").unwrap();
        assert_eq!(
            settings_model_initial_modes(&state, Purpose::GlobalDefault, &same),
            (Some("high".into()), Some("fast".into()), Some("low".into()))
        );
        let different = ModelRef::try_new_with_adapter("provider", "adapter", "different").unwrap();
        assert_eq!(
            settings_model_initial_modes(&state, Purpose::GlobalDefault, &different),
            (None, None, None)
        );
    }

    #[test]
    fn settings_model_direct_edits_keep_unedited_raw_fields_and_remove_stale_mode_values() {
        let raw = json!({
            "provider":"old","adapter":"old-adapter","model":"old-model",
            "thinking_mode":"high","speed_mode":"fast","verbosity":"high",
            "parallel_tool_calls":false,
            "permission":{"tools":{"default":"ask"}},
        });
        let model = ModelRef::try_new("new", "new-model").unwrap();
        let saved = model_selection_value(Some(&raw), &model, None, Some("slow".into()), None);
        assert_eq!(
            saved,
            json!({
                "provider":"new","model":"new-model","speed_mode":"slow",
                "parallel_tool_calls":false,"permission":{"tools":{"default":"ask"}},
            })
        );
        let saved_without_local = model_selection_value(None, &model, None, None, None);
        assert_eq!(
            saved_without_local,
            json!({"provider":"new","model":"new-model"})
        );
        assert_eq!(raw["adapter"], "old-adapter");
    }
}
