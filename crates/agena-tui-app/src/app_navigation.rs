use crate::{ChoiceItem, ChoiceOverlayAction};

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

    /// Ask how the retracted turn should be kept: a new branch (the default)
    /// or by hard-truncating this session's own history.
    pub(crate) fn open_rewind_mode_picker(
        &mut self,
        session_id: i64,
        at_message_id: i64,
        message_document: agena_domain::ComposerDocument,
        target: String,
    ) {
        let items = vec![
            self.rewind_mode_choice(
                "overlay-rewind-mode-fork-label",
                "overlay-rewind-mode-fork-detail",
                "fork",
            ),
            self.rewind_mode_choice(
                "overlay-rewind-mode-in-place-label",
                "overlay-rewind-mode-in-place-detail",
                "in_place",
            ),
        ];
        self.open_choice_overlay(self.build_choice_overlay(
            ui_text::t(&self.i18n, "overlay-rewind-mode-title"),
            self.i18n.text_args(
                "overlay-rewind-mode-prompt",
                &agena_tui::fl_args!("target" => target.clone()),
            ),
            None,
            items,
            ChoiceOverlayAction::RewindMode {
                session_id,
                at_message_id,
                message_document,
                target,
            },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        ));
    }

    fn rewind_mode_choice(&self, label_key: &str, detail_key: &str, value: &str) -> ChoiceItem {
        let label = ui_text::t(&self.i18n, label_key);
        let detail = ui_text::t(&self.i18n, detail_key);
        ChoiceItem {
            search_text: format!("{label} {detail}"),
            label,
            detail,
            value: value.to_owned(),
            current: false,
        }
    }

    pub(crate) fn open_rewind_confirm_overlay(
        &mut self,
        session_id: i64,
        at_message_id: i64,
        message_document: agena_domain::ComposerDocument,
        target: String,
        mode: agena_domain::RewindMode,
    ) {
        let body = match mode {
            agena_domain::RewindMode::Fork => vec![
                self.i18n.text_args(
                    "overlay-rewind-confirm-keep",
                    &agena_tui::fl_args!("target" => target.clone()),
                ),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-warning"),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-draft"),
            ],
            agena_domain::RewindMode::InPlace => vec![
                self.i18n.text_args(
                    "overlay-rewind-confirm-keep",
                    &agena_tui::fl_args!("target" => target.clone()),
                ),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-in-place-warning"),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-in-place-draft"),
            ],
        };
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-rewind-confirm-title"),
            body,
            ConfirmAction::Rewind {
                session_id,
                at_message_id,
                message_document,
                target,
                mode,
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

    pub(crate) fn session_search_item(&self, mut session: SessionResource) -> SessionSearchItem {
        self.session_mark_writes.project(&mut session);
        let mut detail_parts = vec![
            ui_text::session_state_label(&self.i18n, &session.state),
            ui_text::session_meta(
                &self.i18n,
                session.id,
                session.message_count,
                session.updated_at,
            ),
        ];
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
            favorite: session.favorite,
            pinned: session.pinned,
            summary: format!(
                "{} · {}",
                ui_text::session_state_label(&self.i18n, &session.state),
                chrono::DateTime::<chrono::Local>::from(session.updated_at).format("%m-%d %H:%M")
            ),
            label: session.title,
            detail: detail_parts.join(" | "),
        }
    }

    pub(crate) fn rewind_turn_navigation_item(
        &self,
        session_id: i64,
        target: crate::RewindTarget,
    ) -> (
        agena_tui_session::session_navigation::SessionNavigationItem,
        SessionNavigationCommand,
    ) {
        let message_document = target.message_document.clone();
        let normalized = message_document
            .text()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let turn_label = self.i18n.text_args(
            "rewind-turn-label",
            &agena_tui::fl_args!("sequence" => target.sequence),
        );
        let label = if normalized.is_empty() {
            turn_label.clone()
        } else {
            normalized.chars().take(96).collect()
        };
        let detail = self.i18n.text_args(
            "rewind-turn-detail",
            &agena_tui::fl_args!(
                "sequence" => target.sequence,
                "time" => format_timestamp(
                    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(target.created_at_ms)
                        .unwrap_or_default(),
                ),
            ),
        );
        let target_text = format!("{label} ({turn_label})");
        (
            agena_tui_session::session_navigation::SessionNavigationItem::new(
                format!("message:{}", target.at_message_id),
                label.clone(),
                detail.clone(),
                format!("{label} {detail} {}", target.at_message_id),
            ),
            SessionNavigationCommand::Rewind {
                session_id,
                at_message_id: target.at_message_id,
                message_document,
                target: target_text,
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

    pub(crate) fn persist_current_session_model_stack(
        &mut self,
        rollback: RunOptionsState,
    ) -> bool {
        let Some(session_id) = self.transcript.session_id else {
            // No session is open: the run-options model stack is already
            // updated in memory and will be applied to the next session
            // created. Nothing to persist, and this must not be reported as
            // an error so model switching works before a session exists.
            return true;
        };
        let options = self.run_options.model_stack_request();
        self.session_selection_revision = self.session_selection_revision.wrapping_add(1);
        let revision = self.session_selection_revision;
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .update_session_selection(session_id, options)
                    .await
            },
            move |app, result| match result {
                Ok(execution) => {
                    if app.transcript.session_id == Some(session_id) {
                        let _ = app.apply_transcript_execution(execution);
                    }
                    app.request_sessions(false);
                }
                Err(error) => {
                    if app.transcript.session_id == Some(session_id)
                        && app.session_selection_revision == revision
                    {
                        app.run_options = rollback;
                    }
                    app.flash_error(error);
                }
            },
        );
        true
    }

    pub(crate) fn apply_model_override(&mut self, model: ModelRef) -> bool {
        let previous = self.run_options.clone();
        self.run_options
            .replace_model_stack(Some(model.clone()), None, None, None, None);
        if !self.persist_current_session_model_stack(previous.clone()) {
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
        self.transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
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
            // The server's session state machine (`SessionResource.state`) is
            // the single authoritative source: it is derived from persisted
            // run markers and pending interactions by the process that owns
            // the data directory, so it survives disconnects and needs no
            // client-side tracking.
            // Pending interactive asks refine *what* the awaiting is for — a
            // permission ask stays under SessionState::Running (permissions
            // are not user-input interactions), so it is surfaced here rather
            // than folded into Running.
            if let Some(kind) = pending_interactive_kind_for_execution(execution) {
                return match kind {
                    PendingInteractiveKind::Permission => SessionActivity::AwaitingPermission,
                    PendingInteractiveKind::UserInput => SessionActivity::AwaitingInteraction,
                };
            }
            match &execution.session.state {
                agena_api::resource::SessionState::Running { .. }
                | agena_api::resource::SessionState::Creating => {
                    return SessionActivity::Running;
                }
                agena_api::resource::SessionState::AwaitingInteraction { .. } => {
                    return SessionActivity::AwaitingInteraction;
                }
                // A failed session is a recovery/attention state, not active
                // execution. It must never park a composer message behind a
                // completion event that can no longer arrive.
                agena_api::resource::SessionState::Failed { .. } => {
                    return SessionActivity::NeedsRecovery;
                }
                agena_api::resource::SessionState::Ready { .. } => {}
            }
        }

        // No authoritative execution view yet (submit/continue is still in
        // flight before the first SSE refresh). The client-side run tracker
        // covers this short window only.
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

    pub(crate) fn session_is_busy(&self, session_id: i64) -> bool {
        self.session_activity(session_id).is_busy()
    }

    pub(crate) fn current_or_selected_session_title(&self) -> Option<String> {
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
        if self.is_bot_session() {
            return None;
        }
        let execution = self
            .transcript
            .execution
            .as_ref()
            .filter(|execution| self.transcript.session_id == Some(execution.session.id))?;
        execution.session.workspace_id?;
        let effective = execution
            .execution
            .effective_workspace_root
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let path = effective
            .map(str::to_owned)
            .unwrap_or_else(|| self.application.workspace_root().display().to_string());
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
                at_message_id,
                message_document,
                target,
                mode,
            } => self.request_session_rewind(
                session_id,
                at_message_id,
                message_document,
                target,
                mode,
            ),
            ConfirmAction::PermissionStudioDeletePathRule { pattern } => {
                self.delete_permission_studio_path_rule(pattern.as_str())
            }
            ConfirmAction::PermissionStudioDeleteNetworkRule { target } => {
                self.delete_permission_studio_network_rule(target.as_str())
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
        }
    }
}
use crate::{
    App, ConfirmAction, ModelRef, Overlay, PendingInteractiveKind, Route, RunActivityTarget,
    RunOperation, RunOptionsState, SessionActivity, SessionNavigationCommand,
    SessionNavigationQuery, SessionResource, SessionSearchItem, TimelineOverlay, format_timestamp,
    pending_interactive_kind_for_execution, preferred_visible_session_selection, ui_text,
};
use agena_tui::main_focus::Focus;

