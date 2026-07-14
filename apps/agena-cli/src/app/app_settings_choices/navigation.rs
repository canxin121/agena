impl App {
    pub(in crate::app) fn open_runtime_config_in_editor(&mut self) {
        let path = self.backend.config_path();
        if !path.exists() {
            if let Some(parent) = path.parent()
                && let Err(error) = fs::create_dir_all(parent)
            {
                self.flash_error(self.i18n.text_args(
                    "flash-config-dir-prepare-failed",
                    &crate::fl_args!(
                        "path" => parent.display().to_string(),
                        "error" => error.to_string(),
                    ),
                ));
                return;
            }
            if let Err(error) = fs::write(&path, "") {
                self.flash_error(self.i18n.text_args(
                    "flash-config-file-create-failed",
                    &crate::fl_args!(
                        "path" => path.display().to_string(),
                        "error" => error.to_string(),
                    ),
                ));
                return;
            }
        }
        self.pending_ui_action = Some(UiAction::OpenPath { path });
    }

    pub(in crate::app) fn open_agent_profile_source(&mut self, profile: &AgentProfile) {
        match self.agent_profile_storage(profile) {
            AgentProfileStorage::Markdown => {
                if let Some(path) = profile.source_path.clone() {
                    self.pending_ui_action = Some(UiAction::OpenPath { path });
                }
            }
            AgentProfileStorage::Config => self.open_runtime_config_in_editor(),
            AgentProfileStorage::BuiltIn => {
                self.flash_info(ui_text::t(&self.i18n, "flash-agent-built-in-no-source"));
            }
            AgentProfileStorage::Runtime => {
                self.flash_info(ui_text::t(&self.i18n, "flash-agent-runtime-no-source"));
            }
        }
    }

    pub(in crate::app) fn open_inspector_picker(
        &mut self,
        title: String,
        prompt: String,
        query: &str,
        rows: Vec<crate::backend::InspectorRow>,
    ) {
        let overlay = self.build_picker_overlay(
            title,
            prompt,
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            Editor::from_text(query.to_string()),
            rows.into_iter()
                .map(|row| PickerItem {
                    label: row.label,
                    detail: row.detail,
                    value: PickerValue::Inspector,
                })
                .collect(),
            PickerKind::Inspector,
            false,
        );
        self.current_route = Route::Picker(overlay);
    }

    pub(in crate::app) fn open_permission_rule_picker(&mut self, query: &str) {
        match self.build_permission_rule_picker_overlay(query) {
            Ok(overlay) => self.current_route = Route::Picker(overlay),
            Err(error) => self.flash_error(error),
        }
    }

    pub(in crate::app) fn build_permission_rule_picker_overlay(
        &self,
        query: &str,
    ) -> UiResult<PickerOverlay> {
        let rules = self
            .block_on_async(self.backend.list_permission_rules())
            .map_err(|error| error.to_string())?;
        let mut all_items = vec![PickerItem {
            label: ui_text::t(&self.i18n, "permission-rule-create-label"),
            detail: ui_text::t(&self.i18n, "permission-rule-create-detail"),
            value: PickerValue::PermissionRuleCreate,
        }];
        all_items.extend(rules.into_iter().map(|rule| PickerItem {
            label: permission_rule_label(&self.i18n, &rule),
            detail: permission_rule_detail(&self.i18n, &rule),
            value: PickerValue::PermissionRule(Box::new(rule)),
        }));
        let overlay = self.build_picker_overlay(
            ui_text::t(&self.i18n, "overlay-permission-rules-title"),
            ui_text::t(&self.i18n, "overlay-permission-rules-prompt"),
            ui_text::t(&self.i18n, "overlay-permission-rules-footer"),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            Editor::from_text(query.to_string()),
            all_items,
            PickerKind::PermissionRules,
            false,
        );
        Ok(overlay)
    }

    pub(in crate::app) fn build_permission_rule_studio_overlay(
        &self,
        rule_id: Option<i64>,
        title: String,
        draft: PermissionRuleDraft,
        preferred_item_label: Option<&str>,
    ) -> PermissionRuleStudioOverlay {
        let items = permission_rule_studio_items(&self.i18n, &draft, rule_id);
        let selected = preferred_item_label
            .and_then(|label| items.iter().position(|item| item.label == label))
            .unwrap_or(0);
        let footer = ui_text::t(&self.i18n, "overlay-permission-rule-studio-footer");
        PermissionRuleStudioOverlay {
            rule_id,
            draft,
            return_to_permission: false,
            workbench: ListWorkbenchState::new(
                title,
                footer,
                SelectableListState::new(items, selected),
            ),
        }
    }

    pub(in crate::app) fn open_permission_rule_studio(
        &mut self,
        rule: Option<&PermissionRuleResource>,
        draft_override: Option<PermissionRuleDraft>,
    ) {
        let (rule_id, title, draft) = match (rule, draft_override) {
            (_, Some(draft)) => (
                rule.map(|rule| rule.id),
                ui_text::t(&self.i18n, "overlay-permission-rule-workbench-title"),
                draft,
            ),
            (Some(rule), None) => (
                Some(rule.id),
                format!(
                    "{} · {}",
                    ui_text::t(&self.i18n, "overlay-permission-rule-workbench-title"),
                    permission_rule_label(&self.i18n, rule)
                ),
                permission_rule_draft_from_resource(rule),
            ),
            (None, None) => (
                None,
                ui_text::t(&self.i18n, "overlay-permission-rule-workbench-title"),
                PermissionRuleDraft::default(),
            ),
        };
        self.current_route = Route::PermissionRuleStudio(
            self.build_permission_rule_studio_overlay(rule_id, title, draft, None),
        );
    }

    pub(in crate::app) fn refresh_permission_rule_studio(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
    ) {
        refresh_permission_rule_studio_dialog(&self.i18n, dialog);
    }

    pub(in crate::app) fn open_snapshot_remove_confirm(
        &mut self,
        session_id: i64,
        discard_changes: bool,
    ) {
        let mut body_lines = vec![ui_text::t(&self.i18n, "overlay-snapshot-remove-body")];
        if discard_changes {
            body_lines.push(ui_text::t(&self.i18n, "overlay-snapshot-remove-force"));
        }
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-snapshot-remove-title"),
            body_lines,
            ConfirmAction::ExitSnapshot {
                session_id,
                discard_changes,
            },
        )));
    }

    pub(in crate::app) fn open_command_palette(&mut self) {
        let mut all_items = commands::COMMANDS
            .iter()
            .map(|spec| PickerItem {
                label: spec.palette_invocation(),
                detail: ui_text::t(&self.i18n, spec.summary_key),
                value: PickerValue::Command(spec),
            })
            .collect::<Vec<_>>();
        all_items.extend(
            self.plugin_slash_commands()
                .into_iter()
                .filter_map(|entry| {
                    let name = plugin_command_slash_name(&entry)?;
                    Some(PickerItem {
                        label: format!("/{name}"),
                        detail: plugin_command_detail(&entry),
                        value: PickerValue::PluginCommand(Box::new(entry)),
                    })
                }),
        );
        let overlay = self.build_picker_overlay(
            ui_text::t(&self.i18n, "overlay-commands-title"),
            ui_text::t(&self.i18n, "overlay-commands-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            Editor::default(),
            all_items,
            PickerKind::Commands,
            false,
        );
        self.current_route = Route::Picker(overlay);
    }

    pub(in crate::app) fn plugin_slash_commands(
        &self,
    ) -> Vec<agena::plugin::PluginCommandCatalogItem> {
        self.backend
            .plugin_slash_commands()
            .into_iter()
            .filter(|entry| {
                plugin_command_slash_name(entry)
                    .is_some_and(|name| commands::find_command(name.as_str()).is_none())
            })
            .collect()
    }

    pub(in crate::app) fn open_resume_session_picker(&mut self) {
        self.open_resume_session_picker_with_query("");
    }

    pub(in crate::app) fn open_resume_session_picker_with_query(&mut self, query: &str) {
        let input = Editor::from_text(query.trim().to_string());
        let scope_session_id = (self.sessions.view_mode == SessionViewMode::Subtree)
            .then(|| self.current_or_selected_session_id())
            .flatten();
        let dialog =
            self.build_session_search_overlay(input, self.sessions.view_mode, scope_session_id);
        match dialog.meta.mode {
            SessionViewMode::Subtree => {
                let Some(session_id) = dialog.meta.scope_session_id else {
                    self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
                    return;
                };
                self.request_session_search_subtree(
                    session_id,
                    dialog.input.text().trim().to_string(),
                );
            }
            SessionViewMode::All | SessionViewMode::Roots => {
                self.request_session_search_page(
                    dialog.meta.mode,
                    dialog.input.text().trim().to_string(),
                    0,
                    None,
                );
            }
        }
        self.current_route = Route::SessionSearch(dialog);
    }

    pub(in crate::app) fn open_lineage_picker(&mut self) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        let dialog = self.build_picker_overlay(
            self.i18n.text_args(
                "overlay-lineage-title",
                &crate::fl_args!("session" => session_id),
            ),
            ui_text::t(&self.i18n, "overlay-lineage-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            Editor::default(),
            Vec::new(),
            PickerKind::Lineage { session_id },
            true,
        );
        self.current_route = Route::Picker(dialog);
        self.request_lineage(session_id);
    }

    pub(in crate::app) fn open_rewind_messages_picker(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        if self.prompt_for_pending_interactive_on_session(session_id) {
            return;
        }
        if self.session_is_busy(session_id) {
            self.flash_warning(ui_text::t(&self.i18n, "flash-session-busy"));
            return;
        }
        let dialog = self.build_picker_overlay(
            self.i18n.text_args(
                "overlay-rewind-title",
                &crate::fl_args!("session" => session_id),
            ),
            ui_text::t(&self.i18n, "overlay-rewind-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            Editor::default(),
            Vec::new(),
            PickerKind::RewindMessages { session_id },
            true,
        );
        self.current_route = Route::Picker(dialog);
        self.request_rewind_messages(session_id);
    }
}
use crate::app::{
    AgentProfile, AgentProfileStorage, App, ConfirmAction, Editor, ListWorkbenchState, Overlay,
    PermissionRuleDraft, PermissionRuleResource, PermissionRuleStudioOverlay, PickerItem,
    PickerKind, PickerOverlay, PickerValue, Route, SelectableListState, SessionViewMode, UiAction,
    UiResult, commands, fs, permission_rule_detail, permission_rule_draft_from_resource,
    permission_rule_label, permission_rule_studio_items, plugin_command_detail,
    plugin_command_slash_name, refresh_permission_rule_studio_dialog, ui_text,
};
