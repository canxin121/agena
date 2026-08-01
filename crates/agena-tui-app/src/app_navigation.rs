impl App {
    pub(crate) fn open_child_sessions_picker(&mut self) {
        let Some(parent_session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        let dialog = self.build_session_navigation_overlay(
            self.i18n.text_args(
                "overlay-children-title",
                &agena_tui::fl_args!("session" => parent_session_id),
            ),
            ui_text::t(&self.i18n, "overlay-children-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            agena_tui_session::session_navigation::SessionNavigationMode::Open,
            SessionNavigationQuery::ChildSessions { parent_session_id },
        );
        self.current_route = Route::SessionNavigation(dialog);
        self.request_child_sessions(parent_session_id);
    }

    pub(crate) fn open_rewind_confirm_overlay(
        &mut self,
        session_id: i64,
        turn_id: agena_domain::TurnId,
        message_text: String,
        target: String,
    ) {
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-rewind-confirm-title"),
            vec![
                self.i18n.text_args(
                    "overlay-rewind-confirm-keep",
                    &agena_tui::fl_args!("target" => target.clone()),
                ),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-warning"),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-draft"),
            ],
            ConfirmAction::Rewind {
                session_id,
                turn_id,
                message_text,
                target,
            },
        )));
    }

    pub(crate) fn lineage_session_navigation_item(
        &self,
        session: SessionResource,
        item: agena_tui_session::session_navigation::SessionLineageItem,
    ) -> (
        agena_tui_session::session_navigation::SessionNavigationItem,
        SessionNavigationCommand,
    ) {
        let mut detail_parts = vec![ui_text::session_meta(
            &self.i18n,
            session.id,
            session.message_count,
            session.updated_at,
        )];
        detail_parts.push(ui_text::t(&self.i18n, item.relation.localization_key()));
        if item.is_leaf {
            detail_parts.push(ui_text::t(&self.i18n, "session-tag-leaf"));
        }
        if let Some(parent_id) = session.parent_id {
            detail_parts.push(self.i18n.text_args(
                "session-summary-parent",
                &agena_tui::fl_args!("id" => parent_id),
            ));
        }
        if session.child_session_count > 0 {
            detail_parts.push(self.i18n.text_args(
                "session-summary-children",
                &agena_tui::fl_args!("count" => session.child_session_count as i64),
            ));
        }

        let label = format!(
            "{}{}{}",
            "  ".repeat(item.depth),
            if item.depth == 0 { "◆ " } else { "↳ " },
            session.title
        );
        let detail = detail_parts.join(" | ");
        (
            agena_tui_session::session_navigation::SessionNavigationItem::new(
                format!("session:{}", session.id),
                label.clone(),
                detail.clone(),
                format!("{label} {detail} #{}", session.id),
            ),
            SessionNavigationCommand::OpenSession {
                session_id: session.id,
            },
        )
    }

    pub(crate) fn session_search_item(&self, session: SessionResource) -> SessionSearchItem {
        let mut detail_parts = vec![ui_text::session_meta(
            &self.i18n,
            session.id,
            session.message_count,
            session.updated_at,
        )];
        if self.transcript.session_id == Some(session.id) {
            detail_parts.push(ui_text::t(&self.i18n, "session-tag-current"));
        }
        if let Some(parent_id) = session.parent_id {
            detail_parts.push(self.i18n.text_args(
                "session-summary-parent",
                &agena_tui::fl_args!("id" => parent_id),
            ));
        }
        if session.child_session_count > 0 {
            detail_parts.push(self.i18n.text_args(
                "session-summary-children",
                &agena_tui::fl_args!("count" => session.child_session_count as i64),
            ));
        }

        SessionSearchItem {
            session_id: session.id,
            title: session.title.clone(),
            label: session.title,
            detail: detail_parts.join(" | "),
        }
    }

    pub(crate) fn rewind_turn_navigation_item(
        &self,
        session_id: i64,
        turn: agena_domain::TurnSnapshot,
    ) -> (
        agena_tui_session::session_navigation::SessionNavigationItem,
        SessionNavigationCommand,
    ) {
        let message_text = turn.input.text();
        let normalized = message_text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let label = if normalized.is_empty() {
            format!("Turn {}", turn.sequence)
        } else {
            normalized.chars().take(96).collect()
        };
        let detail = format!(
            "turn {} | {}",
            turn.sequence,
            format_timestamp(
                chrono::DateTime::<chrono::Utc>::from_timestamp_millis(turn.created_at_ms)
                    .unwrap_or_default()
            )
        );
        let target = format!(
            "{} ({})",
            label,
            detail.split(" | ").next().unwrap_or_default()
        );
        (
            agena_tui_session::session_navigation::SessionNavigationItem::new(
                format!("turn:{}", turn.id),
                label.clone(),
                detail.clone(),
                format!("{label} {detail} {}", turn.id),
            ),
            SessionNavigationCommand::Rewind {
                session_id,
                turn_id: turn.id,
                message_text,
                target,
            },
        )
    }

    pub(crate) fn refresh_timeline_overlay(dialog: &mut TimelineOverlay) {
        dialog.refresh_results();
    }

    pub(crate) fn prepare_composer_command(&mut self, command_name: &str) {
        self.composer.set_text(format!("/{command_name} "));
        self.focus = Focus::Composer;
        self.sync_composer_suggestions();
    }

    pub(crate) fn persist_current_session_model_stack(&mut self) -> bool {
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return false;
        };
        let options = self.run_options.model_stack_request();
        match self.block_on_async(self.backend.update_session_selection(session_id, options)) {
            Ok(execution) => {
                let _ = self.apply_transcript_execution(execution);
                self.request_sessions(false);
                true
            }
            Err(error) => {
                self.flash_error(error);
                false
            }
        }
    }

    pub(crate) fn apply_model_override(&mut self, model: ModelRef) -> bool {
        let previous = self.run_options.clone();
        self.run_options
            .replace_model_stack(Some(model.clone()), None, None, None, None);
        if !self.persist_current_session_model_stack() {
            self.run_options = previous;
            return false;
        }
        self.focus = Focus::Composer;
        self.flash_success(self.i18n.text_args(
            "flash-model-selected",
            &agena_tui::fl_args!(
                "model" => format!("{}/{}", model.provider_id, model.model_id),
            ),
        ));
        self.open_session_model_thinking_step_or_next();
        true
    }

    pub(crate) fn current_or_selected_session_id(&self) -> Option<i64> {
        if self.focus == Focus::Sessions {
            self.sessions
                .current_selected_id()
                .or(self.transcript.session_id)
        } else {
            self.transcript
                .session_id
                .or_else(|| self.sessions.current_selected_id())
        }
    }

    pub(crate) fn begin_run_operation(
        &mut self,
        target: RunActivityTarget,
        operation: RunOperation,
    ) {
        self.run_activity.begin(target, operation);
    }

    pub(crate) fn finish_run_operation(
        &mut self,
        target: RunActivityTarget,
        operation: RunOperation,
    ) {
        self.run_activity.finish(target, operation);
    }

    pub(crate) fn session_activity(&self, session_id: i64) -> SessionActivity {
        if self.transcript.session_id == Some(session_id)
            && let Some(execution) = self.transcript.execution.as_ref()
        {
            if let Some(kind) = pending_interactive_kind_for_execution(execution) {
                return match kind {
                    PendingInteractiveKind::Permission => SessionActivity::AwaitingPermission,
                    PendingInteractiveKind::UserInput => SessionActivity::AwaitingUserInput,
                };
            }
            if execution.workflow_state == agena_api::resource::WorkflowState::Blocked {
                return SessionActivity::Blocked;
            }
            if execution.active_execution.is_some() {
                return SessionActivity::Running;
            }
        }

        if self
            .run_activity
            .is_active(RunActivityTarget::Session(session_id))
        {
            SessionActivity::Running
        } else {
            SessionActivity::Idle
        }
    }

    pub(crate) fn current_session_activity(&self) -> SessionActivity {
        match self.transcript.session_id {
            Some(session_id) => self.session_activity(session_id),
            None if self.run_activity.is_active(RunActivityTarget::NewSession) => {
                SessionActivity::Running
            }
            None => SessionActivity::Idle,
        }
    }

    pub(crate) fn active_run_session_id(&self) -> Option<i64> {
        self.transcript.session_id.filter(|session_id| {
            self.run_activity
                .is_active(RunActivityTarget::Session(*session_id))
                || self.session_activity(*session_id).is_running()
        })
    }

    pub(crate) fn session_is_busy(&self, session_id: i64) -> bool {
        self.session_activity(session_id).is_busy()
    }

    pub(crate) fn current_or_selected_session_title(&self) -> Option<String> {
        if self.focus == Focus::Sessions
            && let Some(session) = self.sessions.current_selected()
        {
            return Some(session.title.clone());
        }
        if let Some(execution) = self.transcript.execution.as_ref() {
            return Some(execution.session.title.clone());
        }
        if self.transcript.session_id.is_some() && !self.transcript.session_title.trim().is_empty()
        {
            return Some(self.transcript.session_title.clone());
        }
        self.sessions
            .current_selected()
            .map(|session| session.title.clone())
    }

    pub(crate) fn current_parent_session_id(&self) -> Option<i64> {
        self.transcript.execution.as_ref()?.session.parent_id
    }

    pub(crate) fn sync_session_list_selection_to_current_execution(&mut self) {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return;
        };
        if self.transcript.session_id != Some(execution.session.id) {
            return;
        }
        let session_list = self.sessions.view();
        if let Some(session_id) =
            preferred_visible_session_selection(&execution.session, session_list.items)
        {
            let _ = self.sessions.select_by_id(session_id);
        }
    }

    pub(crate) fn current_lineage_context_parts(&self) -> Vec<String> {
        let Some(lineage) = self.current_lineage.as_ref() else {
            return Vec::new();
        };
        if self.transcript.session_id != Some(lineage.session_id) {
            return Vec::new();
        }

        let mut parts = vec![
            self.i18n.text_args(
                "session-summary-root",
                &agena_tui::fl_args!("id" => lineage.summary.root_session_id),
            ),
            self.i18n.text_args(
                "session-summary-depth",
                &agena_tui::fl_args!("depth" => lineage.summary.depth as i64),
            ),
        ];
        if lineage.summary.side_branch_count > 0 {
            parts.push(self.i18n.text_args(
                "session-summary-side-branches",
                &agena_tui::fl_args!("count" => lineage.summary.side_branch_count as i64),
            ));
        }
        if lineage.summary.descendant_count > 0 {
            parts.push(self.i18n.text_args(
                "session-summary-descendants",
                &agena_tui::fl_args!("count" => lineage.summary.descendant_count as i64),
            ));
        }
        parts
    }

    pub(crate) fn current_session_path_label(&self) -> Option<String> {
        self.transcript.session_id?;
        let effective = self
            .transcript
            .execution
            .as_ref()
            .filter(|execution| self.transcript.session_id == Some(execution.session.id))
            .and_then(|execution| execution.execution.effective_workspace_root.as_deref())
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let path = effective
            .map(str::to_owned)
            .unwrap_or_else(|| self.backend.workspace_root().display().to_string());
        if path.trim().is_empty() {
            return None;
        }
        Some(path)
    }

    pub(crate) fn open_parent_session(&mut self) {
        let Some(parent_id) = self.current_parent_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-parent-session-missing"));
            return;
        };
        self.open_session(
            parent_id,
            ui_text::session_fallback_title(&self.i18n, parent_id),
        );
        self.focus = Focus::Transcript;
    }

    pub(crate) fn current_session_model_ref(&self) -> Option<ModelRef> {
        if let Some(model) = self.run_options.model.as_ref() {
            return Some(model.clone());
        }
        let execution = self.transcript.execution.as_ref()?;
        let provider_id = execution.execution.model_provider_id.as_deref()?;
        let model_id = execution.execution.model_id.as_deref()?;
        Some(
            execution
                .execution
                .model_adapter_id
                .as_deref()
                .map(|adapter_id| ModelRef::new_with_adapter(provider_id, adapter_id, model_id))
                .unwrap_or_else(|| ModelRef::new(provider_id, model_id)),
        )
    }

    pub(crate) fn handle_confirm_action(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Rewind {
                session_id,
                turn_id,
                message_text,
                target,
            } => self.request_session_rewind(session_id, turn_id, message_text, target),
            ConfirmAction::PermissionStudioDeletePathRule { pattern } => {
                self.delete_permission_studio_path_rule(pattern.as_str())
            }
            ConfirmAction::PermissionStudioDeleteNetworkRule { target } => {
                self.delete_permission_studio_network_rule(target.as_str())
            }
            ConfirmAction::PermissionStudioDeleteToolTag { key } => {
                self.delete_permission_studio_tool_tag(key.as_str())
            }
            ConfirmAction::PermissionStudioDeleteToolName { key } => {
                self.delete_permission_studio_tool_name(key.as_str())
            }
            ConfirmAction::PermissionStudioDeleteToolRule { tool_name } => {
                self.delete_permission_studio_tool_rule(tool_name.as_str())
            }
            ConfirmAction::PermissionStudioDeleteToolCommandPattern { tool_name, pattern } => self
                .delete_permission_studio_tool_command_pattern(
                    tool_name.as_str(),
                    pattern.as_str(),
                ),
            ConfirmAction::ProviderStudioDeleteProvider { provider_id } => {
                let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return;
                };
                dialog.saving = true;
                self.request_provider_studio_delete_provider(provider_id);
                self.restore_provider_studio_dialog(host, dialog);
            }
            ConfirmAction::ProviderStudioDeleteAdapter { adapter_id } => {
                let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return;
                };
                self.delete_provider_studio_adapter(&mut dialog, adapter_id);
                self.restore_provider_studio_dialog(host, dialog);
            }
            ConfirmAction::ProviderStudioDeleteModel {
                adapter_id,
                model_id,
            } => {
                let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return;
                };
                self.delete_provider_studio_model(&mut dialog, adapter_id, model_id);
                self.restore_provider_studio_dialog(host, dialog);
            }
            ConfirmAction::SkillStudioDelete { name } => {
                self.delete_skill_studio_skill(name.as_str())
            }
        }
    }
}
use crate::{
    App, ConfirmAction, ModelRef, Overlay, PendingInteractiveKind, Route, RunActivityTarget,
    RunOperation, SessionActivity, SessionNavigationCommand, SessionNavigationQuery,
    SessionResource, SessionSearchItem, TimelineOverlay, format_timestamp,
    pending_interactive_kind_for_execution, preferred_visible_session_selection, ui_text,
};
use agena_tui::main_focus::Focus;
