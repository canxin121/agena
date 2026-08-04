use super::super::{
    permission_config_from_json_value, permission_override_summary, settings_studio_field_items,
    settings_studio_harness_items, settings_studio_permission_items, settings_studio_plugin_items,
    settings_studio_provider_approval_model_item, settings_studio_provider_items,
};

impl App {
    pub(crate) fn request_file_attachment(&mut self, images_only: bool) {
        // Keep file selection entirely inside the TUI. The attachment target
        // is a directory browser rather than a workspace-wide search: it
        // opens at the workspace root and refreshes as its path is edited.
        let title_key = if images_only {
            "overlay-attach-image-title"
        } else {
            "overlay-attach-title"
        };
        let prompt_key = if images_only {
            "overlay-attach-image-browser-prompt"
        } else {
            "overlay-attach-browser-prompt"
        };
        let empty_key = if images_only {
            "overlay-attach-image-no-match"
        } else {
            "overlay-attach-no-match"
        };
        self.overlay = Some(Overlay::PathBrowser(self.build_path_browser_overlay(
            ui_text::t(&self.i18n, title_key),
            ui_text::t(&self.i18n, prompt_key),
            ui_text::t(&self.i18n, "overlay-attach-browser-footer"),
            ui_text::t(&self.i18n, empty_key),
            PathBrowserMode::AnyPath,
            self.backend.workspace_root().display().to_string(),
            PathBrowserTarget::FileAttachment { images_only },
        )));
    }

