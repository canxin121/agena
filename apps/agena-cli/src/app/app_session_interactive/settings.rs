use super::super::{
    permission_config_from_json_value, permission_override_summary,
    settings_studio_agent_browser_item, settings_studio_field_items, settings_studio_harness_items,
    settings_studio_permission_items, settings_studio_plugin_items, settings_studio_provider_items,
    settings_studio_runtime_items,
};

impl App {
    pub(in crate::app) fn open_file_attach_overlay(&mut self) {
        self.overlay = Some(Overlay::FileAttach(self.build_file_attach_overlay()));
    }

    pub(in crate::app) fn request_file_attachment(&mut self, images_only: bool) {
        // The iTerm2 utility owns its terminal request/response protocol and
        // opens the native file picker on the local Mac, even though Agena is
        // running in the remote SSH session. If it is not installed, retain
        // the existing workspace picker (or native clipboard-image path).
        if iterm2::upload_utility().is_some() {
            self.pending_ui_action = Some(UiAction::AttachIterm2Files { images_only });
        } else if images_only {
            self.pending_ui_action = Some(UiAction::AttachClipboardImage);
        } else {
            self.open_file_attach_overlay();
        }
    }

    pub(in crate::app) fn request_iterm2_download(&mut self, raw_path: &str) {
        let raw_path = raw_path.trim();
        if raw_path.is_empty() {
            self.flash_warning("Usage: /download <workspace-path>".to_string());
            return;
        }
        if iterm2::download_utility().is_none() {
            self.flash_warning(
                "iTerm2 download utility `it2dl` is unavailable; install iTerm2 Shell Integration and Utilities on this remote account."
                    .to_string(),
            );
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
        self.pending_ui_action = Some(UiAction::DownloadIterm2File {
            path: canonical_path,
        });
    }

    pub(in crate::app) fn open_rename_session_overlay(&mut self) {
        let Some(title) = self.current_or_selected_session_title() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.overlay = Some(Overlay::SessionRename(
            self.build_session_rename_overlay(title),
        ));
    }

    pub(in crate::app) fn open_timeline_overlay(&mut self, limit: u64) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.current_route = Route::Timeline(self.build_timeline_overlay(session_id));
        self.request_timeline(session_id, limit);
    }

