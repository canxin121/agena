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

    /// Derive the review decision selection from the transcript cursor — the
    /// cursor IS the review cursor — and stage the derived marker on the
    /// interaction views before the renderer draws. Called just before every
    /// render, so every navigation path (j/k, PgUp/PgDn, Space, Ctrl+U/D,
    /// gg/G, h/l, mouse) keeps the decision selection in sync through one call
    /// site. Only invalidates the render cache when the derived value changed,
    /// so plain cursor movement over non-interaction content stays cheap.
    ///
    /// Ask-user parts are skipped entirely: their wizard page and option cursor
    /// are presentation state, rebuilt by [`Self::sync_interaction_documents`]
    /// after every wizard mutation — never overwritten by the cursor.
    pub(crate) fn refresh_interaction_selection(&mut self, width: u16) {
        if self.user_input_interactions.is_empty() {
            return;
        }
        let Some(hit) = self.interaction_cursor_hit(width) else {
            return;
        };
        let Some(mut view) = self.transcript.interaction_views.get(&hit.request_id).cloned() else {
            return;
        };
        let Some(dialog) = self.user_input_interactions.get(&hit.request_id) else {
            return;
        };
        let layouts =
            agena_tui_transcript::interaction_question_layouts(&dialog.request, &view.answers);
        let changed = match hit.line_kind {
            agena_tui_transcript::InteractionLineKind::ReviewOption { option_index } => {
                if view.selected_option == Some(option_index) {
                    false
                } else {
                    view.selected_option = Some(option_index);
                    true
                }
            }
            agena_tui_transcript::InteractionLineKind::ReviewCustomLabel
            | agena_tui_transcript::InteractionLineKind::ReviewEditor => {
                let custom_index = layouts
                    .first()
                    .map(|layout| layout.options_len)
                    .unwrap_or(0);
                if view.selected_option == Some(custom_index) {
                    false
                } else {
                    view.selected_option = Some(custom_index);
                    true
                }
            }
            _ => false,
        };
        if changed {
            self.transcript
                .interaction_views
                .insert(hit.request_id, view);
            self.transcript.invalidate_render();
        }
    }

    /// Derive the review decision hit under the transcript cursor. The
    /// transcript cursor IS the review cursor: the body offset and row kind
    /// come from where the cursor sits on the rendered part, using the exact
    /// layout arithmetic the renderer draws. Ask-user parts return `None` —
    /// the wizard is presentation-driven, not cursor-row-driven.
    fn interaction_cursor_hit(&mut self, width: u16) -> Option<InteractionCursorHit> {
        let node = self.transcript.current_cursor_node_cloned(width)?;
        if !node.expanded {
            return None;
        }
        let request_id = self.interaction_request_id_for_node_key(&node.key)?;
        let cursor_line = self.transcript.navigation_cursor_line()?;
        if cursor_line < node.start_line || cursor_line >= node.end_line {
            return None;
        }
        let dialog = self.user_input_interactions.get(&request_id)?;
        if !agena_tui_transcript::request_is_review_decision(&dialog.request) {
            return None;
        }
        let view = self.transcript.interaction_views.get(&request_id)?;
        // Body offset: the headline row precedes the body.
        let body_offset = cursor_line.saturating_sub(node.start_line).saturating_sub(1);
        let editing_custom = dialog.presentation.review().is_editing_custom();
        let layouts =
            agena_tui_transcript::interaction_question_layouts(&dialog.request, &view.answers);
        let line_kind = agena_tui_transcript::classify_interaction_line(
            &layouts,
            view.plan_body_lines,
            body_offset,
            editing_custom,
        );
        Some(InteractionCursorHit {
            request_id,
            line_kind,
        })
    }

    /// Route a transcript key into the active pending user-input interaction.
    /// Unlike the old pre-step hijack this is a **thin context-aware action
    /// layer**. Plan review only intercepts `Enter` on a decision row,
    /// `Ctrl+X` to cancel, `e` on the custom-feedback label, and `Esc` to
    /// collapse — every other key falls through to the normal transcript
    /// dispatch. Ask-user is a paged wizard: while the cursor is on the
    /// expanded part, Up/Down/Left/Right/Enter/Esc drive the presentation page
    /// and option cursor, and everything else still falls through (the chat
    /// keeps owning paging and navigation — "everything is a part", no
    /// injected review component).
    pub(crate) fn handle_active_interaction_action(&mut self, key: KeyEvent) -> bool {
        if self.overlay.is_some()
            || self.focus != Focus::Transcript
            || !self.current_route_is_main()
        {
            return false;
        }
        // While the inline custom editor is open it owns the whole key stream.
        if self.interaction_editing.is_some() {
            return self.handle_interaction_edit_key(key);
        }
        // The transcript cursor marks which pending part is active; the row
        // kind below derives the review decision, or the ask-user wizard takes
        // over the arrows/Enter for the whole part.
        let Some(request_id) = self.active_user_input_interaction_request_id() else {
            return false;
        };
        let review = self
            .user_input_interactions
            .get(&request_id)
            .is_some_and(|dialog| {
                agena_tui_transcript::request_is_review_decision(&dialog.request)
            });
        // Ctrl+X cancels the request; it is unbound in the Transcript context,
        // so it is safe to own it while the cursor is on a pending part.
        if matches!(key.code, KeyCode::Char('x')) && key.modifiers == KeyModifiers::CONTROL {
            self.cancel_active_interaction(&request_id);
            return true;
        }
        if !review {
            // Ask-user is a paged wizard: arrows/Enter drive the presentation
            // page + option cursor, not the transcript cursor.
            return self.handle_ask_user_wizard_key(&request_id, key);
        }
        // Review: the transcript cursor IS the decision cursor — the hit
        // derives the active request and the row kind from where the cursor
        // sits on the rendered decision rows.
        let Some(hit) = self.interaction_cursor_hit(self.layout.transcript_body.width) else {
            return false;
        };
        match resolve_tui_key(KeyContext::Transcript, key) {
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
                // `e` opens the inline custom-feedback editor ("展开即可交互").
                self.begin_interaction_custom_edit(&hit.request_id);
                true
            }
            Some(KeyAction::CancelSelection)
                if self.transcript_motion_prefix.is_none()
                    && !self.transcript_yank_pending
                    && !self.transcript_goto_pending
                    && !self.transcript_viewport_pending =>
            {
                // Esc with no pending motion prefix collapses the part back to
                // its configured state; the request stays reachable by
                // re-expanding the part.
                self.collapse_active_interaction(&hit.request_id);
                true
            }
            _ => false,
        }
    }

    /// Route a key while the cursor is on an expanded pending ask-user part
    /// (the paged wizard). Up/Down move the option cursor within the current
    /// question, Left/Right switch question pages without committing (and
    /// reach the summary page), Space toggles the option under the cursor,
    /// and Enter submits the current page and advances (submitting the whole
    /// request on the summary page). Esc is deliberately NOT owned: it falls
    /// through to the normal transcript dispatch (the wizard never collapses
    /// the part out from under the user). Every other key — including `h`/`l`
    /// on non-part text and all chat paging — falls through too.
    fn handle_ask_user_wizard_key(&mut self, request_id: &str, key: KeyEvent) -> bool {
        if key.modifiers != KeyModifiers::NONE {
            return false;
        }
        match key.code {
            KeyCode::Up => {
                self.wizard_move_option(request_id, -1);
                true
            }
            KeyCode::Down => {
                self.wizard_move_option(request_id, 1);
                true
            }
            KeyCode::Left => {
                self.wizard_move_tab(request_id, -1);
                true
            }
            KeyCode::Right => {
                self.wizard_move_tab(request_id, 1);
                true
            }
            KeyCode::Char(' ') => {
                self.wizard_toggle_option(request_id);
                true
            }
            KeyCode::Enter => {
                self.wizard_enter(request_id);
                true
            }
            _ => false,
        }
    }

    /// Move the wizard's option cursor within the current question page.
    fn wizard_move_option(&mut self, request_id: &str, delta: isize) {
        let Some(mut dialog) = self.user_input_interactions.remove(request_id) else {
            return;
        };
        dialog.presentation.move_option(delta);
        self.user_input_interactions.insert(request_id.to_string(), dialog);
        self.sync_interaction_documents();
    }

    /// Switch the wizard to the previous/next question page, or into/out of
    /// the final summary page.
    fn wizard_move_tab(&mut self, request_id: &str, delta: isize) {
        let Some(mut dialog) = self.user_input_interactions.remove(request_id) else {
            return;
        };
        dialog.presentation.move_wizard_tab(delta);
        self.user_input_interactions.insert(request_id.to_string(), dialog);
        self.sync_interaction_documents();
    }

    /// Space on a wizard question page: toggles the option/custom row under
    /// the wizard's option cursor (opening the inline custom editor on the
    /// custom row). The page stays put — selection never advances the flow.
    fn wizard_toggle_option(&mut self, request_id: &str) {
        let Some(mut dialog) = self.user_input_interactions.remove(request_id) else {
            return;
        };
        dialog.presentation.toggle_option();
        if dialog.presentation.is_editing_custom() {
            // The presentation opened the inline custom editor; the app takes
            // ownership of the key stream.
            self.interaction_editing = Some(request_id.to_string());
        }
        self.user_input_interactions.insert(request_id.to_string(), dialog);
        self.sync_interaction_documents();
    }

    /// Enter on a wizard page: on a question page, submits the current page's
    /// answer and advances to the next page (or the summary); on the summary
    /// page, submits the whole request — jumping to the unanswered question's
    /// page on a validation miss. The summary page has no Submit row and no
    /// locked cursor, so Enter anywhere on it submits.
    fn wizard_enter(&mut self, request_id: &str) {
        let Some(mut dialog) = self.user_input_interactions.remove(request_id) else {
            return;
        };
        if dialog.presentation.screen()
            == agena_tui::user_input::QuestionFlowScreen::Review
        {
            match Self::build_structured_user_input_reply(&self.i18n, &mut dialog, None) {
                Ok(reply) => {
                    let session_id = dialog.session_id;
                    self.request_user_input_reply(session_id, reply);
                    // The dialog is gone from the map; rebuild the views so the
                    // part stops rendering the (now stale) summary page.
                    self.sync_interaction_documents();
                    return;
                }
                Err(error) => {
                    // Keep the dialog so the user can correct the missing
                    // answer; `focus_question` moved the page there, and the
                    // view rebuild below lands on it.
                    self.flash_warning(error);
                }
            }
        } else {
            // Question page: commit the page (Space already wrote the draft)
            // and advance to the next page — the summary is reached from the
            // last question page, and it is the only submit surface.
            dialog.presentation.move_wizard_tab(1);
        }
        self.user_input_interactions.insert(request_id.to_string(), dialog);
        self.sync_interaction_documents();
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
        // effects (sending the reply, beginning the editor) without a live
        // borrow on the map. Reinserted unless the reply was sent.
        let Some(mut dialog) = self.user_input_interactions.remove(request_id) else {
            return;
        };
        let mut keep = true;
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
                        keep = false;
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
                    keep = false;
                }
            }
            _ => {}
        }
        if keep {
            self.user_input_interactions.insert(request_id.to_string(), dialog);
        }
        self.sync_interaction_documents();
    }

    /// Cancel the pending user-input interaction and send the cancellation
    /// reply. The dialog stays in the map until the reply is acknowledged
    /// (handlers.rs clears it), so the part keeps rendering as pending.
    fn cancel_active_interaction(&mut self, request_id: &str) {
        let Some(dialog) = self.user_input_interactions.get(request_id) else {
            return;
        };
        let session_id = dialog.session_id;
        let reply = UserInputReply {
            request_id: dialog.request.request_id.clone(),
            kind: UserInputReplyKind::Cancel,
            answers: BTreeMap::new(),
            reason: None,
        };
        self.request_user_input_reply(session_id, reply);
    }

    /// Collapse the pending interaction part back to its configured state; the
    /// request stays reachable by re-expanding the part.
    fn collapse_active_interaction(&mut self, request_id: &str) {
        let Some(node_key) = self.pending_interaction_part_node_key(request_id) else {
            return;
        };
        self.transcript.node_expansions.insert(node_key, false);
        self.transcript.invalidate_render();
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
        let effect = dialog.presentation.handle_custom_edit_key(key);
        match effect {
            agena_tui::user_input::UserInputEffect::Close => {
                // Esc exits editing back to the part (cursor stays on the
                // custom row); the request stays pending.
                self.interaction_editing = None;
                self.user_input_interactions.insert(request_id, dialog);
            }
            agena_tui::user_input::UserInputEffect::Submit => {
                // Enter committed non-empty custom feedback: submit the reply.
                self.interaction_editing = None;
                if let Ok(reply) =
                    Self::build_structured_user_input_reply(&self.i18n, &mut dialog, None)
                {
                    let session_id = dialog.session_id;
                    self.request_user_input_reply(session_id, reply);
                } else {
                    self.user_input_interactions.insert(request_id, dialog);
                }
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
        true
    }

    /// Insert pasted text into the custom-feedback field of the active
    /// expanded pending user-input interaction, if any.
    pub(crate) fn paste_into_active_interaction(&mut self, text: &str) -> bool {
        let Some(request_id) = self.active_user_input_interaction_request_id() else {
            return false;
        };
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
            if !custom_text.is_empty() {
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
            if values.is_empty() {
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
    App, BTreeMap, ConfirmOverlay, I18n, InputDialogKeyResult, KeyEvent, LineInputOverlay,
    OverlayCommit, TranscriptContentId, TranscriptNodeKey, UserInputOverlay, UserInputReply,
    UserInputReplyKind, drive_input_dialog_key, ui_text, user_input_answer_values,
    user_input_question_label,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;
use crossterm::event::{KeyCode, KeyModifiers};

/// A hit on the shared "transcript cursor is on an expanded pending
/// interaction part" computation: the derived body offset and the semantic
/// kind of the row under the cursor. Both the selection sync and the key
/// router derive from this one path so they cannot drift.
struct InteractionCursorHit {
    request_id: String,
    line_kind: agena_tui_transcript::InteractionLineKind,
}