#[cfg(test)]
mod rewind_mode_tests {
    use crate::{App, ChoiceOverlayAction, ConfirmAction, Overlay, Route};
    use agena_tui::i18n::I18n;
    use agena_tui::main_focus::Focus;

    fn app() -> (tempfile::TempDir, App) {
        let temp = tempfile::tempdir().unwrap();
        let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
        let mut app =
            App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
        app.draft_store_path = temp.path().join("tui-drafts.json");
        app.prompt_history_path = temp.path().join("tui-prompt-history.jsonl");
        app.current_route = Route::Main;
        app.focus = Focus::Composer;
        (temp, app)
    }

    fn document() -> agena_domain::ComposerDocument {
        agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
            text: "retracted turn".into(),
        }])
    }

    fn pick(app: &mut App, value: &str) -> bool {
        let Some(Overlay::Choice(dialog)) = &mut app.overlay else {
            panic!("expected the rewind mode picker");
        };
        assert!(matches!(
            dialog.action,
            ChoiceOverlayAction::RewindMode { .. }
        ));
        let picked = dialog
            .presentation
            .select_item_where(|item| item.value == value);
        if picked {
            app.handle_key_event(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter,
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        picked
    }

    #[test]
    fn rewind_mode_and_in_place_copy_exist_in_every_locale() {
        for (locale, _) in agena_tui::i18n::SUPPORTED_LOCALES {
            let i18n = I18n::resolve(Some(locale), None);
            let prompt = i18n.text_args(
                "overlay-rewind-mode-prompt",
                &agena_tui::fl_args!("target" => "turn 2"),
            );
            assert!(
                prompt.contains("turn 2") && !prompt.contains("Unknown localization key"),
                "{locale}: {prompt}"
            );
            let flash = i18n.text_args(
                "flash-session-rewound-in-place",
                &agena_tui::fl_args!("target" => "turn 2"),
            );
            assert!(
                flash.contains("turn 2") && !flash.contains("Unknown localization key"),
                "{locale}: {flash}"
            );
            for key in [
                "overlay-rewind-mode-title",
                "overlay-rewind-mode-fork-label",
                "overlay-rewind-mode-fork-detail",
                "overlay-rewind-mode-in-place-label",
                "overlay-rewind-mode-in-place-detail",
                "overlay-rewind-confirm-in-place-warning",
                "overlay-rewind-confirm-in-place-draft",
            ] {
                let value = i18n.text(key);
                assert!(
                    !value.trim().is_empty() && !value.contains("Unknown localization key"),
                    "{locale}/{key}: {value}"
                );
            }
        }
    }

    #[test]
    fn rewind_offers_both_branch_and_in_place_outcomes() {
        let (_temp, mut app) = app();
        app.open_rewind_mode_picker(7, 42, document(), "turn 2".into());
        let Some(Overlay::Choice(dialog)) = &app.overlay else {
            panic!("expected the rewind mode picker");
        };
        let values = ["fork", "in_place"]
            .into_iter()
            .filter(|value| {
                let mut presentation = dialog.presentation.clone();
                presentation.select_item_where(|item| item.value == *value)
            })
            .count();
        assert_eq!(values, 2, "rewind must offer a branch and an in-place edit");
    }

    #[test]
    fn choosing_in_place_opens_a_confirm_that_carries_the_mode() {
        let (_temp, mut app) = app();
        app.open_rewind_mode_picker(7, 42, document(), "turn 2".into());
        assert!(pick(&mut app, "in_place"));
        let Some(Overlay::Confirm(confirm)) = &app.overlay else {
            panic!("expected the rewind confirm");
        };
        match &confirm.action {
            ConfirmAction::Rewind {
                session_id,
                at_message_id,
                mode,
                ..
            } => {
                assert_eq!((*session_id, *at_message_id), (7, 42));
                assert_eq!(*mode, agena_domain::RewindMode::InPlace);
            }
            other => panic!("unexpected confirm action: {other:?}"),
        }
        assert!(
            confirm
                .body_lines
                .iter()
                .any(|line| line.contains("cannot be undone")),
            "in-place rewind must warn that it is destructive: {:?}",
            confirm.body_lines
        );
    }

    #[test]
    fn choosing_fork_keeps_the_branch_wording() {
        let (_temp, mut app) = app();
        app.open_rewind_mode_picker(7, 42, document(), "turn 2".into());
        assert!(pick(&mut app, "fork"));
        let Some(Overlay::Confirm(confirm)) = &app.overlay else {
            panic!("expected the rewind confirm");
        };
        match &confirm.action {
            ConfirmAction::Rewind { mode, .. } => {
                assert_eq!(*mode, agena_domain::RewindMode::Fork)
            }
            other => panic!("unexpected confirm action: {other:?}"),
        }
        assert!(
            confirm
                .body_lines
                .iter()
                .any(|line| line.contains("new branch")),
            "fork rewind must describe the branch: {:?}",
            confirm.body_lines
        );
    }

    #[tokio::test]
    async fn in_place_rewind_resets_the_transcript_instead_of_switching_sessions() {
        let (_temp, mut app) = app();
        app.transcript.session_id = Some(17);
        app.transcript.reset(17, "Rewind test".into());
        let state = serde_json::json!({
            "session": {
                "id": 17, "parent_id": null, "root_id": 17, "depth": 0,
                "workspace_id": 1, "title": "Rewind test", "version": 2,
                "relation_kind": "root", "lifecycle_state": "ready",
                "state": {"kind": "ready", "data": {}},
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "message_count": 1, "child_session_count": 0
            },
            "parts": [], "part_page": null,
            "execution": {
                "agent_id": "test", "selected_permission": {},
                "effective_permission": {}, "permission_ceiling": {}
            },
            "usage": { "current_tokens": 0 }
        });
        let execution: crate::SessionExecutionResource = serde_json::from_value(state).unwrap();
        app.handle_session_rewound(
            17,
            document(),
            "turn 2".into(),
            agena_domain::RewindMode::InPlace,
            Ok(execution),
        );
        assert_eq!(
            app.transcript.session_id,
            Some(17),
            "in-place rewind must stay in the same session"
        );
        assert_eq!(app.transcript.execution.as_ref().unwrap().session.id, 17);
        assert!(
            !app.run_activity
                .is_active(crate::RunActivityTarget::Session(17)),
            "rewind activity must be finished"
        );
        assert!(
            !app.transcript_cache.contains_key(&17),
            "retracted cached parts must be dropped"
        );
    }
}