    pub(crate) fn request_terminal_download(&mut self, raw_path: &str) {
        let raw_path = raw_path.trim();
        if raw_path.is_empty() {
            self.flash_warning("Usage: /download <workspace-path>".to_string());
            return;
        }
        let requested = Path::new(raw_path);
        if requested.is_absolute() {
            self.flash_warning("Use a path relative to the current workspace.".to_string());
            return;
        }

        let workspace = match fs::canonicalize(self.backend.workspace_root()) {
            Ok(workspace) => workspace,
            Err(error) => {
                self.flash_error(format!("Could not access the current workspace: {error}"));
                return;
            }
        };
        let path = self.backend.resolve_workspace_path(requested);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.flash_warning(format!(
                    "Could not access download path {}: {error}",
                    path.display()
                ));
                return;
            }
        };
        if metadata.file_type().is_symlink() {
            self.flash_warning("Symbolic links cannot be downloaded.".to_string());
            return;
        }
        if !metadata.is_file() {
            self.flash_warning("Only regular workspace files can be downloaded.".to_string());
            return;
        }

        let canonical_path = match fs::canonicalize(&path) {
            Ok(path) => path,
            Err(error) => {
                self.flash_warning(format!(
                    "Could not resolve download path {}: {error}",
                    path.display()
                ));
                return;
            }
        };
        if !canonical_path.starts_with(workspace) {
            self.flash_warning(
                "Only files inside the current workspace can be downloaded.".to_string(),
            );
            return;
        }
        self.pending_ui_action = Some(UiAction::DownloadTerminalFile {
            path: canonical_path,
        });
    }

    pub(crate) fn open_rename_session_overlay(&mut self) {
        let Some(title) = self.current_or_selected_session_title() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.overlay = Some(Overlay::SessionRename(
            self.build_session_rename_overlay(title),
        ));
    }

    pub(crate) fn open_timeline_overlay(&mut self, limit: u64) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.current_route = Route::Timeline(self.build_timeline_overlay(session_id));
        self.request_timeline(session_id, limit);
    }

    pub(crate) fn open_settings_studio(&mut self) {
        match self.build_settings_studio_overlay(None, None, SettingsStudioFocus::Navigation) {
            Ok(dialog) => {
                self.route_stack.clear();
                self.current_route = Route::SettingsStudio(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn build_settings_studio_overlay(
        &self,
        preferred_section: Option<SettingsStudioSectionId>,
        preferred_item_label: Option<&str>,
        focus: SettingsStudioFocus,
    ) -> UiResult<SettingsStudioOverlay> {
        let sources = self
            .backend
            .config_json_sources()
            .map_err(crate::UiFailure::internal)?;
        let configured_providers = self.backend.list_configured_providers();
        let global_permission = permission_config_from_json_value(
            &get_json_path(&sources.file, Some("permission")).unwrap_or(JsonValue::Null),
        )?;
        let workspace_permission = permission_config_from_json_value(
            &get_json_path(&sources.project_file, Some("permission")).unwrap_or(JsonValue::Null),
        )?;
        let effective_permission = permission_config_from_json_value(
            &get_json_path(&sources.effective, Some("permission")).unwrap_or(JsonValue::Null),
        )?;
        let current_session_permission =
            self.current_or_selected_session_id()
                .and_then(|session_id| {
                    self.block_on_async(
                        self.backend.get_session_permission_studio_state(session_id),
                    )
                    .ok()
                });
        let model_catalog = self
            .backend
            .list_model_catalog_models("", 0, 1)
            .map_err(crate::UiFailure::internal)?;

        let mut plugin_items = settings_studio_plugin_items(&self.i18n, &sources);
        plugin_items.extend(settings_studio_harness_items(&self.i18n, &sources));
        let mut provider_items =
            settings_studio_provider_items(&self.i18n, &sources, &configured_providers);
        provider_items.push(settings_studio_provider_approval_model_item(
            &self.i18n,
            &sources,
            &global_permission,
            &effective_permission,
        ));
        provider_items.extend(settings_studio_model_catalog_items(
            &self.i18n,
            &model_catalog,
        ));
        let mut diagnostic_items =
            settings_studio_field_items(&self.i18n, &sources, SettingsStudioSectionId::Diagnostics);
        diagnostic_items.extend(settings_studio_file_items(&self.i18n, &sources));
        diagnostic_items.push(SettingsStudioItem::new(
            ui_text::t(&self.i18n, "terminal-diagnostics-title"),
            self.block_on_async(self.backend.runtime_snapshot_summary())
                .map_err(crate::UiFailure::internal)?,
            ui_text::t(&self.i18n, "command-diagnostics-summary"),
            SettingsPickerAction::OpenTerminalDiagnostics,
        ));
        let ui_items =
            settings_studio_field_items(&self.i18n, &sources, SettingsStudioSectionId::Interface);
        let mut runtime_session_items = vec![SettingsStudioItem::new(
            ui_text::t(&self.i18n, "settings-client-versions-entry-label"),
            ui_text::t(&self.i18n, "settings-client-versions-entry-value"),
            ui_text::t(&self.i18n, "settings-client-versions-entry-detail"),
            SettingsPickerAction::OpenProviderClientVersions,
        )];
        runtime_session_items.extend(settings_studio_field_items(
            &self.i18n,
            &sources,
            SettingsStudioSectionId::RuntimeSession,
        ));
        let permission_items = settings_studio_permission_items(
            &self.i18n,
            &sources,
            &global_permission,
            &workspace_permission,
            &effective_permission,
            current_session_permission.as_ref(),
        );
        let mut sections = vec![
            SettingsStudioSection {
                id: SettingsStudioSectionId::ModelsProviders,
                label: ui_text::t(&self.i18n, "overlay-settings-section-providers-label"),
                summary: self.i18n.text_args(
                    "overlay-settings-section-providers-summary",
                    &agena_tui::fl_args!("count" => configured_providers.len() as i64),
                ),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-providers-description",
                ),
                items: provider_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Permissions,
                label: ui_text::t(&self.i18n, "overlay-settings-section-permissions-label"),
                summary: current_session_permission
                    .as_ref()
                    .map(|state| {
                        permission_override_summary(&self.i18n, &state.effective_permission)
                    })
                    .unwrap_or_else(|| permission_override_summary(&self.i18n, &global_permission)),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-permissions-description",
                ),
                items: permission_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::PluginsTools,
                label: ui_text::t(&self.i18n, "overlay-settings-section-plugins-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-plugins-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-plugins-description"),
                items: plugin_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::RuntimeSession,
                label: ui_text::t(&self.i18n, "overlay-settings-section-runtime-session-label"),
                summary: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-runtime-session-summary",
                ),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-runtime-session-description",
                ),
                items: runtime_session_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Interface,
                label: ui_text::t(&self.i18n, "overlay-settings-section-ui-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-ui-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-ui-description"),
                items: ui_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Diagnostics,
                label: ui_text::t(&self.i18n, "terminal-diagnostics-title"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-tracing-summary"),
                description: ui_text::t(&self.i18n, "command-diagnostics-summary"),
                items: diagnostic_items,
            },
        ];

        let selected_section = preferred_section
            .and_then(|target| sections.iter().position(|section| section.id == target))
            .unwrap_or(0);
        let mut selected_item = preferred_item_label
            .and_then(|label| {
                sections
                    .get(selected_section)
                    .and_then(|section| section.items.iter().position(|item| item.label == label))
            })
            .unwrap_or(0);
        if sections
            .get(selected_section)
            .is_none_or(|section| section.items.is_empty())
        {
            selected_item = 0;
        } else {
            selected_item = min(
                selected_item,
                sections[selected_section].items.len().saturating_sub(1),
            );
        }

        Ok(SettingsStudioOverlay {
            title: ui_text::t(&self.i18n, "overlay-settings-title"),
            footer: ui_text::t(&self.i18n, "overlay-settings-footer"),
            state: SettingsStudioPresentation::new(
                std::mem::take(&mut sections),
                selected_section,
                selected_item,
                focus,
            ),
        })
    }

    pub(crate) fn rebuild_settings_studio_overlay(
        &self,
        dialog: &SettingsStudioOverlay,
    ) -> UiResult<SettingsStudioOverlay> {
        self.build_settings_studio_overlay(
            dialog.state.selected_section().map(|section| section.id),
            dialog.state.selected_item().map(|item| item.label.as_str()),
            dialog.state.focus(),
        )
    }

    pub(crate) fn refresh_settings_studio_overlay(&mut self, dialog: &mut SettingsStudioOverlay) {
        match self.rebuild_settings_studio_overlay(dialog) {
            Ok(updated) => *dialog = updated,
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn build_client_versions_studio_overlay(&self) -> UiResult<SettingsStudioOverlay> {
        let sources = self
            .backend
            .config_json_sources()
            .map_err(crate::UiFailure::internal)?;
        let items = settings_studio_field_items(
            &self.i18n,
            &sources,
            SettingsStudioSectionId::ProviderClientVersions,
        );
        let sections = vec![SettingsStudioSection {
            id: SettingsStudioSectionId::ProviderClientVersions,
            label: ui_text::t(&self.i18n, "settings-client-versions-section-label"),
            summary: ui_text::t(&self.i18n, "settings-client-versions-section-summary"),
            description: ui_text::t(&self.i18n, "settings-client-versions-section-description"),
            items,
        }];
        Ok(SettingsStudioOverlay {
            title: ui_text::t(&self.i18n, "overlay-client-versions-title"),
            footer: ui_text::t(&self.i18n, "overlay-settings-footer"),
            state: SettingsStudioPresentation::new(sections, 0, 0, SettingsStudioFocus::Items),
        })
    }

    pub(crate) fn rebuild_client_versions_studio_overlay(
        &self,
        dialog: &SettingsStudioOverlay,
    ) -> UiResult<SettingsStudioOverlay> {
        let mut rebuilt = self.build_client_versions_studio_overlay()?;
        rebuilt.state.set_indices(
            dialog.state.selected_section_index(),
            dialog.state.selected_item_index(),
        );
        rebuilt.state.set_focus(dialog.state.focus());
        Ok(rebuilt)
    }

    pub(crate) fn open_client_versions_studio(&mut self) {
        match self.build_client_versions_studio_overlay() {
            Ok(dialog) => self.current_route = Route::ClientVersionsStudio(dialog),
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn refresh_client_versions_studio_overlay(
        &mut self,
        dialog: &mut SettingsStudioOverlay,
    ) {
        match self.rebuild_client_versions_studio_overlay(dialog) {
            Ok(updated) => *dialog = updated,
            Err(error) => self.flash_error(error),
        }
    }
}
use crate::{
    App, JsonValue, Overlay, Path, PathBrowserMode, PathBrowserTarget, Route, SettingsPickerAction,
    SettingsStudioFocus, SettingsStudioItem, SettingsStudioOverlay, SettingsStudioPresentation,
    SettingsStudioSection, SettingsStudioSectionId, UiAction, UiResult, fs, get_json_path, min,
    settings_studio_file_items, settings_studio_model_catalog_items, ui_text,
};
