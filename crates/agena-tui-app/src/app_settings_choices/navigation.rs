impl App {
    pub(crate) fn open_runtime_config_in_editor(&mut self) {
        self.flash_warning(ui_text::t(
            &self.i18n,
            "flash-server-config-edit-in-settings",
        ));
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
        for entry in self.plugin_slash_operations() {
            let Some(name) = plugin_operation_slash_name(&entry) else {
                continue;
            };
            let key = format!(
                "plugin-operation:{}:{}",
                entry.plugin_id, entry.operation.id
            );
            let label = format!("/{name}");
            let detail = plugin_operation_detail(&entry);
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

    pub(crate) fn plugin_slash_operations(
        &self,
    ) -> Vec<agena_plugin_host::PluginOperationCatalogItem> {
        crate::app_backend::plugin_effects::plugin_slash_operations(&self.application)
            .into_iter()
            .filter(|entry| {
                plugin_operation_slash_name(entry)
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
            agena_tui_session::session_navigation::SessionNavigationMode::Open,
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
            agena_tui_session::session_navigation::SessionNavigationMode::Rewind,
            SessionNavigationQuery::RewindMessages { session_id },
        );
        self.current_route = Route::SessionNavigation(dialog);
        self.request_rewind_messages(session_id);
    }
}
use crate::{
    App, CommandPaletteCommand, CommandPaletteOverlay, Editor, PermissionRuleDraft,
    PermissionRuleStudioOverlay, Route, SessionNavigationQuery, commands,
    permission_rule_studio_items, plugin_operation_detail, plugin_operation_slash_name,
    refresh_permission_rule_studio_dialog, ui_text,
};
use agena_tui::command_palette::CommandPaletteItem;
use agena_tui_permission_studio::permission_rule_studio::PermissionRuleStudioPresentation;
use agena_tui_session::session_view::SessionViewMode;
use std::collections::BTreeMap;
