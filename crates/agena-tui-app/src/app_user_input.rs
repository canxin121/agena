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
        let content_id = match &node.key {
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

    /// Route a transcript key into the active pending user-input interaction,
    /// driving the same `UserInputPresentation` state machine the overlay
    /// used. Runs before normal transcript dispatch; returns whether the key
    /// was consumed by the interaction.
    pub(crate) fn handle_active_interaction_node_key(&mut self, key: KeyEvent) -> bool {
        if self.overlay.is_some()
            || self.focus != Focus::Transcript
            || !self.current_route_is_main()
        {
            return false;
        }
        let Some(request_id) = self.active_user_input_interaction_request_id() else {
            return false;
        };
        // Resolve the node key up front: the mutable dialog borrow below
        // would otherwise conflict with the shared borrow this call needs.
        let node_key = self.pending_interaction_part_node_key(&request_id);
        // Take the dialog out of the map so the arms can drive `&mut self`
        // effects (sending the reply) without a live borrow on the map.
        let Some(mut dialog) = self.user_input_interactions.remove(&request_id) else {
            return false;
        };
        let page_size = agena_tui::user_input::review_decision_page_size(
            &dialog.presentation,
            &self.i18n,
            self.layout.transcript_body,
        );
        match dialog.presentation.handle_key(key, page_size) {
            agena_tui::user_input::UserInputEffect::Close => {
                // ESC collapses the part back to its configured state; the
                // request stays reachable by re-expanding the part.
                if let Some(node_key) = node_key {
                    self.transcript.node_expansions.insert(node_key, false);
                }
                self.transcript.invalidate_render();
                self.user_input_interactions.insert(request_id, dialog);
            }
            agena_tui::user_input::UserInputEffect::Submit => {
                match Self::build_structured_user_input_reply(&self.i18n, &mut dialog) {
                    Ok(reply) => {
                        let session_id = dialog.session_id;
                        self.request_user_input_reply(session_id, reply);
                    }
                    Err(error) => {
                        // Keep the dialog so the user can correct the missing
                        // answer; `focus_question` moved the cursor there.
                        self.user_input_interactions.insert(request_id, dialog);
                        self.flash_warning(error);
                    }
                }
            }
            agena_tui::user_input::UserInputEffect::Cancel => {
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
                self.user_input_interactions.insert(request_id, dialog);
            }
        }
        // Any keystroke can move the selection or start/stop custom editing;
        // rebuild the inline document so the transcript reflects the new
        // cursor position and markers.
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
        self.sync_interaction_documents();
        true
    }

    pub(crate) fn build_structured_user_input_reply(
        i18n: &I18n,
        dialog: &mut UserInputOverlay,
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
            let Some(option) = question
                .options
                .get(dialog.presentation.review().selected_option())
            else {
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
