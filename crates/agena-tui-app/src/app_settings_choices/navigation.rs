impl App {
    pub(crate) fn open_runtime_config_in_editor(&mut self) {
        let path = self.backend.config_path();
        if !path.exists() {
            if let Some(parent) = path.parent()
                && let Err(error) = fs::create_dir_all(parent)
            {
                self.flash_error(self.i18n.text_args(
                    "flash-config-dir-prepare-failed",
                    &agena_tui::fl_args!(
                        "path" => parent.display().to_string(),
                        "error" => error.to_string(),
                    ),
                ));
                return;
            }
            if let Err(error) = fs::write(&path, "") {
                self.flash_error(self.i18n.text_args(
                    "flash-config-file-create-failed",
                    &agena_tui::fl_args!(
                        "path" => path.display().to_string(),
                        "error" => error.to_string(),
                    ),
                ));
                return;
            }
        }
        self.pending_ui_action = Some(UiAction::OpenPath { path });
    }

    pub(crate) fn open_agent_profile_source(&mut self, profile: &AgentProfile) {
        match self.agent_profile_storage(profile) {
            AgentProfileStorage::Markdown => {
                if let Some(path) = profile.source_path.clone() {
                    self.pending_ui_action = Some(UiAction::OpenPath { path: path.into() });
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

    pub(crate) fn open_inspector_picker(
        &mut self,
        title: String,
        prompt: String,
        query: &str,
        rows: Vec<agena_tui_backend::InspectorRow>,
    ) {
        let mut overlay = self.build_selection_picker_overlay(
            title,
            prompt,
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            query.to_string(),
            SelectionPickerQuery::Inspector,
            false,
        );
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let key = format!("inspector:{index}");
                (
                    agena_tui::selection_picker::SelectionPickerItem::new(
                        key,
                        row.label.clone(),
                        row.detail.clone(),
                        format!("{} {}", row.label, row.detail),
                    ),
                    SelectionPickerCommand::Inspector,
                )
            })
            .collect::<Vec<_>>();
        overlay.actions = rows
            .iter()
            .map(|(item, action)| (item.key.clone(), action.clone()))
            .collect();
        overlay
            .presentation
            .replace_items(rows.into_iter().map(|(item, _)| item).collect());
        self.current_route = Route::SelectionPicker(overlay);
    }

    pub(crate) fn build_permission_rule_studio_overlay(
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
            return_permission: None,
            presentation: PermissionRuleStudioPresentation::new(title, footer, items, selected),
            editor: None,
        }
    }

    pub(crate) fn refresh_permission_rule_studio(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
    ) {
        refresh_permission_rule_studio_dialog(&self.i18n, dialog);
    }

    pub(crate) fn open_snapshot_remove_confirm(&mut self, session_id: i64, discard_changes: bool) {
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

    pub(crate) fn open_command_palette(&mut self) {
        let mut actions = BTreeMap::new();
        let mut items = Vec::new();
        for spec in commands::COMMANDS {
            let key = format!("command:{}", spec.name);
            let label = spec.palette_invocation();
            let detail = ui_text::t(&self.i18n, spec.summary_key);
            items.push(CommandPaletteItem::new(
                key.clone(),
                label.clone(),
                detail.clone(),
                format!(
                    "{label} {detail} {} {}",
                    spec.aliases.join(" "),
                    spec.arguments
                ),
            ));
            actions.insert(key, CommandPaletteCommand::BuiltIn(spec));
        }
        for entry in self.plugin_slash_commands() {
            let Some(name) = plugin_command_slash_name(&entry) else {
                continue;
            };
            let key = format!("plugin-command:{}:{}", entry.plugin_id, entry.command.id);
            let label = format!("/{name}");
            let detail = plugin_command_detail(&entry);
            items.push(CommandPaletteItem::new(
                key.clone(),
                label.clone(),
                detail.clone(),
                format!("{label} {detail}"),
            ));
            actions.insert(key, CommandPaletteCommand::Plugin(Box::new(entry)));
        }
        let presentation = agena_tui::command_palette::new_presentation(
            ui_text::t(&self.i18n, "overlay-commands-title"),
            ui_text::t(&self.i18n, "overlay-commands-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            items,
        );
        self.current_route = Route::CommandPalette(CommandPaletteOverlay {
            presentation,
            actions,
        });
    }

    pub(crate) fn plugin_slash_commands(&self) -> Vec<agena_plugin_host::PluginCommandCatalogItem> {
        self.backend
            .plugin_slash_commands()
            .into_iter()
            .filter(|entry| {
                plugin_command_slash_name(entry)
                    .is_some_and(|name| commands::find_command(name.as_str()).is_none())
            })
            .collect()
    }

    pub(crate) fn open_resume_session_picker(&mut self) {
        self.open_resume_session_picker_with_query("");
    }

    pub(crate) fn open_resume_session_picker_with_query(&mut self, query: &str) {
        let input = Editor::from_text(query.trim().to_string());
        let scope_session_id = (self.sessions.view_mode() == SessionViewMode::Subtree)
            .then(|| self.current_or_selected_session_id())
            .flatten();
        let dialog =
            self.build_session_search_overlay(input, self.sessions.view_mode(), scope_session_id);
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

    pub(crate) fn open_lineage_picker(&mut self) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        let dialog = self.build_session_navigation_overlay(
            self.i18n.text_args(
                "overlay-lineage-title",
                &agena_tui::fl_args!("session" => session_id),
            ),
            ui_text::t(&self.i18n, "overlay-lineage-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            agena_tui::session_navigation::SessionNavigationMode::Open,
            SessionNavigationQuery::Lineage { session_id },
        );
        self.current_route = Route::SessionNavigation(dialog);
        self.request_lineage(session_id);
    }

    pub(crate) fn open_rewind_messages_picker(&mut self) {
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
        let dialog = self.build_session_navigation_overlay(
            self.i18n.text_args(
                "overlay-rewind-title",
                &agena_tui::fl_args!("session" => session_id),
            ),
            ui_text::t(&self.i18n, "overlay-rewind-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            agena_tui::session_navigation::SessionNavigationMode::Rewind,
            SessionNavigationQuery::RewindMessages { session_id },
        );
        self.current_route = Route::SessionNavigation(dialog);
        self.request_rewind_messages(session_id);
    }
}
use crate::{
    AgentProfile, AgentProfileStorage, App, CommandPaletteCommand, CommandPaletteOverlay,
    ConfirmAction, Editor, Overlay, PermissionRuleDraft, PermissionRuleStudioOverlay, Route,
    SelectionPickerCommand, SelectionPickerQuery, SessionNavigationQuery, UiAction, commands, fs,
    permission_rule_studio_items, plugin_command_detail, plugin_command_slash_name,
    refresh_permission_rule_studio_dialog, ui_text,
};
use agena_tui::command_palette::CommandPaletteItem;
use agena_tui::permission_rule_studio::PermissionRuleStudioPresentation;
use agena_tui::session_view::SessionViewMode;
use std::collections::BTreeMap;
