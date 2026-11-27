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
            initial_draft: draft.clone(),
            draft,
            editable: true,
            scope_locked: false,
            save_request: None,
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
        self.load_external_agent_command_preview();
        let (items, actions) = self.command_palette_items();
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

    pub(crate) fn refresh_command_palette(&mut self) {
        if !matches!(self.current_route, Route::CommandPalette(_)) {
            return;
        }
        let (items, actions) = self.command_palette_items();
        if let Route::CommandPalette(dialog) = &mut self.current_route {
            dialog.presentation.replace_items(items);
            dialog.actions = actions;
        }
    }

    fn command_palette_items(
        &self,
    ) -> (
        Vec<CommandPaletteItem>,
        BTreeMap<String, CommandPaletteCommand>,
    ) {
        let mut actions = BTreeMap::new();
        let mut items = Vec::new();
        for command in self.client_commands() {
            let key = format!("command:{}", command.id());
            let label = command.palette_invocation();
            let detail = commands::docs_summary(&self.i18n, command.docs(), command.id());
            items.push(CommandPaletteItem::new(
                key.clone(),
                label.clone(),
                detail.clone(),
                format!(
                    "{label} {detail} {} {}",
                    command.aliases().join(" "),
                    command.docs().usage.as_deref().unwrap_or_default()
                ),
            ));
            actions.insert(key, CommandPaletteCommand::BuiltIn(Box::new(command)));
        }
        for entry in self.plugin_slash_commands() {
            let Some(name) = plugin_command_slash_name(&entry) else {
                continue;
            };
            let key = format!("plugin-operation:{}:{}", entry.plugin_id, entry.command.id);
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
        let key = "agena:native-agent-browser".to_owned();
        let label = self.i18n.text("external-agent-browser");
        items.push(CommandPaletteItem::new(
            key.clone(),
            label.clone(),
            "ACP",
            format!("{label} acp claude code antigravity"),
        ));
        actions.insert(key, CommandPaletteCommand::NativeAgentBrowser);
        if let Some((session_id, agent, commands)) = self.external_agent_command_catalog() {
            for (key, text, action) in [
                (
                    "agena:native-agent-manage",
                    "external-agent-manage",
                    CommandPaletteCommand::NativeAgentManagement { session_id },
                ),
                (
                    "agena:native-agent-auth",
                    "external-agent-auth",
                    CommandPaletteCommand::NativeAgentAuth { session_id },
                ),
            ] {
                let label = self.i18n.text(text);
                items.push(CommandPaletteItem::new(
                    key,
                    label.clone(),
                    agent.label(),
                    format!("{label} {} acp", agent.label()),
                ));
                actions.insert(key.into(), action);
            }
            let key = "agena:external-agent-activity".to_owned();
            let label = ui_text::t(&self.i18n, "external-agent-activity-title");
            let detail = agent.label();
            items.push(CommandPaletteItem::new(
                key.clone(),
                label.clone(),
                detail,
                format!("{label} {detail} acp"),
            ));
            actions.insert(
                key,
                CommandPaletteCommand::ExternalAgentActivity { session_id },
            );
            let key = "agena:external-agent-work".to_owned();
            let label = ui_text::t(&self.i18n, "external-agent-work-cancel");
            items.push(CommandPaletteItem::new(
                key.clone(),
                label.clone(),
                detail,
                format!("{label} {detail} acp"),
            ));
            actions.insert(key, CommandPaletteCommand::ExternalAgentWork { session_id });
            for command in commands {
                let name = command.qualified_name();
                let key = format!("external:{name}");
                let label = format!("/{name}");
                let detail = crate::app_command_helpers::external_command_detail(
                    &self.i18n, &agent, command,
                );
                items.push(CommandPaletteItem::new(
                    key.clone(),
                    label.clone(),
                    detail.clone(),
                    format!("{label} {detail}"),
                ));
                actions.insert(
                    key,
                    CommandPaletteCommand::External {
                        session_id,
                        command: command.clone(),
                    },
                );
            }
        }
        if self.composer.text().is_empty()
            && self.composer_items.is_empty()
            && let Some((session_id, suggestions)) = self.external_agent_suggestions()
        {
            let label = ui_text::t(&self.i18n, "external-agent-use-suggestion");
            for (index, suggestion) in suggestions.iter().enumerate() {
                let key = format!("agena:external-agent-suggestion:{index}");
                let text = suggestion.display_text();
                let preview = text.split_whitespace().collect::<Vec<_>>().join(" ");
                items.push(CommandPaletteItem::new(
                    key.clone(),
                    label.clone(),
                    preview,
                    format!("{label} {text}"),
                ));
                actions.insert(
                    key,
                    CommandPaletteCommand::ExternalAgentSuggestion {
                        session_id,
                        suggestion: suggestion.clone(),
                    },
                );
            }
        }
        (items, actions)
    }

    pub(crate) fn external_agent_suggestions(
        &self,
    ) -> Option<(i64, &[agena_domain::ExternalAgentSuggestion])> {
        let (session_id, _, _) = self.external_agent_commands()?;
        let execution = self.transcript.execution.as_ref()?;
        Some((
            session_id,
            &execution.execution.external_agent_details.suggestions,
        ))
    }

    pub(crate) fn external_agent_commands(
        &self,
    ) -> Option<(
        i64,
        agena_domain::ExternalAgent,
        &[agena_domain::ExternalAgentCommand],
    )> {
        let execution = self.transcript.execution.as_ref()?;
        let session_id = self.current_or_selected_session_id()?;
        if execution.session.id != session_id {
            return None;
        }
        Some((
            session_id,
            execution.session.external_agent.clone()?,
            &execution.execution.external_agent_commands,
        ))
    }

    pub(crate) fn can_preview_external_agent_commands(&self, session_id: i64) -> bool {
        self.current_or_selected_session_id() == Some(session_id)
            && self.transcript.execution.as_ref().is_some_and(|execution| {
                execution.session.id == session_id
                    && execution.execution.external_agent_command_preview
            })
    }

    pub(crate) fn external_agent_command_catalog(
        &self,
    ) -> Option<(
        i64,
        agena_domain::ExternalAgent,
        &[agena_domain::ExternalAgentCommand],
    )> {
        let (session_id, agent, commands) = self.external_agent_commands()?;
        if self.can_preview_external_agent_commands(session_id)
            && let Some((id, preview)) = &self.external_agent_command_preview
            && *id == session_id
        {
            return Some((session_id, agent, preview));
        }
        Some((session_id, agent, commands))
    }

    /// The built-in commands this client runs, in catalog order.
    ///
    /// The published catalog is the only source: before the first plugin
    /// surface snapshot arrives the list is empty rather than a hand-written
    /// fallback.
    pub(crate) fn client_commands(&self) -> Vec<commands::ClientCommand> {
        let external_agent = self.external_agent_commands().is_some();
        commands::client_commands(
            self.application.plugin_catalog().as_ref(),
            self.is_bot_session(),
        )
        .into_iter()
        .filter(|command| {
            !external_agent || commands::command_applies_to_external_agent(command.action())
        })
        .collect()
    }

    /// Resolve `/name` to a built-in command this client can run.
    pub(crate) fn find_client_command(&self, name: &str) -> Option<commands::ClientCommand> {
        let external_agent = self.external_agent_commands().is_some();
        commands::find_client_command(
            self.application.plugin_catalog().as_ref(),
            name,
            self.is_bot_session(),
        )
        .filter(|command| {
            !external_agent || commands::command_applies_to_external_agent(command.action())
        })
    }

    /// The commands a server-owned target resolves, i.e. everything that is not
    /// this client's own to run.
    pub(crate) fn plugin_slash_commands(
        &self,
    ) -> Vec<agena_plugin_sdk::inspection::CommandCatalogItem> {
        crate::app_backend::plugin_effects::plugin_slash_commands(
            &self.application,
            self.is_bot_session(),
        )
    }

    pub(crate) fn open_resume_session_picker(&mut self) {
        self.open_resume_session_picker_with_query("");
    }

    pub(crate) fn open_resume_session_picker_with_query(&mut self, query: &str) {
        let input = Editor::from_text(query.trim().to_string());
        let scope_session_id = (self.sessions.view_mode() == SessionViewMode::Subtree)
            .then(|| self.current_or_selected_session_id())
            .flatten();
        let mut dialog =
            self.build_session_search_overlay(input, self.sessions.view_mode(), scope_session_id);
        match dialog.meta.mode {
            SessionViewMode::Subtree => {
                let Some(session_id) = dialog.meta.scope_session_id else {
                    self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
                    return;
                };
                dialog.meta.request_id = self.request_session_search_subtree(
                    session_id,
                    dialog.input.text().trim().to_string(),
                );
            }
            SessionViewMode::All | SessionViewMode::Roots => {
                dialog.meta.request_id = self.request_session_search_page(
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
    permission_rule_studio_items, plugin_command_detail, plugin_command_slash_name,
    refresh_permission_rule_studio_dialog, ui_text,
};
use agena_tui::command_palette::CommandPaletteItem;
use agena_tui_permission_studio::permission_rule_studio::PermissionRuleStudioPresentation;
use agena_tui_session::session_view::SessionViewMode;
use std::collections::BTreeMap;
