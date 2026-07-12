impl App {
    pub(in crate::app) fn open_child_sessions_picker(&mut self) {
        let Some(parent_session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        let dialog = self.build_picker_overlay(
            self.i18n.text_args(
                "overlay-children-title",
                &crate::fl_args!("session" => parent_session_id),
            ),
            ui_text::t(&self.i18n, "overlay-children-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            Editor::default(),
            Vec::new(),
            PickerKind::ChildSessions { parent_session_id },
            true,
        );
        self.current_route = Route::Picker(dialog);
        self.request_child_sessions(parent_session_id);
    }

    pub(in crate::app) fn open_rewind_confirm_overlay(
        &mut self,
        session_id: i64,
        message_id: i64,
        target: String,
    ) {
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-rewind-confirm-title"),
            vec![
                self.i18n.text_args(
                    "overlay-rewind-confirm-keep",
                    &crate::fl_args!("target" => target.clone()),
                ),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-warning"),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-draft"),
            ],
            ConfirmAction::Rewind {
                session_id,
                message_id,
                target,
            },
        )));
    }

    pub(in crate::app) fn lineage_session_picker_item(
        &self,
        item: LineageSessionItem,
    ) -> PickerItem {
        let session = item.session;
        let mut detail_parts = vec![ui_text::session_meta(
            &self.i18n,
            session.id,
            session.message_count,
            session.updated_at,
        )];
        detail_parts.push(ui_text::t(
            &self.i18n,
            lineage_relation_tag_key(item.relation),
        ));
        if item.is_leaf {
            detail_parts.push(ui_text::t(&self.i18n, "session-tag-leaf"));
        }
        if let Some(parent_id) = session.parent_id {
            detail_parts.push(self.i18n.text_args(
                "session-summary-parent",
                &crate::fl_args!("id" => parent_id),
            ));
        }
        if session.child_session_count > 0 {
            detail_parts.push(self.i18n.text_args(
                "session-summary-children",
                &crate::fl_args!("count" => session.child_session_count as i64),
            ));
        }

        PickerItem {
            label: format!(
                "{}{}{}",
                "  ".repeat(item.depth),
                if item.depth == 0 { "◆ " } else { "↳ " },
                session.title
            ),
            detail: detail_parts.join(" | "),
            value: PickerValue::Session(session.id),
        }
    }

    pub(in crate::app) fn session_search_item(
        &self,
        session: SessionResource,
    ) -> SessionSearchItem {
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
                &crate::fl_args!("id" => parent_id),
            ));
        }
        if session.child_session_count > 0 {
            detail_parts.push(self.i18n.text_args(
                "session-summary-children",
                &crate::fl_args!("count" => session.child_session_count as i64),
            ));
        }

        SessionSearchItem {
            label: session.title.clone(),
            detail: detail_parts.join(" | "),
            session,
        }
    }

    pub(in crate::app) fn rewind_message_picker_item(
        &self,
        message: MessageResource,
    ) -> PickerItem {
        PickerItem {
            label: self.rewind_message_target_label(&message),
            detail: format!(
                "#{} | {} | {}",
                message.id,
                ui_text::message_state_label(&self.i18n, message.state),
                format_timestamp(message.created_at)
            ),
            value: PickerValue::Message(message.id),
        }
    }

    pub(in crate::app) fn rewind_message_target_label(&self, message: &MessageResource) -> String {
        format!(
            "[{}] {}",
            ui_text::role_label(&self.i18n, message.role),
            rewind_message_preview(message, &self.i18n)
        )
    }

    pub(in crate::app) fn refresh_picker_overlay(dialog: &mut PickerOverlay) {
        dialog.refresh_results();
    }

    pub(in crate::app) fn refresh_session_model_chooser_overlay(
        dialog: &mut SessionModelChooserOverlay,
        prefer_current_model: bool,
        current_model: Option<&ModelRef>,
    ) {
        let previous_model = if dialog.query_changed_since_results() {
            None
        } else {
            dialog.selected_item().map(|item| item.model.clone())
        };
        dialog.refresh_results();
        if dialog.result_count() == 0 {
            dialog.selected = 0;
            return;
        }

        if prefer_current_model
            && let Some(current_model) = current_model
            && dialog
                .select_item_where(|item| session_model_matches_current(&item.model, current_model))
        {
            return;
        }

        if let Some(previous_model) = previous_model
            && dialog.select_item_where(|item| item.model == previous_model)
        {
            return;
        }

        dialog.clamp_selection();
    }

    pub(in crate::app) fn refresh_timeline_overlay(dialog: &mut TimelineOverlay) {
        dialog.refresh_results();
    }

    pub(in crate::app) fn handle_picker_selection(&mut self, kind: PickerKind, item: PickerItem) {
        match (kind, item.value) {
            (PickerKind::Commands, PickerValue::Command(spec)) => {
                self.execute_command(spec, "");
            }
            (PickerKind::Commands, PickerValue::RuntimeTool(tool_name)) => {
                self.composer
                    .set_text(format!("/{tool_name} ").trim_end().to_string());
                self.focus = Focus::Composer;
                self.sync_composer_suggestions();
            }
            (PickerKind::Lineage { .. }, PickerValue::Session(session_id)) => {
                self.open_session(
                    session_id,
                    ui_text::session_fallback_title(&self.i18n, session_id),
                );
                self.focus = Focus::Transcript;
            }
            (PickerKind::RewindMessages { session_id }, PickerValue::Message(message_id)) => {
                let target = format!(
                    "{} ({})",
                    item.label,
                    item.detail.split(" | ").next().unwrap_or_default()
                );
                self.open_rewind_confirm_overlay(session_id, message_id, target);
            }
            (
                PickerKind::Providers(ProviderPickerPurpose::SetProvider),
                PickerValue::Provider(provider),
            ) => {
                self.apply_provider_override(provider);
            }
            (PickerKind::ChildSessions { .. }, PickerValue::Session(session_id)) => {
                self.open_session(
                    session_id,
                    ui_text::session_fallback_title(&self.i18n, session_id),
                );
                self.focus = Focus::Transcript;
            }
            (PickerKind::PermissionRules, PickerValue::PermissionRuleCreate) => {
                self.open_permission_rule_studio(None, None);
            }
            (PickerKind::PermissionRules, PickerValue::PermissionRule(rule)) => {
                self.open_permission_rule_studio(Some(&rule), None);
            }
            (PickerKind::Inspector, PickerValue::Inspector) => {}
            _ => {}
        }
    }

    pub(in crate::app) fn apply_provider_override(&mut self, provider: ProviderSummaryResource) {
        self.run_options.model = Some(match provider.defaults.adapter.clone() {
            Some(adapter_id) => ModelRef::new_with_adapter(
                provider.provider_id.clone(),
                adapter_id,
                provider.defaults.model.clone(),
            ),
            None => ModelRef::new(
                provider.provider_id.clone(),
                provider.defaults.model.clone(),
            ),
        });
        self.run_options.thinking_mode = None;
        self.run_options.speed_mode = None;
        self.run_options.verbosity = None;
        self.run_options.parallel_tool_calls = None;
        self.focus = Focus::Composer;
        self.flash_success(self.i18n.text_args(
            "flash-provider-selected",
            &crate::fl_args!(
                "provider" => provider.provider_id,
                "model" => provider.defaults.model,
            ),
        ));
    }

    pub(in crate::app) fn apply_model_override(&mut self, model: ModelRef) {
        self.run_options.model = Some(model.clone());
        self.run_options.thinking_mode = None;
        self.run_options.speed_mode = None;
        self.run_options.verbosity = None;
        self.run_options.parallel_tool_calls = None;
        self.focus = Focus::Composer;
        self.flash_success(self.i18n.text_args(
            "flash-model-selected",
            &crate::fl_args!(
                "model" => format!("{}/{}", model.provider_id, model.model_id),
            ),
        ));
        self.open_session_model_thinking_step_or_next();
    }

    pub(in crate::app) fn current_or_selected_session_id(&self) -> Option<i64> {
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

    pub(in crate::app) fn begin_run_operation(
        &mut self,
        target: RunActivityTarget,
        operation: RunOperation,
    ) {
        self.run_activity.begin(target, operation);
    }

    pub(in crate::app) fn finish_run_operation(
        &mut self,
        target: RunActivityTarget,
        operation: RunOperation,
    ) {
        self.run_activity.finish(target, operation);
    }

    pub(in crate::app) fn session_activity(&self, session_id: i64) -> SessionActivity {
        if self.transcript.session_id == Some(session_id)
            && let Some(execution) = self.transcript.execution.as_ref()
        {
            if let Some(kind) = pending_interactive_kind_for_execution(execution) {
                return match kind {
                    PendingInteractiveKind::Permission => SessionActivity::AwaitingPermission,
                    PendingInteractiveKind::UserInput => SessionActivity::AwaitingUserInput,
                };
            }
            if execution.workflow_state == agena::session::WorkflowState::Blocked {
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

    pub(in crate::app) fn current_session_activity(&self) -> SessionActivity {
        match self.transcript.session_id {
            Some(session_id) => self.session_activity(session_id),
            None if self.run_activity.is_active(RunActivityTarget::NewSession) => {
                SessionActivity::Running
            }
            None => SessionActivity::Idle,
        }
    }

    pub(in crate::app) fn active_run_session_id(&self) -> Option<i64> {
        self.transcript
            .session_id
            .filter(|session_id| self.session_activity(*session_id).is_running())
    }

    pub(in crate::app) fn session_is_busy(&self, session_id: i64) -> bool {
        self.session_activity(session_id).is_busy()
    }

    pub(in crate::app) fn current_or_selected_session_title(&self) -> Option<String> {
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

    pub(in crate::app) fn current_parent_session_id(&self) -> Option<i64> {
        self.transcript.execution.as_ref()?.session.parent_id
    }

    pub(in crate::app) fn sync_session_list_selection_to_current_execution(&mut self) {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return;
        };
        if self.transcript.session_id != Some(execution.session.id) {
            return;
        }
        if let Some(session_id) = preferred_visible_session_selection(
            &execution.session,
            self.sessions.list.items.as_slice(),
        ) {
            let _ = self.sessions.select_by_id(session_id);
        }
    }

    pub(in crate::app) fn current_lineage_context_parts(&self) -> Vec<String> {
        let Some(lineage) = self.current_lineage.as_ref() else {
            return Vec::new();
        };
        if self.transcript.session_id != Some(lineage.session_id) {
            return Vec::new();
        }

        let mut parts = vec![
            self.i18n.text_args(
                "session-summary-root",
                &crate::fl_args!("id" => lineage.summary.root_id),
            ),
            self.i18n.text_args(
                "session-summary-depth",
                &crate::fl_args!("depth" => lineage.summary.depth as i64),
            ),
        ];
        if lineage.summary.side_branch_count > 0 {
            parts.push(self.i18n.text_args(
                "session-summary-side-branches",
                &crate::fl_args!("count" => lineage.summary.side_branch_count as i64),
            ));
        }
        if lineage.summary.descendant_count > 0 {
            parts.push(self.i18n.text_args(
                "session-summary-descendants",
                &crate::fl_args!("count" => lineage.summary.descendant_count as i64),
            ));
        }
        parts
    }

    pub(in crate::app) fn current_session_path_label(&self) -> Option<String> {
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

    pub(in crate::app) fn open_parent_session(&mut self) {
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

    pub(in crate::app) fn current_session_model_ref(&self) -> Option<ModelRef> {
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

    pub(in crate::app) fn handle_confirm_action(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Rewind {
                session_id,
                message_id,
                target,
            } => self.request_session_rewind(session_id, message_id, target),
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
            ConfirmAction::ExitSnapshot {
                session_id,
                discard_changes,
            } => match self
                .backend
                .exit_snapshot(session_id, "remove".to_string(), discard_changes)
            {
                Ok(output) => self.flash_success(ui_text::snapshot_exit_message(
                    &self.i18n,
                    output.action.as_deref(),
                    output.path.as_str(),
                )),
                Err(error) => self.flash_error(error.to_string()),
            },
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
        }
    }
}
use crate::app::{
    App, ConfirmAction, Editor, Focus, LineageSessionItem, MessageResource, ModelRef, Overlay,
    PendingInteractiveKind, PickerItem, PickerKind, PickerOverlay, PickerValue,
    ProviderPickerPurpose, ProviderSummaryResource, Route, RunActivityTarget, RunOperation,
    SessionActivity, SessionModelChooserOverlay, SessionResource, SessionSearchItem,
    TimelineOverlay, format_timestamp, lineage_relation_tag_key,
    pending_interactive_kind_for_execution, preferred_visible_session_selection,
    rewind_message_preview, session_model_matches_current, ui_text,
};
