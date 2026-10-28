impl App {
    pub(crate) fn handle_line_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut LineInputOverlay,
        commit: OverlayCommit,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(_, value) => {
                let value = value.trim().to_string();
                match commit {
                    OverlayCommit::TranscriptSearch => {
                        self.transcript.set_search_query(value);
                        self.jump_search_match(self.transcript_search_forward);
                    }
                }
                true
            }
            InputDialogKeyResult::Continue => false,
        }
    }

    pub(crate) fn handle_confirm_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ConfirmOverlay,
    ) -> bool {
        match resolve_tui_key(KeyContext::Confirm, key) {
            Some(KeyAction::Close) => true,
            Some(KeyAction::Confirm) => {
                self.handle_confirm_action(dialog.action.clone());
                true
            }
            _ => false,
        }
    }

    /// The `request_id` of the pending user-input interaction the transcript
    /// cursor currently sits on, when that node is an expanded pending
    /// interaction part. This is the "展开即可交互" boundary: expanded pending
    /// parts are the interaction surface.
    pub(crate) fn active_user_input_interaction_request_id(&mut self) -> Option<String> {
        let width = self.layout.transcript_body.width;
        let node = self.transcript.current_cursor_node_cloned(width)?;
        if !node.expanded {
            return None;
        }
        self.interaction_request_id_for_node_key(&node.key)
    }

    /// Maps an Activity node key to the pending interaction `request_id` it
    /// renders, when the node is a pending user-input interaction part.
    fn interaction_request_id_for_node_key(&self, key: &TranscriptNodeKey) -> Option<String> {
        let content_id = match key {
            TranscriptNodeKey::Activity { content_id, .. } => content_id,
            _ => return None,
        };
        let TranscriptContentId::StoredPart(part_id) = content_id else {
            return None;
        };
        agena_tui_transcript::parts_entries(&self.transcript.parts)
            .into_iter()
            .flat_map(|entry| entry.parts.into_iter())
            .find_map(|part| {
                (part.id == TranscriptContentId::StoredPart(*part_id))
                    .then(|| {
                        agena_tui_transcript::interaction_request_id_for_part(&part)
                            .map(str::to_owned)
                    })
                    .flatten()
            })
    }

    /// Derive the review decision hit under the transcript cursor. The
    /// renderer records the action on every wrapped row of a control.
    fn interaction_cursor_hit(&mut self, width: u16) -> Option<InteractionCursorHit> {
        let control = self.transcript.control_at_cursor(width)?;
        let agena_tui_transcript::TranscriptControlAction::UserInput { request_id, kind } =
            control.action
        else {
            return None;
        };
        let dialog = self.user_input_interactions.get(&request_id)?;
        agena_tui_transcript::request_is_review_decision(&dialog.request).then_some(
            InteractionCursorHit {
                request_id,
                line_kind: kind,
            },
        )
    }

    /// Actions belong to the control under the transcript cursor. All motion
    /// uses the shared text/control cursor. Questionnaire controls reserve
    /// horizontal keys for pages; vertical motion can still leave the part.
    pub(crate) fn handle_active_interaction_action(&mut self, key: KeyEvent) -> bool {
        if self.overlay.is_some()
            || self.focus != Focus::Transcript
            || !self.current_route_is_main()
        {
            return false;
        }
        if self.handle_inline_transcript_key(key) {
            return true;
        }
        if self.interaction_editing.is_some() {
            return self.handle_interaction_edit_key(key);
        }
        if !self.transcript.has_visual_selection()
            && !self.transcript_yank_pending
            && self.handle_inline_permission_key(key)
        {
            return true;
        }
        let Some(request_id) = self.active_user_input_interaction_request_id() else {
            return false;
        };
        if matches!(key.code, KeyCode::Char('x')) && key.modifiers == KeyModifiers::CONTROL {
            self.cancel_active_interaction(&request_id);
            return true;
        }
        if self.transcript.has_visual_selection()
            || self.transcript_yank_pending
            || self.transcript_goto_pending
            || self.transcript_viewport_pending
            || self.transcript_find_pending.is_some()
            || self.transcript_text_object_pending.is_some()
        {
            return false;
        }
        let review = self
            .user_input_interactions
            .get(&request_id)
            .is_some_and(|dialog| {
                agena_tui_transcript::request_is_review_decision(&dialog.request)
            });
        let width = self.layout.transcript_body.width;
        if key.code == KeyCode::Esc && key.modifiers.is_empty() {
            self.transcript
                .focus_part_headline(width, self.layout.transcript_body.height);
            return true;
        }
        if self.interaction_reply_in_flight(&request_id)
            && self
                .transcript
                .control_at_cursor(width)
                .is_some_and(|control| {
                    matches!(control.action,
                    agena_tui_transcript::TranscriptControlAction::UserInput { request_id: id, .. }
                        if id == request_id)
                })
        {
            // A deferred request retains its draft until acknowledgement. It
            // may still be browsed, but cannot send a competing answer.
            return key.modifiers.is_empty()
                && (matches!(key.code, KeyCode::Enter | KeyCode::Char(' '))
                    || (review && key.code == KeyCode::Char('e')));
        }
        if !review {
            return self.handle_ask_user_key(key);
        }
        let Some(hit) = self.interaction_cursor_hit(width) else {
            return false;
        };
        match resolve_tui_key(KeyContext::Transcript, key) {
            Some(KeyAction::Toggle)
                if hit.line_kind == agena_tui_transcript::InteractionLineKind::AskCancel =>
            {
                self.cancel_active_interaction(&hit.request_id);
                true
            }
            Some(KeyAction::Toggle) if hit.line_kind.is_submit_eligible() => {
                self.handle_decision_row_enter(&hit.request_id, hit.line_kind);
                true
            }
            Some(KeyAction::WordEnd)
                if matches!(
                    hit.line_kind,
                    agena_tui_transcript::InteractionLineKind::ReviewCustomLabel
                ) =>
            {
                self.begin_interaction_custom_edit(&hit.request_id);
                true
            }
            _ => false,
        }
    }

    fn handle_inline_permission_key(&mut self, key: KeyEvent) -> bool {
        use agena_tui_transcript::TranscriptControlAction;
        let width = self.layout.transcript_body.width;
        let Some(control) = self.transcript.control_at_cursor(width) else {
            return false;
        };
        let TranscriptControlAction::Permission {
            request_id,
            decision,
        } = control.action
        else {
            return false;
        };
        if key.code == KeyCode::Esc && key.modifiers.is_empty() {
            self.transcript
                .focus_part_headline(width, self.layout.transcript_body.height);
            return true;
        }
        if key.code != KeyCode::Enter || !key.modifiers.is_empty() {
            return false;
        }
        let pending = self
            .transcript
            .execution
            .as_ref()
            .and_then(|execution| {
                execution
                    .session
                    .state
                    .pending_interactive_requests()
                    .iter()
                    .find(|pending| {
                        pending
                            .request
                            .as_permission()
                            .is_some_and(|request| request.request_id == request_id)
                    })
            })
            .map(|pending| pending.session_id);
        let Some(session_id) = pending else {
            return true;
        };
        let request = agena_tui_transcript::parts_entries(&self.transcript.parts)
            .into_iter()
            .flat_map(|entry| entry.parts.into_iter())
            .find_map(|part| match part.content {
                agena_tui_transcript::TranscriptPartContent::Activity(
                    agena_tui_transcript::TranscriptActivityContent::Operation(tool),
                ) => tool
                    .operation
                    .authorization
                    .permissions
                    .iter()
                    .find(|record| {
                        record.reply.is_none() && record.request.request_id == request_id
                    })
                    .map(|record| record.request.clone()),
                _ => None,
            });
        let Some(request) = request else {
            return true;
        };
        if decision == Some(crate::PermissionReplyKind::AllowAlways) {
            let mut dialog = self.build_permission_overlay(session_id, request);
            dialog
                .presentation
                .open_scope(agena_tui::permission_prompt::PermissionPromptDecision::Allow);
            self.overlay = Some(Overlay::Permission(dialog));
        } else if let Some(kind) = decision {
            self.submit_permission_reply(
                session_id,
                request,
                kind,
                None,
                ui_text::permission_reply_label(&self.i18n, kind),
            );
        } else {
            let mut dialog = self.build_permission_overlay(session_id, request);
            let _ = dialog.presentation.open_details();
            self.overlay = Some(Overlay::Permission(dialog));
        }
        true
    }

    fn handle_ask_user_key(&mut self, key: KeyEvent) -> bool {
        let Some(hit) = self.ask_user_cursor_hit(self.layout.transcript_body.width) else {
            return false;
        };
        if !key.modifiers.is_empty() {
            return false;
        }
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => self.ask_move_question(&hit.request_id, -1),
            KeyCode::Right | KeyCode::Char('l') => self.ask_move_question(&hit.request_id, 1),
            KeyCode::Char(' ') => self.ask_space(&hit.request_id, hit.line_kind),
            KeyCode::Enter => self.ask_enter(&hit.request_id, hit.line_kind),
            _ => false,
        }
    }

    /// Horizontal keys browse questions without changing answers or submitting.
    /// The optional summary remains an explicit action on the last page.
    fn ask_move_question(&mut self, request_id: &str, delta: isize) -> bool {
        let Some(dialog) = self.user_input_interactions.get_mut(request_id) else {
            return false;
        };
        let count = dialog.request.questions.len();
        if count == 0 {
            return true;
        }
        let summary =
            dialog.presentation.screen() == agena_tui::user_input::QuestionFlowScreen::Review;
        let page = if summary {
            count
        } else {
            dialog.presentation.selected_question()
        };
        if summary && delta > 0 {
            return true;
        }
        let target = (page as isize + delta).clamp(0, count.saturating_sub(1) as isize) as usize;
        if target != page {
            dialog.presentation.focus_question(target);
            self.sync_interaction_documents();
            self.ask_focus_page(request_id);
        }
        true
    }

    /// Derive the ask-user row under the transcript cursor — the cursor IS the
    /// option cursor on the current page. Review parts return `None`; the
    /// renderer records the question/action identity on the visible rows.
    fn ask_user_cursor_hit(&mut self, width: u16) -> Option<InteractionCursorHit> {
        let control = self.transcript.control_at_cursor(width)?;
        let agena_tui_transcript::TranscriptControlAction::UserInput { request_id, kind } =
            control.action
        else {
            return None;
        };
        let dialog = self.user_input_interactions.get(&request_id)?;
        (!agena_tui_transcript::request_is_review_decision(&dialog.request)).then_some(
            InteractionCursorHit {
                request_id,
                line_kind: kind,
            },
        )
    }

    /// Change the page without discarding any answers. Boundaries do not wrap.
    fn ask_jump_question(
        &mut self,
        request_id: &str,
        _line_kind: agena_tui_transcript::InteractionLineKind,
        delta: isize,
    ) -> bool {
        let Some(dialog) = self.user_input_interactions.get_mut(request_id) else {
            return false;
        };
        let before = (
            dialog.presentation.screen(),
            dialog.presentation.selected_question(),
        );
        dialog.presentation.move_wizard_tab(delta);
        if before
            != (
                dialog.presentation.screen(),
                dialog.presentation.selected_question(),
            )
        {
            self.sync_interaction_documents();
            self.ask_focus_page(request_id);
        }
        true
    }

    /// Space on an ask option row toggles that option; Space on the 其他 row
    /// opens its inline custom editor (the editor takes ownership of the key
    /// stream). Any other row falls through to the normal PageDown dispatch.
    fn ask_space(
        &mut self,
        request_id: &str,
        line_kind: agena_tui_transcript::InteractionLineKind,
    ) -> bool {
        use agena_tui_transcript::InteractionLineKind;
        match line_kind {
            InteractionLineKind::AskOption {
                question_index,
                option_index,
            } => {
                let Some(mut dialog) = self.user_input_interactions.remove(request_id) else {
                    return false;
                };
                dialog
                    .presentation
                    .toggle_option_index(question_index, option_index);
                self.user_input_interactions
                    .insert(request_id.to_string(), dialog);
                self.sync_interaction_documents();
                true
            }
            InteractionLineKind::AskCustomRow { question_index } => {
                let Some(mut dialog) = self.user_input_interactions.remove(request_id) else {
                    return false;
                };
                let Some(option_index) = dialog
                    .request
                    .questions
                    .get(question_index)
                    .and_then(|question| question.custom_option_index())
                else {
                    self.user_input_interactions
                        .insert(request_id.to_string(), dialog);
                    return false;
                };
                dialog
                    .presentation
                    .toggle_option_index(question_index, option_index);
                if dialog.presentation.is_editing_custom() {
                    self.interaction_editing = Some(request_id.to_string());
                }
                self.user_input_interactions
                    .insert(request_id.to_string(), dialog);
                self.sync_interaction_documents();
                true
            }
            _ => false,
        }
    }

    /// Confirm the existing answer without selecting the focused option.
    /// The last question submits directly; earlier questions advance.
    fn ask_confirm_question(&mut self, request_id: &str, question_index: usize) -> bool {
        let Some(dialog) = self.user_input_interactions.get_mut(request_id) else {
            return false;
        };
        let Some(question) = dialog.request.questions.get(question_index) else {
            return false;
        };
        let answered = dialog.presentation.question_is_answered(question_index);
        if !answered {
            let label = user_input_question_label(question).to_owned();
            if dialog.presentation.custom_option_selected(question_index)
                && dialog.presentation.begin_custom_edit_for(question_index)
            {
                self.interaction_editing = Some(request_id.to_string());
            }
            self.flash_warning(self.i18n.text_args(
                "overlay-user-input-missing-answer",
                &agena_tui::fl_args!("label" => label),
            ));
            self.sync_interaction_documents();
            return true;
        }
        if question_index + 1 == dialog.request.questions.len() {
            return self.ask_enter(
                request_id,
                agena_tui_transcript::InteractionLineKind::AskSubmit,
            );
        }
        dialog.presentation.focus_question(question_index + 1);
        self.sync_interaction_documents();
        self.ask_focus_page(request_id);
        true
    }

    /// Explicit navigation, editing, confirmation and submission share the
    /// same paths for pointer and keyboard activation.
    fn ask_enter(
        &mut self,
        request_id: &str,
        line_kind: agena_tui_transcript::InteractionLineKind,
    ) -> bool {
        use agena_tui_transcript::InteractionLineKind::*;
        match line_kind {
            AskPrevious => return self.ask_jump_question(request_id, line_kind, -1),
            AskNext => {
                let Some(dialog) = self.user_input_interactions.get(request_id) else {
                    return false;
                };
                let index = dialog.presentation.selected_question();
                if index + 1 < dialog.request.questions.len() {
                    return self.ask_confirm_question(request_id, index);
                }
                return self.ask_jump_question(request_id, line_kind, 1);
            }
            AskCancel => {
                self.cancel_active_interaction(request_id);
                return true;
            }
            AskCustomRow { question_index } | AskOption { question_index, .. } => {
                return self.ask_confirm_question(request_id, question_index);
            }
            AskSummary { .. } | AskSubmit => {}
            _ => return false,
        }
        let Some(mut dialog) = self.user_input_interactions.remove(request_id) else {
            return false;
        };
        match line_kind {
            AskSummary { question_index } => dialog.presentation.focus_question(question_index),
            AskSubmit => {
                match Self::build_structured_user_input_reply(&self.i18n, &mut dialog, None) {
                    Ok(reply) => {
                        let session_id = dialog.session_id;
                        self.user_input_interactions
                            .insert(request_id.to_owned(), dialog);
                        self.request_user_input_reply(session_id, reply);
                        self.sync_interaction_documents();
                        return true;
                    }
                    Err(error) => self.flash_warning(error),
                }
            }
            _ => unreachable!("guarded above"),
        }
        self.user_input_interactions
            .insert(request_id.to_string(), dialog);
        self.sync_interaction_documents();
        self.ask_focus_page(request_id);
        true
    }

    /// Page mutations land on an actual rendered control, even when long
    /// questions or option descriptions wrap to several terminal rows.
    fn ask_focus_page(&mut self, request_id: &str) {
        self.transcript.focus_request_control(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            request_id,
        );
    }

    /// Enter on a decision row of the pending interaction part. The line kind
    /// was derived from the transcript cursor, so the choice is already known
    /// — the part natively owns the decision.
    fn handle_decision_row_enter(
        &mut self,
        request_id: &str,
        line_kind: agena_tui_transcript::InteractionLineKind,
    ) {
        use agena_tui_transcript::InteractionLineKind;
        // Take the dialog out of the map so the arms can drive `&mut self`
        // effects without a live borrow on the map. Keep the draft until the
        // server acknowledges the reply so a failed send remains editable.
        let Some(mut dialog) = self.user_input_interactions.remove(request_id) else {
            return;
        };
        match line_kind {
            InteractionLineKind::ReviewOption { option_index } => {
                match Self::build_structured_user_input_reply(
                    &self.i18n,
                    &mut dialog,
                    Some(option_index),
                ) {
                    Ok(reply) => {
                        let session_id = dialog.session_id;
                        self.request_user_input_reply(session_id, reply);
                    }
                    Err(error) => self.flash_warning(error),
                }
            }
            InteractionLineKind::ReviewCustomLabel | InteractionLineKind::ReviewEditor => {
                if dialog.presentation.review().custom_text().is_empty() {
                    // No feedback typed yet: Enter opens the inline editor.
                    if dialog.presentation.begin_review_custom_edit() {
                        self.interaction_editing = Some(request_id.to_string());
                    }
                } else if let Ok(reply) =
                    Self::build_structured_user_input_reply(&self.i18n, &mut dialog, None)
                {
                    let session_id = dialog.session_id;
                    self.request_user_input_reply(session_id, reply);
                }
            }
            _ => {}
        }
        self.user_input_interactions
            .insert(request_id.to_string(), dialog);
        self.sync_interaction_documents();
    }

    fn interaction_reply_in_flight(&self, request_id: &str) -> bool {
        self.user_input_interactions
            .get(request_id)
            .is_some_and(|dialog| {
                self.run_activity.has_operation(
                    crate::RunActivityTarget::Session(dialog.session_id),
                    crate::RunOperation::UserInputReply,
                )
            })
    }

    /// Cancel the pending user-input interaction and send the cancellation
    /// reply. The dialog stays in the map until the reply is acknowledged
    /// (handlers.rs clears it), so the part keeps rendering as pending.
    fn cancel_active_interaction(&mut self, request_id: &str) {
        let Some(dialog) = self.user_input_interactions.get(request_id) else {
            return;
        };
        let session_id = dialog.session_id;
        if self.interaction_reply_in_flight(request_id) {
            return;
        }
        let reply = UserInputReply {
            request_id: dialog.request.request_id.clone(),
            kind: UserInputReplyKind::Cancel,
            answers: BTreeMap::new(),
            reason: None,
        };
        self.request_user_input_reply(session_id, reply);
    }

    /// Open the inline custom-feedback editor for the pending interaction part
    /// and give the editor ownership of the transcript key stream.
    fn begin_interaction_custom_edit(&mut self, request_id: &str) {
        let Some(dialog) = self.user_input_interactions.get_mut(request_id) else {
            return;
        };
        if !dialog.presentation.begin_review_custom_edit() {
            return;
        }
        self.interaction_editing = Some(request_id.to_string());
        self.sync_interaction_documents();
    }

    /// Drive the inline custom editor while it owns the key stream. The
    /// editor itself routes through the presentation's custom-key handlers,
    /// which already resolve Esc/Enter/Ctrl+X to the right lifecycle. The
    /// editor keeps ownership only while the presentation is still editing;
    /// an Enter-on-empty (which exits editing without submitting) hands the
    /// stream back too.
    fn handle_interaction_edit_key(&mut self, key: KeyEvent) -> bool {
        let Some(request_id) = self.interaction_editing.clone() else {
            return false;
        };
        let Some(mut dialog) = self.user_input_interactions.remove(&request_id) else {
            self.interaction_editing = None;
            return false;
        };
        let was_ask = !dialog.presentation.is_review_decision();
        let request_key = request_id.clone();
        let effect = dialog.presentation.handle_custom_edit_key(key);
        match effect {
            agena_tui::user_input::UserInputEffect::Close => {
                // Esc exits editing back to the part (cursor stays on the
                // custom row); the request stays pending.
                self.interaction_editing = None;
                self.user_input_interactions.insert(request_id, dialog);
            }
            agena_tui::user_input::UserInputEffect::Submit => {
                // Enter on final-question text or review feedback submits.
                self.interaction_editing = None;
                match Self::build_structured_user_input_reply(&self.i18n, &mut dialog, None) {
                    Ok(reply) => {
                        let session_id = dialog.session_id;
                        self.request_user_input_reply(session_id, reply);
                    }
                    Err(error) => self.flash_warning(error),
                }
                self.user_input_interactions.insert(request_id, dialog);
            }
            agena_tui::user_input::UserInputEffect::Cancel => {
                self.interaction_editing = None;
                let session_id = dialog.session_id;
                let reply = UserInputReply {
                    request_id: dialog.request.request_id.clone(),
                    kind: UserInputReplyKind::Cancel,
                    answers: BTreeMap::new(),
                    reason: None,
                };
                self.request_user_input_reply(session_id, reply);
                self.user_input_interactions.insert(request_id, dialog);
            }
            agena_tui::user_input::UserInputEffect::KeepOpen => {
                // The editor may still be open (typing) or may have just
                // exited on an empty Enter; re-derive ownership from the
                // presentation instead of assuming.
                let still_editing = if dialog.presentation.is_review_decision() {
                    dialog.presentation.review().is_editing_custom()
                } else {
                    dialog.presentation.is_editing_custom()
                };
                if !still_editing {
                    self.interaction_editing = None;
                }
                self.user_input_interactions.insert(request_id, dialog);
            }
        }
        self.sync_interaction_documents();
        if was_ask && self.interaction_editing.is_none() {
            self.ask_focus_page(&request_key);
        }
        true
    }

    /// Insert pasted text into the custom-feedback field of the active
    /// expanded pending user-input interaction, if any.
    pub(crate) fn paste_into_active_interaction(&mut self, text: &str) -> bool {
        let Some(request_id) = self.active_user_input_interaction_request_id() else {
            return false;
        };
        if self.interaction_reply_in_flight(&request_id) {
            return true;
        }
        let Some(dialog) = self.user_input_interactions.get_mut(&request_id) else {
            return false;
        };
        if !dialog.presentation.insert_custom_text(text) {
            return false;
        }
        // The presentation auto-begins the review edit on insert; take
        // ownership of the key stream so further typing lands in the editor.
        self.interaction_editing = Some(request_id);
        self.sync_interaction_documents();
        true
    }

    /// Build the structured user-input reply. For plan-review requests the
    /// option under the transcript cursor is the choice: `review_selection`
    /// carries the cursor-derived option index when the caller knows it
    /// (native part path); `None` falls back to the presentation's own cursor
    /// (overlay path / tests).
    pub(crate) fn build_structured_user_input_reply(
        i18n: &I18n,
        dialog: &mut UserInputOverlay,
        review_selection: Option<usize>,
    ) -> std::result::Result<UserInputReply, String> {
        if let Some(question) = Self::user_input_review_question(&dialog.request) {
            let custom_text = dialog.presentation.review().custom_text();
            if review_selection.is_none() && !custom_text.is_empty() {
                return Ok(UserInputReply {
                    request_id: dialog.request.request_id.clone(),
                    kind: UserInputReplyKind::Submit,
                    answers: BTreeMap::from([("0".to_string(), vec![custom_text])]),
                    reason: None,
                });
            }
            let selected =
                review_selection.unwrap_or_else(|| dialog.presentation.review().selected_option());
            let Some(option) = question.options.get(selected) else {
                return Err(ui_text::t(i18n, "overlay-user-input-review-feedback-empty"));
            };
            return Ok(UserInputReply {
                request_id: dialog.request.request_id.clone(),
                kind: UserInputReplyKind::Submit,
                answers: BTreeMap::from([("0".to_string(), vec![option.label.clone()])]),
                reason: None,
            });
        }

        let mut answers = BTreeMap::new();
        for index in 0..dialog.request.questions.len() {
            let question = &dialog.request.questions[index];
            let values = dialog
                .presentation
                .answer(index)
                .map(|draft| user_input_answer_values(question, draft))
                .unwrap_or_default();
            if values.is_empty() || !dialog.presentation.question_is_answered(index) {
                let label = user_input_question_label(question).to_string();
                dialog.presentation.focus_question(index);
                return Err(i18n.text_args(
                    "overlay-user-input-missing-answer",
                    &agena_tui::fl_args!("label" => label),
                ));
            }
            answers.insert(index.to_string(), values);
        }

        Ok(UserInputReply {
            request_id: dialog.request.request_id.clone(),
            kind: UserInputReplyKind::Submit,
            answers,
            reason: None,
        })
    }
}
use crate::{
    App, BTreeMap, ConfirmOverlay, I18n, InputDialogKeyResult, KeyEvent, LineInputOverlay, Overlay,
    OverlayCommit, TranscriptContentId, TranscriptNodeKey, UserInputOverlay, UserInputReply,
    UserInputReplyKind, drive_input_dialog_key, ui_text, user_input_answer_values,
    user_input_question_label,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;
use crossterm::event::{KeyCode, KeyModifiers};

/// An actual rendered action under the shared transcript cursor.
struct InteractionCursorHit {
    request_id: String,
    line_kind: agena_tui_transcript::InteractionLineKind,
}
