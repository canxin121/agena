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

    pub(crate) fn handle_user_input_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut UserInputOverlay,
    ) -> bool {
        let page_size = agena_tui::user_input::review_decision_page_size(
            &dialog.presentation,
            &self.i18n,
            self.layout.overlay_area,
        );
        match dialog.presentation.handle_key(key, page_size) {
            agena_tui::user_input::UserInputEffect::Close => true,
            agena_tui::user_input::UserInputEffect::Submit => {
                self.submit_user_input_overlay(dialog)
            }
            agena_tui::user_input::UserInputEffect::Cancel => {
                self.cancel_user_input_overlay(dialog)
            }
            agena_tui::user_input::UserInputEffect::KeepOpen => false,
        }
    }

    pub(crate) fn cancel_user_input_overlay(&mut self, dialog: &UserInputOverlay) -> bool {
        let reply = UserInputReply {
            request_id: dialog.request.request_id.clone(),
            kind: UserInputReplyKind::Cancel,
            answers: BTreeMap::new(),
            reason: None,
        };
        self.request_user_input_reply(dialog.session_id, reply);
        true
    }

    pub(crate) fn submit_user_input_overlay(&mut self, dialog: &mut UserInputOverlay) -> bool {
        match Self::build_structured_user_input_reply(&self.i18n, dialog) {
            Ok(reply) => {
                self.request_user_input_reply(dialog.session_id, reply);
                true
            }
            Err(error) => {
                self.flash_warning(error);
                false
            }
        }
    }

    pub(crate) fn build_structured_user_input_reply(
        i18n: &I18n,
        dialog: &mut UserInputOverlay,
    ) -> std::result::Result<UserInputReply, String> {
        if let Some(question) = Self::user_input_review_question(&dialog.request) {
            let Some(option) = question
                .options
                .get(dialog.presentation.review().selected_option())
            else {
                return Err(ui_text::t(i18n, "overlay-user-input-no-questions"));
            };
            return Ok(UserInputReply {
                request_id: dialog.request.request_id.clone(),
                kind: UserInputReplyKind::Submit,
                answers: BTreeMap::from([(question.id.clone(), vec![option.label.clone()])]),
                reason: None,
            });
        }

        let mut answers = BTreeMap::new();
        for index in 0..dialog.request.questions.len() {
            let question = &dialog.request.questions[index];
            let values = dialog
                .presentation
                .answer(&question.id)
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
            answers.insert(question.id.clone(), values);
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
    OverlayCommit, UserInputOverlay, UserInputReply, UserInputReplyKind, drive_input_dialog_key,
    ui_text, user_input_answer_values, user_input_question_label,
};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
