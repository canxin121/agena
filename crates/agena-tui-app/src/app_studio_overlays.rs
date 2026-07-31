use super::{
    permission_config_from_json_value, permission_studio_read_only_message,
    refresh_permission_studio_dialog, set_permission_studio_pane_focus,
};

impl App {
    pub(crate) fn open_global_permission_studio(&mut self) {
        match self.build_permission_studio_overlay(
            PermissionStudioSource::GlobalConfig,
            PermissionStudioPage::Overview,
            Some(PermissionStudioSectionId::RootPath),
            None,
            PermissionStudioFocus::Navigation,
        ) {
            Ok(dialog) => self.current_route = Route::PermissionStudio(dialog),
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn open_workspace_permission_studio(&mut self) {
        match self.build_permission_studio_overlay(
            PermissionStudioSource::WorkspaceConfig,
            PermissionStudioPage::Overview,
            Some(PermissionStudioSectionId::RootPath),
            None,
            PermissionStudioFocus::Navigation,
        ) {
            Ok(dialog) => self.current_route = Route::PermissionStudio(dialog),
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn open_session_permission_studio(&mut self, session_id: i64) {
        match self.build_permission_studio_overlay(
            PermissionStudioSource::Session { session_id },
            PermissionStudioPage::Overview,
            Some(PermissionStudioSectionId::RootPath),
            None,
            PermissionStudioFocus::Navigation,
        ) {
            Ok(dialog) => self.current_route = Route::PermissionStudio(dialog),
            Err(error) => self.flash_error(error),
        }
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
                let state = self
                    .block_on_async(
                        self.backend
                            .get_session_permission_studio_state(*session_id),
                    )
                    .map_err(crate::UiFailure::internal)?;
                (
                    state.session_title,
                    session_id.to_string(),
                    ui_text::t(&self.i18n, "permission-studio-source-session"),
                    true,
                    state.permission,
                )
            }
            PermissionStudioSource::EffectiveSession { session_id } => {
                let state = self
                    .block_on_async(
                        self.backend
                            .get_session_permission_studio_state(*session_id),
                    )
                    .map_err(crate::UiFailure::internal)?;
                (
                    ui_text::t(&self.i18n, "settings-permission-effective-label"),
                    state.session_title,
                    ui_text::t(&self.i18n, "permission-studio-source-effective"),
                    false,
                    state.effective_permission,
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
        match &dialog.source {
            PermissionStudioSource::GlobalConfig => {
                if permission.is_empty() {
                    self.block_on_async(self.backend.delete_config_setting("permission"))
                        .map_err(crate::UiFailure::internal)?;
                    self.flash_success(settings_path_cleared_message(&self.i18n, "permission"));
                } else {
                    self.block_on_async(self.backend.set_config_setting(
                        "permission",
                        serde_json::to_value(&permission).map_err(crate::UiFailure::internal)?,
                    ))
                    .map_err(crate::UiFailure::internal)?;
                    self.flash_success(settings_path_updated_message(&self.i18n, "permission"));
                }
                self.refresh_current_transcript_execution_state();
            }
            PermissionStudioSource::WorkspaceConfig => {
                if permission.is_empty() {
                    self.block_on_async(self.backend.delete_workspace_config_setting("permission"))
                        .map_err(crate::UiFailure::internal)?;
                    self.flash_success(settings_path_cleared_message(&self.i18n, "permission"));
                } else {
                    self.block_on_async(self.backend.set_workspace_config_setting(
                        "permission",
                        serde_json::to_value(&permission).map_err(crate::UiFailure::internal)?,
                    ))
                    .map_err(crate::UiFailure::internal)?;
                    self.flash_success(settings_path_updated_message(&self.i18n, "permission"));
                }
                self.refresh_current_transcript_execution_state();
            }
            PermissionStudioSource::Session { session_id } => {
                let execution = self
                    .block_on_async(self.backend.set_session_permission(*session_id, permission))
                    .map_err(crate::UiFailure::internal)?;
                if self.transcript.session_id == Some(*session_id) {
                    let _ = self.apply_transcript_execution(execution);
                }
                self.flash_success(ui_text::t(&self.i18n, "flash-session-permission-updated"));
            }
            PermissionStudioSource::EffectiveSession { .. } => {
                return Err(crate::UiFailure::message(
                    permission_studio_read_only_message(&self.i18n, &dialog.source),
                ));
            }
        }
        self.refresh_permission_studio_overlay(dialog);
        Ok(())
    }
}
use crate::{
    App, JsonValue, PermissionConfig, PermissionStudioFocus, PermissionStudioOverlay,
    PermissionStudioPage, PermissionStudioPaneFocus, PermissionStudioSectionId,
    PermissionStudioSource, Route, SectionedListState, SelectableListState, UiResult,
    get_json_path, settings_path_cleared_message, settings_path_updated_message, ui_text,
};
