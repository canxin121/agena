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
        let Some(mut view) = self
            .transcript
            .interaction_views
            .get(&hit.request_id)
            .cloned()
        else {
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
        let body_offset = cursor_line
            .saturating_sub(node.start_line)
            .saturating_sub(1);
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
            // Ask-user is a continuous body: the transcript cursor IS the
            // option cursor. Left/Right, Space and Enter are owned only
            // because the cursor sits on the part; Up/Down, `e`, paging and
            // everything else fall through to the normal transcript dispatch.
            return self.handle_ask_user_key(key);
        }
        // Review: the transcript cursor IS the decision cursor — the hit
        // derives the active request and the row kind from where the cursor
        // sits on the rendered decision rows.
        let Some(hit) = self.interaction_cursor_hit(self.layout.transcript_body.width) else {
            return false;
        };
        // While the review part is pending, the decision cursor stays confined
        // INSIDE the expanded part block: `j`/`k` (and the arrow keys) are
        // owned so the cursor roams only across the plan body and decision
        // rows, never off onto the message header's role-label column or a
        // neighbouring part. Plain motions only — a pending operator or
        // prefix (`3j`, `yj`, `g`, `z`, search, visual selection) still falls
        // through to the normal transcript dispatch.
        let action = resolve_tui_key(KeyContext::Transcript, key);
        if matches!(action, Some(KeyAction::MoveUp | KeyAction::MoveDown))
            && self.transcript_motion_prefix.is_none()
            && !self.transcript_yank_pending
            && !self.transcript_goto_pending
            && !self.transcript_viewport_pending
            && self.transcript_find_pending.is_none()
            && self.transcript_text_object_pending.is_none()
            && !self.transcript.has_visual_selection()
        {
            let width = self.layout.transcript_body.width;
            let height = self.layout.transcript_body.height;
            let node = self
                .transcript
                .current_cursor_node_cloned(width)
                .expect("the hit derived a cursor node");
            let direction = if action == Some(KeyAction::MoveUp) {
                TranscriptMoveDirection::Up
            } else {
                TranscriptMoveDirection::Down
            };
            let count = self.transcript_motion_count();
            let Some(cursor_line) = self.transcript.navigation_cursor_line() else {
                return false;
            };
            // Walk the requested number of rows, stopping at the part
            // boundary: the headline (part top) is the closest row above, the
            // last decision/footer row the closest row below.
            let mut target = cursor_line;
            for _ in 0..count {
                let next = match direction {
                    TranscriptMoveDirection::Up => target.saturating_sub(1),
                    TranscriptMoveDirection::Down => target.saturating_add(1),
                };
                if next < node.start_line || next >= node.end_line {
                    break;
                }
                target = next;
            }
            if target != cursor_line {
                self.transcript
                    .move_cursor_to_visual_line_number(width, height, Some(target + 1));
                self.transcript.invalidate_render();
            }
            return true;
        }
        match action {
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
    /// (the continuous body). Up/Down and `j`/`k` are NOT owned — the
    /// transcript cursor is a normal line-by-line cursor and can always leave
    /// the part (this is the two-cursor bug fix). Left/Right jump the cursor
    /// to the previous/next question's first option row; Space toggles the
    /// option under the cursor (or opens the inline custom editor on the 其他
    /// row); Enter checks the option and submits the request, jumping to the
    /// first unanswered question on a validation miss. `e`, Esc, PgUp/PgDn,
    /// Shift+Space and Ctrl+f are deliberately NOT owned: they fall through to
    /// the normal transcript dispatch.
    fn handle_ask_user_key(&mut self, key: KeyEvent) -> bool {
        let Some(hit) = self.ask_user_cursor_hit(self.layout.transcript_body.width) else {
            return false;
        };
        if key.modifiers != KeyModifiers::NONE {
            return false;
        }
        // Left/Right are owned only for plain motions: a pending operator or
        // prefix (`3l`, `yj`, `g`, `z`, search, visual selection) still falls
        // through, mirroring the review Up/Down guard.
        let plain_motion = self.transcript_motion_prefix.is_none()
            && !self.transcript_yank_pending
            && !self.transcript_goto_pending
            && !self.transcript_viewport_pending
            && self.transcript_find_pending.is_none()
            && self.transcript_text_object_pending.is_none()
            && !self.transcript.has_visual_selection();
        if plain_motion {
            match resolve_tui_key(KeyContext::Transcript, key) {
                Some(KeyAction::MoveLeft) => {
                    return self.ask_jump_question(&hit.request_id, hit.line_kind, -1);
                }
                Some(KeyAction::MoveRight) => {
                    return self.ask_jump_question(&hit.request_id, hit.line_kind, 1);
                }
                _ => {}
            }
        }
        // Space is matched on the RAW key (never via the keymap, which maps an
        // unmodified Space to PageDown) so a plain space toggles while PgDn /
        // Ctrl+f / Shift+Space still page normally. Enter is raw too.
        match key.code {
            KeyCode::Char(' ') => self.ask_space(&hit.request_id, hit.line_kind),
            KeyCode::Enter => self.ask_enter(&hit.request_id, hit.line_kind),
            _ => false,
        }
    }

    /// Derive the ask-user row under the transcript cursor — the cursor IS the
    /// option cursor in the continuous body. Review parts return `None`; the
    /// ask classifier maps the cursor's body offset to a concrete row kind.
    fn ask_user_cursor_hit(&mut self, width: u16) -> Option<InteractionCursorHit> {
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
        if agena_tui_transcript::request_is_review_decision(&dialog.request) {
            return None;
        }
        let view = self.transcript.interaction_views.get(&request_id)?;
        let body_offset = cursor_line
            .saturating_sub(node.start_line)
            .saturating_sub(1);
        let editing_custom = dialog.presentation.is_editing_custom();
        let layouts =
            agena_tui_transcript::interaction_question_layouts(&dialog.request, &view.answers);
        let line_kind = agena_tui_transcript::classify_ask_user_line(
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

    /// Jump the transcript cursor to the previous/next question's first option
    /// row (or its header row when the question has no options). No-op at the
    /// first/last question, but the key is still consumed while the cursor is
    /// on the part.
    fn ask_jump_question(
        &mut self,
        request_id: &str,
        line_kind: agena_tui_transcript::InteractionLineKind,
        delta: isize,
    ) -> bool {
        use agena_tui_transcript::InteractionLineKind;
        // The current question comes from the cursor's row kind; plan,
        // separator and footer rows sit "before Q0".
        let current = match line_kind {
            InteractionLineKind::AskQuestionHeader { question_index }
            | InteractionLineKind::AskQuestionText { question_index }
            | InteractionLineKind::AskOption { question_index, .. }
            | InteractionLineKind::AskCustomRow { question_index }
            | InteractionLineKind::AskCustomEditor { question_index }
            | InteractionLineKind::AskCustomDetail { question_index }
            | InteractionLineKind::AskAnsweredPreview { question_index } => question_index as isize,
            _ => -1,
        };
        let Some(dialog) = self.user_input_interactions.get(request_id) else {
            return false;
        };
        let question_count = dialog.request.questions.len() as isize;
        let target = current + delta;
        if target < 0 || target >= question_count {
            return true;
        }
        let target = target as usize;
        let Some(view) = self.transcript.interaction_views.get(request_id).cloned() else {
            return false;
        };
        let layouts =
            agena_tui_transcript::interaction_question_layouts(&dialog.request, &view.answers);
        let body_offset = agena_tui_transcript::ask_user_question_landing_offset(
            view.plan_body_lines,
            &layouts,
            target,
        );
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            return false;
        };
        // `node.start_line` is 0-indexed; `move_cursor_to_visual_line_number`
        // is 1-indexed and the body starts one row below the headline.
        self.transcript.move_cursor_to_visual_line_number(
            width,
            height,
            Some(node.start_line + 2 + body_offset),
        );
        self.transcript.invalidate_render();
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
                if dialog.presentation.begin_custom_edit_for(question_index) {
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

    /// Enter on an ask option row checks that option and submits the whole
    /// request when every question is answered; otherwise it does not submit
    /// and moves the cursor to the first unanswered question. Enter on the 其他
    /// row submits its committed custom text, or opens the inline editor when
    /// empty. Any other row falls through to the normal Toggle dispatch (the
    /// part collapses, exactly like review's plan-row Enter).
    fn ask_enter(
        &mut self,
        request_id: &str,
        line_kind: agena_tui_transcript::InteractionLineKind,
    ) -> bool {
        use agena_tui_transcript::InteractionLineKind;
        if !matches!(
            line_kind,
            InteractionLineKind::AskOption { .. } | InteractionLineKind::AskCustomRow { .. }
        ) {
            return false;
        }
        let Some(mut dialog) = self.user_input_interactions.remove(request_id) else {
            return false;
        };
        match line_kind {
            InteractionLineKind::AskOption {
                question_index,
                option_index,
            } => {
                dialog
                    .presentation
                    .commit_option_index(question_index, option_index);
            }
            InteractionLineKind::AskCustomRow { question_index } => {
                let answered = dialog
                    .presentation
                    .answer(question_index)
                    .is_some_and(|draft| !draft.custom_values.is_empty());
                if !answered {
                    // No custom text yet: Enter opens the inline editor.
                    if dialog.presentation.begin_custom_edit_for(question_index) {
                        self.interaction_editing = Some(request_id.to_string());
                    }
                    self.user_input_interactions
                        .insert(request_id.to_string(), dialog);
                    self.sync_interaction_documents();
                    return true;
                }
            }
            _ => unreachable!("guarded above"),
        }
        match Self::build_structured_user_input_reply(&self.i18n, &mut dialog, None) {
            Ok(reply) => {
                let session_id = dialog.session_id;
                self.request_user_input_reply(session_id, reply);
                // Reply sent: the dialog stays out of the map and the views are
                // rebuilt so the part stops rendering the pending body.
                self.sync_interaction_documents();
                true
            }
            Err(error) => {
                // Keep the dialog so the user can correct the missing answer;
                // `build_structured_user_input_reply` focused the first
                // unanswered question on the miss.
                self.flash_warning(error);
                self.user_input_interactions
                    .insert(request_id.to_string(), dialog);
                self.sync_interaction_documents();
                self.ask_focus_unanswered(request_id);
                true
            }
        }
    }

    /// Move the transcript cursor onto the presentation's selected question
    /// (the first unanswered one after a validation miss), landing on its
    /// first option row.
    fn ask_focus_unanswered(&mut self, request_id: &str) {
        let Some(dialog) = self.user_input_interactions.get(request_id) else {
            return;
        };
        let target = dialog.presentation.selected_question();
        let Some(view) = self.transcript.interaction_views.get(request_id).cloned() else {
            return;
        };
        let layouts =
            agena_tui_transcript::interaction_question_layouts(&dialog.request, &view.answers);
        let body_offset = agena_tui_transcript::ask_user_question_landing_offset(
            view.plan_body_lines,
            &layouts,
            target,
        );
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            return;
        };
        self.transcript.move_cursor_to_visual_line_number(
            width,
            height,
            Some(node.start_line + 2 + body_offset),
        );
        self.transcript.invalidate_render();
    }

    /// The transcript cursor line when it sits inside an expanded pending
    /// ask-user part, for the whole-line cursor highlight. Review parts return
    /// `None` — they keep the normal single-grapheme cursor.
    pub(crate) fn active_ask_part_cursor_line(&mut self, width: u16) -> Option<usize> {
        let node = self.transcript.current_cursor_node_cloned(width)?;
        if !node.expanded {
            return None;
        }
        let request_id = self.interaction_request_id_for_node_key(&node.key)?;
        let dialog = self.user_input_interactions.get(&request_id)?;
        if agena_tui_transcript::request_is_review_decision(&dialog.request) {
            return None;
        }
        let cursor_line = self.transcript.navigation_cursor_line()?;
        (cursor_line >= node.start_line && cursor_line < node.end_line).then_some(cursor_line)
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
            self.user_input_interactions
                .insert(request_id.to_string(), dialog);
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
    OverlayCommit, TranscriptContentId, TranscriptMoveDirection, TranscriptNodeKey,
    UserInputOverlay, UserInputReply, UserInputReplyKind, drive_input_dialog_key, ui_text,
    user_input_answer_values, user_input_question_label,
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