    pub(in crate::app) fn open_settings_studio(&mut self, query: &str) {
        match self.build_settings_studio_overlay(None, None, SettingsStudioFocus::Navigation) {
            Ok(mut dialog) => {
                self.select_settings_studio_query(&mut dialog, query);
                self.route_stack.clear();
                self.current_route = Route::SettingsStudio(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(in crate::app) fn build_settings_studio_overlay(
        &self,
        preferred_section: Option<SettingsStudioSectionId>,
        preferred_item_label: Option<&str>,
        focus: SettingsStudioFocus,
    ) -> UiResult<SettingsStudioOverlay> {
        let sources = self
            .backend
            .config_json_sources()
            .map_err(|error| error.to_string())?;
        let agents = self.backend.list_agent_descriptors();
        let default_agent = self.backend.default_agent_name();
        let configured_providers = self.backend.list_configured_providers();
        let permission_rule_count = self
            .block_on_async(self.backend.list_permission_rules())
            .map(|rules| rules.len())
            .unwrap_or_default();
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
            .map_err(|error| error.to_string())?;

        let runtime_override_items = settings_studio_runtime_items(&self.i18n, &self.run_options);
        let plugin_items = settings_studio_plugin_items(&self.i18n, &sources);
        let mut agent_items = settings_studio_field_items(
            &self.i18n,
            &sources,
            SettingsStudioSectionId::ConfigAgents,
        );
        agent_items.push(settings_studio_agent_browser_item(
            &self.i18n,
            agents.len(),
            default_agent.as_deref(),
        ));
        let provider_items =
            settings_studio_provider_items(&self.i18n, &sources, &configured_providers);
        let runtime_config_items = settings_studio_field_items(
            &self.i18n,
            &sources,
            SettingsStudioSectionId::ConfigRuntime,
        );
        let session_config_items = settings_studio_field_items(
            &self.i18n,
            &sources,
            SettingsStudioSectionId::ConfigSession,
        );
        let tracing_items = settings_studio_field_items(
            &self.i18n,
            &sources,
            SettingsStudioSectionId::ConfigTracing,
        );
        let ui_items =
            settings_studio_field_items(&self.i18n, &sources, SettingsStudioSectionId::ConfigUi);
        let harness_items = settings_studio_harness_items(&self.i18n, &sources);
        let model_catalog_items = settings_studio_model_catalog_items(&self.i18n, &model_catalog);
        let file_items = settings_studio_file_items(&self.i18n, &sources);
        let permission_items = settings_studio_permission_items(
            &self.i18n,
            &sources,
            &global_permission,
            &workspace_permission,
            &effective_permission,
            current_session_permission.as_ref(),
        );
        let mut runtime_rule_items = Vec::new();
        runtime_rule_items.push(SettingsStudioItem::new(
            ui_text::t(&self.i18n, "overlay-settings-manage-permission-rules"),
            permission_rule_count.to_string(),
            ui_text::t(
                &self.i18n,
                "overlay-settings-manage-permission-rules-detail",
            ),
            SettingsPickerAction::OpenPermissionRules,
        ));
        let agent_count = agents.len();
        let mut sections = vec![
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigProviders,
                label: ui_text::t(&self.i18n, "overlay-settings-section-providers-label"),
                summary: self.i18n.text_args(
                    "overlay-settings-section-providers-summary",
                    &crate::fl_args!("count" => configured_providers.len() as i64),
                ),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-providers-description",
                ),
                items: provider_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigAgents,
                label: ui_text::t(&self.i18n, "overlay-settings-section-agents-label"),
                summary: match default_agent.as_deref() {
                    Some(default) => self.i18n.text_args(
                        "overlay-settings-section-agents-summary-default",
                        &crate::fl_args!(
                            "count" => agent_count as i64,
                            "default" => default.to_string(),
                        ),
                    ),
                    None => self.i18n.text_args(
                        "overlay-settings-section-agents-summary",
                        &crate::fl_args!(
                            "count" => agent_count as i64,
                        ),
                    ),
                },
                description: ui_text::t(&self.i18n, "overlay-settings-section-agents-description"),
                items: agent_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigPermission,
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
                id: SettingsStudioSectionId::ConfigPlugins,
                label: ui_text::t(&self.i18n, "overlay-settings-section-plugins-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-plugins-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-plugins-description"),
                items: plugin_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigRuntime,
                label: ui_text::t(&self.i18n, "overlay-settings-section-runtime-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-runtime-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-runtime-description"),
                items: runtime_config_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigSession,
                label: ui_text::t(&self.i18n, "overlay-settings-section-session-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-session-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-session-description"),
                items: session_config_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigHarnesses,
                label: ui_text::t(&self.i18n, "overlay-settings-section-harnesses-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-harnesses-summary"),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-harnesses-description",
                ),
                items: harness_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigTracing,
                label: ui_text::t(&self.i18n, "overlay-settings-section-tracing-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-tracing-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-tracing-description"),
                items: tracing_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigUi,
                label: ui_text::t(&self.i18n, "overlay-settings-section-ui-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-ui-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-ui-description"),
                items: ui_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::RuntimeOverrides,
                label: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-runtime-overrides-label",
                ),
                summary: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-runtime-overrides-summary",
                ),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-runtime-overrides-description",
                ),
                items: runtime_override_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::RuntimeRules,
                label: ui_text::t(&self.i18n, "overlay-settings-section-runtime-rules-label"),
                summary: self.i18n.text_args(
                    "overlay-settings-section-runtime-rules-summary",
                    &crate::fl_args!("count" => permission_rule_count as i64),
                ),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-runtime-rules-description",
                ),
                items: runtime_rule_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Catalogs,
                label: ui_text::t(&self.i18n, "overlay-settings-section-model-catalog-label"),
                summary: self.i18n.text_args(
                    "overlay-settings-section-model-catalog-summary",
                    &crate::fl_args!("count" => model_catalog.summary.model_count as i64),
                ),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-model-catalog-description",
                ),
                items: model_catalog_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Files,
                label: ui_text::t(&self.i18n, "overlay-settings-section-files-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-files-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-files-description"),
                items: file_items,
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
            state: SectionedListState::new(
                std::mem::take(&mut sections),
                selected_section,
                selected_item,
                focus,
            ),
        })
    }

    pub(in crate::app) fn refresh_settings_studio_overlay(
        &mut self,
        dialog: &mut SettingsStudioOverlay,
    ) {
        let preferred_section = dialog.state.selected_section().map(|section| section.id);
        let preferred_item = dialog.state.selected_item().map(|item| item.label.as_str());
        match self.build_settings_studio_overlay(
            preferred_section,
            preferred_item,
            dialog.state.focus(),
        ) {
            Ok(updated) => *dialog = updated,
            Err(error) => self.flash_error(error),
        }
    }
}
use crate::app::{
    App, JsonValue, Overlay, Path, Route, SectionedListState, SettingsPickerAction,
    SettingsStudioFocus, SettingsStudioItem, SettingsStudioOverlay, SettingsStudioSection,
    SettingsStudioSectionId, UiAction, UiResult, fs, get_json_path, iterm2, min,
    settings_studio_file_items, settings_studio_model_catalog_items, ui_text,
};
