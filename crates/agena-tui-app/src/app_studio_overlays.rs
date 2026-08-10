use super::{
    permission_config_from_json_value, permission_studio_read_only_message,
    refresh_permission_studio_dialog, set_permission_studio_pane_focus,
};

impl App {
    pub(crate) fn open_global_permission_studio(&mut self) {
        match self.build_permission_studio_overlay(
            PermissionStudioSource::GlobalConfig,
            PermissionStudioPage::PathDefaults,
            Some(PermissionStudioSectionId::PathDefaults),
            None,
            PermissionStudioFocus::Items,
        ) {
            Ok(dialog) => self.current_route = Route::PermissionStudio(dialog),
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn open_workspace_permission_studio(&mut self) {
        match self.build_permission_studio_overlay(
            PermissionStudioSource::WorkspaceConfig,
            PermissionStudioPage::PathDefaults,
            Some(PermissionStudioSectionId::PathDefaults),
            None,
            PermissionStudioFocus::Items,
        ) {
            Ok(dialog) => self.current_route = Route::PermissionStudio(dialog),
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn open_session_permission_studio(&mut self, session_id: i64) {
        self.open_loaded_session_permission_studio(PermissionStudioSource::Session { session_id });
    }

    pub(crate) fn open_effective_session_permission_studio(&mut self, session_id: i64) {
        self.open_loaded_session_permission_studio(PermissionStudioSource::EffectiveSession {
            session_id,
        });
    }

    fn open_loaded_session_permission_studio(&mut self, source: PermissionStudioSource) {
        let session_id = match &source {
            PermissionStudioSource::Session { session_id }
            | PermissionStudioSource::EffectiveSession { session_id } => *session_id,
            _ => return,
        };
        self.dispatch_backend_operation(
            move |backend| async move {
                backend
                    .get_session_permission_studio_state(session_id)
                    .await
            },
            move |app, result| match result {
                Ok(state) => {
                    app.settings_session_permission = Some((session_id, state));
                    match app.build_permission_studio_overlay(
                        source,
                        PermissionStudioPage::PathDefaults,
                        Some(PermissionStudioSectionId::PathDefaults),
                        None,
                        PermissionStudioFocus::Items,
                    ) {
                        Ok(dialog) => app.current_route = Route::PermissionStudio(dialog),
                        Err(error) => app.flash_error(error),
                    }
                }
                Err(error) => app.flash_error(error),
            },
        );
    }

    fn cached_session_permission_studio_state(
        &self,
        session_id: i64,
    ) -> UiResult<&agena_tui_backend::SessionPermissionStudioState> {
        self.settings_session_permission
            .as_ref()
            .filter(|(cached_session_id, _)| *cached_session_id == session_id)
            .map(|(_, state)| state)
            .ok_or_else(|| crate::UiFailure::internal("session permission state is still loading"))
    }

    pub(crate) fn build_permission_studio_overlay(
        &self,
        source: PermissionStudioSource,
        page: PermissionStudioPage,
        preferred_section: Option<PermissionStudioSectionId>,
        preferred_item_label: Option<&str>,
        preferred_focus: PermissionStudioFocus,
    ) -> UiResult<PermissionStudioOverlay> {
        let (title_context, source_label, scope_label, editable, permission) = match &source {
            PermissionStudioSource::GlobalConfig => {
                let sources = self
                    .backend
                    .config_json_sources()
                    .map_err(crate::UiFailure::internal)?;
                let permission = permission_config_from_json_value(
                    &get_json_path(&sources.file, Some("permission")).unwrap_or(JsonValue::Null),
                )?;
                (
                    ui_text::t(&self.i18n, "settings-permission-global-label"),
                    sources.config_path.display().to_string(),
                    ui_text::t(&self.i18n, "permission-studio-source-global"),
                    true,
                    permission.clone(),
                )
            }
            PermissionStudioSource::WorkspaceConfig => {
                let sources = self
                    .backend
                    .config_json_sources()
                    .map_err(crate::UiFailure::internal)?;
                let permission = permission_config_from_json_value(
                    &get_json_path(&sources.project_file, Some("permission"))
                        .unwrap_or(JsonValue::Null),
                )?;
                (
                    ui_text::t(&self.i18n, "settings-permission-workspace-label"),
                    sources.project_config_path.display().to_string(),
                    ui_text::t(&self.i18n, "permission-studio-source-workspace"),
                    true,
                    permission,
                )
            }
            PermissionStudioSource::Session { session_id } => {
                let state = self.cached_session_permission_studio_state(*session_id)?;
                (
                    state.session_title.clone(),
                    session_id.to_string(),
                    ui_text::t(&self.i18n, "permission-studio-source-session"),
                    true,
                    state.permission.clone(),
                )
            }
            PermissionStudioSource::EffectiveSession { session_id } => {
                let state = self.cached_session_permission_studio_state(*session_id)?;
                (
                    ui_text::t(&self.i18n, "settings-permission-effective-label"),
                    state.session_title.clone(),
                    ui_text::t(&self.i18n, "permission-studio-source-effective"),
                    false,
                    state.effective_permission.clone(),
                )
            }
        };
        let mut dialog = PermissionStudioOverlay {
            title: String::new(),
            footer: String::new(),
            source,
            title_context,
            source_label,
            scope_label,
            editable,
            permission,
            nav: SelectableListState::new(Vec::new(), 0),
            pane_focus: PermissionStudioPaneFocus::Navigation,
            page,
            state: SectionedListState::new(Vec::new(), 0, 0, PermissionStudioFocus::Navigation),
            editor: None,
        };
        refresh_permission_studio_dialog(
            &self.i18n,
            &mut dialog,
            preferred_section,
            preferred_item_label,
            Some(preferred_focus),
        );
        Ok(dialog)
    }

    pub(crate) fn refresh_permission_studio_overlay(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
    ) {
        let preferred_section = dialog.state.selected_section().map(|section| section.id);
        let preferred_item = dialog.state.selected_item().map(|item| item.label.as_str());
        let pane_focus = dialog.pane_focus;
        match self.build_permission_studio_overlay(
            dialog.source.clone(),
            dialog.page.clone(),
            preferred_section,
            preferred_item,
            dialog.state.focus(),
        ) {
            Ok(mut updated) => {
                set_permission_studio_pane_focus(&mut updated, pane_focus);
                *dialog = updated;
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn set_permission_studio_page_with_section(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        page: PermissionStudioPage,
        section: Option<PermissionStudioSectionId>,
        focus: PermissionStudioFocus,
    ) {
        dialog.page = page;
        refresh_permission_studio_dialog(&self.i18n, dialog, section, None, Some(focus));
    }

    pub(crate) fn persist_permission_studio(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        permission: PermissionConfig,
    ) -> UiResult<()> {
        if matches!(
            dialog.source,
            PermissionStudioSource::EffectiveSession { .. }
        ) {
            return Err(crate::UiFailure::message(
                permission_studio_read_only_message(&self.i18n, &dialog.source),
            ));
        }
        let source = dialog.source.clone();
        let completion_source = source.clone();
        let permission_value = (!permission.is_empty())
            .then(|| serde_json::to_value(&permission).map_err(crate::UiFailure::internal))
            .transpose()?;
        let cleared = permission_value.is_none();
        self.dispatch_backend_operation(
            move |backend| async move {
                match source {
                    PermissionStudioSource::GlobalConfig => match permission_value {
                        Some(value) => {
                            backend.set_config_setting("permission", value).await?;
                        }
                        None => {
                            backend.delete_config_setting("permission").await?;
                        }
                    },
                    PermissionStudioSource::WorkspaceConfig => match permission_value {
                        Some(value) => {
                            backend
                                .set_workspace_config_setting("permission", value)
                                .await?;
                        }
                        None => {
                            backend
                                .delete_workspace_config_setting("permission")
                                .await?;
                        }
                    },
                    PermissionStudioSource::Session { session_id } => {
                        let execution = backend
                            .set_session_permission(session_id, permission)
                            .await?;
                        return Ok::<_, anyhow::Error>(Some(execution));
                    }
                    PermissionStudioSource::EffectiveSession { .. } => unreachable!(),
                }
                Ok::<_, anyhow::Error>(None)
            },
            move |app, result| match result {
                Ok(execution) => match completion_source {
                    PermissionStudioSource::GlobalConfig
                    | PermissionStudioSource::WorkspaceConfig => {
                        app.flash_success(if cleared {
                            settings_path_cleared_message(&app.i18n, "permission")
                        } else {
                            settings_path_updated_message(&app.i18n, "permission")
                        });
                        app.refresh_current_transcript_execution_state();
                        app.rebuild_current_permission_studio_route();
                    }
                    PermissionStudioSource::Session { session_id } => {
                        if let Some(execution) = execution
                            && app.transcript.session_id == Some(session_id)
                        {
                            let _ = app.apply_transcript_execution(execution);
                        }
                        app.flash_success(ui_text::t(
                            &app.i18n,
                            "flash-session-permission-updated",
                        ));
                        app.refresh_session_permission_studio_state(session_id);
                    }
                    PermissionStudioSource::EffectiveSession { .. } => {}
                },
                Err(error) => app.flash_error(error),
            },
        );
        Ok(())
    }

    fn refresh_session_permission_studio_state(&mut self, session_id: i64) {
        self.dispatch_backend_operation(
            move |backend| async move {
                backend
                    .get_session_permission_studio_state(session_id)
                    .await
            },
            move |app, result| match result {
                Ok(state) => {
                    app.settings_session_permission = Some((session_id, state));
                    app.rebuild_current_permission_studio_route();
                }
                Err(error) => app.flash_error(error),
            },
        );
    }

    fn rebuild_current_permission_studio_route(&mut self) {
        let route = std::mem::replace(&mut self.current_route, Route::Main);
        self.current_route = match route {
            Route::PermissionStudio(mut dialog) => {
                self.refresh_permission_studio_overlay(&mut dialog);
                Route::PermissionStudio(dialog)
            }
            route => route,
        };
    }
}
use crate::{
    App, JsonValue, PermissionConfig, PermissionStudioFocus, PermissionStudioOverlay,
    PermissionStudioPage, PermissionStudioPaneFocus, PermissionStudioSectionId,
    PermissionStudioSource, Route, SectionedListState, SelectableListState, UiResult,
    get_json_path, settings_path_cleared_message, settings_path_updated_message, ui_text,
};
