impl App {
    pub(crate) fn activate_permission_rule_studio_selection(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
    ) -> bool {
        let Some(item) = dialog.presentation.list.selected_item().cloned() else {
            return false;
        };
        match item.action {
            PermissionRuleStudioAction::SubjectKind => self.open_permission_rule_choice_overlay(
                dialog,
                PermissionRuleStudioChoiceField::SubjectKind,
            ),
            PermissionRuleStudioAction::PathAccessKind => self.open_permission_rule_choice_overlay(
                dialog,
                PermissionRuleStudioChoiceField::PathAccessKind,
            ),
            PermissionRuleStudioAction::Scope => self.open_permission_rule_choice_overlay(
                dialog,
                PermissionRuleStudioChoiceField::Scope,
            ),
            PermissionRuleStudioAction::Mode => self
                .open_permission_rule_choice_overlay(dialog, PermissionRuleStudioChoiceField::Mode),
            PermissionRuleStudioAction::ToolName => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::ToolName,
            ),
            PermissionRuleStudioAction::Qualifier => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::Qualifier,
            ),
            PermissionRuleStudioAction::WorkspaceRoot => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::WorkspaceRoot,
            ),
            PermissionRuleStudioAction::TargetPath => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::TargetPath,
            ),
            PermissionRuleStudioAction::NetworkTarget => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::NetworkTarget,
            ),
            PermissionRuleStudioAction::SessionId => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::SessionId,
            ),
        }
        false
    }

    pub(crate) fn open_permission_rule_choice_overlay(
        &mut self,
        dialog: &PermissionRuleStudioOverlay,
        field: PermissionRuleStudioChoiceField,
    ) {
        let (title, prompt, input, all_items, allow_clear) =
            permission_rule_choice_overlay_spec(&self.i18n, &dialog.draft, field);
        self.open_choice_overlay(self.build_choice_overlay(
            title,
            prompt,
            Some(input.text().to_string()),
            all_items,
            ChoiceOverlayAction::PermissionRuleStudio(field),
            allow_clear,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        ));
    }

    pub(crate) fn open_permission_rule_studio_editor(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
        field: PermissionRuleStudioEditField,
    ) {
        let (title, prompt, footer, value) =
            permission_rule_editor_spec(&self.i18n, &dialog.draft, field);
        dialog.editor = Some(PermissionRuleStudioEditor::new(
            title,
            prompt,
            footer,
            false,
            Editor::from_text(value),
            field,
        ));
    }

    pub(crate) fn commit_permission_rule_studio_editor(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
        field: PermissionRuleStudioEditField,
        input: String,
    ) -> UiResult<()> {
        match field {
            PermissionRuleStudioEditField::ToolName => {
                dialog.draft.tool_name = input.trim().to_string();
            }
            PermissionRuleStudioEditField::Qualifier => {
                dialog.draft.qualifier = input.trim().to_string();
            }
            PermissionRuleStudioEditField::WorkspaceRoot => {
                dialog.draft.workspace_root = input.trim().to_string();
            }
            PermissionRuleStudioEditField::TargetPath => {
                dialog.draft.target_path = input.trim().to_string();
            }
            PermissionRuleStudioEditField::NetworkTarget => {
                dialog.draft.network_target = input.trim().to_string();
            }
            PermissionRuleStudioEditField::SessionId => {
                let trimmed = input.trim();
                if !trimmed.is_empty() && trimmed.parse::<i64>().is_err() {
                    return Err(crate::UiFailure::message(ui_text::t(
                        &self.i18n,
                        "permission-rule-error-session-id-integer",
                    )));
                }
                dialog.draft.session_id = trimmed.to_string();
            }
        }
        self.refresh_permission_rule_studio(dialog);
        Ok(())
    }

    pub(crate) fn commit_permission_rule_studio_save(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
    ) -> UiResult<()> {
        let draft = dialog.draft.clone();
        match draft.subject_kind {
            PermissionRuleSubjectKind::Tool if draft.tool_name.trim().is_empty() => {
                return Err(crate::UiFailure::message(ui_text::t(
                    &self.i18n,
                    "permission-rule-error-tool-name-required",
                )));
            }
            PermissionRuleSubjectKind::PathAccess => {
                if draft.path_access_kind.trim().is_empty() {
                    return Err(crate::UiFailure::message(ui_text::t(
                        &self.i18n,
                        "permission-rule-error-path-access-kind-required",
                    )));
                }
                if draft.target_path.trim().is_empty() {
                    return Err(crate::UiFailure::message(ui_text::t(
                        &self.i18n,
                        "permission-rule-error-target-path-required",
                    )));
                }
            }
            PermissionRuleSubjectKind::NetworkAccess if draft.network_target.trim().is_empty() => {
                return Err(crate::UiFailure::message(ui_text::t(
                    &self.i18n,
                    "permission-rule-error-network-target-required",
                )));
            }
            _ => {}
        }
        if draft.scope == "session" {
            let trimmed = draft.session_id.trim();
            if trimmed.is_empty() {
                return Err(crate::UiFailure::message(ui_text::t(
                    &self.i18n,
                    "permission-rule-error-session-id-required",
                )));
            }
            trimmed.parse::<i64>().map_err(|_| {
                crate::UiFailure::message(ui_text::t(
                    &self.i18n,
                    "permission-rule-error-session-id-integer",
                ))
            })?;
        }
        let params = permission_rule_params_from_draft(&draft);
        let rule_id = dialog.rule_id;
        self.dispatch_backend_operation(
            move |backend| async move {
                match rule_id {
                    Some(rule_id) => backend.replace_permission_rule(rule_id, params).await,
                    None => backend.create_permission_rule(params).await,
                }
            },
            |app, result| match result {
                Ok(saved) => {
                    app.flash_success(app.i18n.text_args(
                        "flash-permission-rule-saved",
                        &agena_tui::fl_args!(
                            "name" => permission_rule_label(&app.i18n, &saved)
                        ),
                    ));
                    let route = std::mem::replace(&mut app.current_route, Route::Main);
                    match route {
                        Route::PermissionRuleStudio(mut dialog) => {
                            dialog.rule_id = Some(saved.id);
                            dialog.presentation.title = format!(
                                "{} · {}",
                                ui_text::t(&app.i18n, "overlay-permission-rule-workbench-title",),
                                permission_rule_label(&app.i18n, &saved)
                            );
                            dialog.draft = permission_rule_draft_from_resource(&saved);
                            app.refresh_permission_rule_studio(&mut dialog);
                            if let Some(permission) = dialog.return_permission.clone() {
                                app.current_route = app
                                    .route_stack
                                    .pop()
                                    .map(|route| app.refresh_restored_route(route))
                                    .unwrap_or(Route::Main);
                                app.overlay = Some(crate::Overlay::Permission(permission));
                            } else {
                                app.current_route = Route::PermissionRuleStudio(dialog);
                            }
                        }
                        route => app.current_route = route,
                    }
                }
                Err(error) => app.flash_error(error),
            },
        );
        Ok(())
    }

    pub(crate) fn revoke_permission_rule_studio_rule(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
    ) -> bool {
        let Some(rule_id) = dialog.rule_id else {
            return false;
        };
        let name = permission_rule_draft_label(&self.i18n, &dialog.draft);
        self.dispatch_backend_operation(
            move |backend| async move { backend.revoke_permission_rule(rule_id).await },
            move |app, result| match result {
                Ok(_) => {
                    app.flash_success(app.i18n.text_args(
                        "flash-permission-rule-revoked",
                        &agena_tui::fl_args!("name" => name),
                    ));
                    app.current_route = app
                        .route_stack
                        .pop()
                        .map(|route| app.refresh_restored_route(route))
                        .unwrap_or(Route::Main);
                }
                Err(error) => app.flash_error(error),
            },
        );
        false
    }
}
use crate::{
    App, ChoiceOverlayAction, Editor, PermissionRuleStudioAction, PermissionRuleStudioChoiceField,
    PermissionRuleStudioEditField, PermissionRuleStudioEditor, PermissionRuleStudioOverlay,
    PermissionRuleSubjectKind, Route, UiResult, permission_rule_choice_overlay_spec,
    permission_rule_draft_from_resource, permission_rule_draft_label, permission_rule_editor_spec,
    permission_rule_label, permission_rule_params_from_draft, ui_text,
};
