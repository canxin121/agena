impl App {
    pub(in crate::app) fn handle_line_overlay_key(
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

    pub(in crate::app) fn handle_confirm_overlay_key(
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

    pub(in crate::app) fn handle_user_input_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut UserInputOverlay,
    ) -> bool {
        if Self::user_input_overlay_is_review(dialog) {
            return self.handle_user_input_review_decision_key(key, dialog);
        }
        if dialog.editing_custom {
            return match resolve_tui_key(KeyContext::UserInputQuestion, key) {
                Some(KeyAction::Close) => {
                    dialog.editing_custom = false;
                    false
                }
                Some(KeyAction::Accept) => {
                    let committed = Self::commit_user_input_custom_values(dialog);
                    let should_advance = dialog
                        .request
                        .questions
                        .get(dialog.state.selected_question())
                        .map(|question| committed && !question.multiple)
                        .unwrap_or(false);
                    if should_advance {
                        if Self::user_input_review_hidden(dialog) {
                            return self.submit_user_input_overlay(dialog);
                        }
                        Self::move_user_input_tab(dialog, 1);
                    }
                    false
                }
                Some(KeyAction::CancelRequest) => self.cancel_user_input_overlay(dialog),
                _ => {
                    dialog.custom_input.handle_line_input_key(key);
                    false
                }
            };
        }

        match dialog.state.screen() {
            QuestionFlowScreen::Question => self.handle_user_input_question_key(key, dialog),
            QuestionFlowScreen::Review => self.handle_user_input_review_key(key, dialog),
        }
    }

    pub(in crate::app) fn handle_user_input_review_decision_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut UserInputOverlay,
    ) -> bool {
        let option_count = Self::user_input_review_question(&dialog.request)
            .map(|question| question.options.len())
            .unwrap_or(0);
        match resolve_tui_key(KeyContext::UserInputReview, key) {
            Some(KeyAction::Close) => true,
            Some(KeyAction::Accept) => self.submit_user_input_overlay(dialog),
            Some(KeyAction::CancelRequest) => self.cancel_user_input_overlay(dialog),
            Some(KeyAction::MoveUp) => {
                move_selected_index(&mut dialog.review_option, option_count, -1);
                false
            }
            Some(KeyAction::MoveDown) => {
                move_selected_index(&mut dialog.review_option, option_count, 1);
                false
            }
            Some(KeyAction::PageUp) => {
                dialog.review_scroll = dialog.review_scroll.saturating_sub(12);
                false
            }
            Some(KeyAction::PageDown) => {
                dialog.review_scroll = dialog.review_scroll.saturating_add(12);
                false
            }
            _ => false,
        }
    }

    pub(in crate::app) fn handle_user_input_question_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut UserInputOverlay,
    ) -> bool {
        match resolve_tui_key(KeyContext::UserInputQuestion, key) {
            Some(KeyAction::Close) => true,
            Some(KeyAction::Accept) => self.commit_user_input_question(dialog),
            Some(KeyAction::CancelRequest) => self.cancel_user_input_overlay(dialog),
            Some(KeyAction::MoveUp) => {
                Self::move_user_input_option(dialog, -1);
                false
            }
            Some(KeyAction::MoveDown) => {
                Self::move_user_input_option(dialog, 1);
                false
            }
            Some(KeyAction::NextTab) => {
                Self::move_user_input_tab(dialog, 1);
                false
            }
            Some(KeyAction::PreviousTab) => {
                Self::move_user_input_tab(dialog, -1);
                false
            }
            Some(KeyAction::Toggle) => {
                Self::toggle_user_input_option(dialog);
                false
            }
            Some(KeyAction::Edit) => {
                Self::begin_user_input_custom_edit(dialog);
                false
            }
            Some(KeyAction::Clear) => {
                Self::clear_user_input_answer(dialog);
                false
            }
            _ => false,
        }
    }

    pub(in crate::app) fn handle_user_input_review_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut UserInputOverlay,
    ) -> bool {
        match resolve_tui_key(KeyContext::UserInputReview, key) {
            Some(KeyAction::Close) => true,
            Some(KeyAction::Accept) => self.submit_user_input_overlay(dialog),
            Some(KeyAction::CancelRequest) => self.cancel_user_input_overlay(dialog),
            Some(KeyAction::NextTab) => {
                Self::move_user_input_tab(dialog, 1);
                false
            }
            Some(KeyAction::PreviousTab) => {
                Self::move_user_input_tab(dialog, -1);
                false
            }
            Some(KeyAction::MoveUp) => {
                Self::move_user_input_question(dialog, -1);
                false
            }
            Some(KeyAction::MoveDown) => {
                Self::move_user_input_question(dialog, 1);
                false
            }
            Some(KeyAction::Edit) => {
                Self::focus_user_input_question(dialog, dialog.state.selected_question());
                false
            }
            Some(KeyAction::Clear) => {
                Self::clear_user_input_answer(dialog);
                false
            }
            _ => false,
        }
    }

    pub(in crate::app) fn cancel_user_input_overlay(&mut self, dialog: &UserInputOverlay) -> bool {
        let reply = UserInputReply {
            request_id: dialog.request.request_id.clone(),
            kind: UserInputReplyKind::Cancel,
            answers: BTreeMap::new(),
            reason: None,
        };
        self.request_user_input_reply(dialog.session_id, reply);
        true
    }

    pub(in crate::app) fn submit_user_input_overlay(
        &mut self,
        dialog: &mut UserInputOverlay,
    ) -> bool {
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

    pub(in crate::app) fn commit_user_input_question(
        &mut self,
        dialog: &mut UserInputOverlay,
    ) -> bool {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return false;
        };
        let is_custom = Self::selected_user_input_row_is_custom(dialog, question);
        let multiple = question.multiple;
        if is_custom {
            Self::begin_user_input_custom_edit(dialog);
            return false;
        }
        if multiple {
            Self::move_user_input_tab(dialog, 1);
            return false;
        }
        Self::select_user_input_option(dialog);
        if Self::user_input_review_hidden(dialog) {
            return self.submit_user_input_overlay(dialog);
        }
        Self::move_user_input_tab(dialog, 1);
        false
    }

    pub(in crate::app) fn move_user_input_question(dialog: &mut UserInputOverlay, delta: isize) {
        if dialog.request.questions.is_empty() {
            dialog.state.clear();
            return;
        }
        let review_mode = dialog.state.screen() == QuestionFlowScreen::Review;
        dialog
            .state
            .move_question(dialog.request.questions.len(), delta);
        if review_mode {
            return;
        }
        Self::sync_user_input_option_selection(dialog);
    }

    pub(in crate::app) fn focus_user_input_question(dialog: &mut UserInputOverlay, index: usize) {
        if dialog.request.questions.is_empty() {
            dialog.state.clear();
            return;
        }
        dialog
            .state
            .focus_question(index, dialog.request.questions.len());
        Self::sync_user_input_option_selection(dialog);
    }

    pub(in crate::app) fn sync_user_input_option_selection(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            dialog.state.set_selected_option(0);
            return;
        };
        let row_count = Self::user_input_option_row_count(question);
        if row_count == 0 {
            dialog.state.set_selected_option(0);
            return;
        }
        let preferred = dialog
            .answers
            .get(&question.id)
            .map(|draft| Self::preferred_user_input_option_row(question, draft))
            .unwrap_or(0);
        dialog.state.set_selected_option(preferred);
        dialog.state.clamp_options(row_count);
    }

    pub(in crate::app) fn move_user_input_option(dialog: &mut UserInputOverlay, delta: isize) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return;
        };
        let row_count = Self::user_input_option_row_count(question);
        if row_count == 0 {
            return;
        }
        dialog.state.move_option(row_count, delta);
    }

    pub(in crate::app) fn move_user_input_tab(dialog: &mut UserInputOverlay, delta: isize) {
        if dialog.request.questions.is_empty() {
            dialog.state.clear();
            return;
        }
        if dialog.state.screen() == QuestionFlowScreen::Review {
            if delta < 0 {
                Self::focus_user_input_question(dialog, dialog.state.selected_question());
            } else {
                Self::focus_user_input_question(dialog, 0);
            }
            return;
        }
        let last_index = dialog.request.questions.len().saturating_sub(1);
        if delta < 0 {
            if dialog.state.selected_question() > 0 {
                Self::focus_user_input_question(dialog, dialog.state.selected_question() - 1);
            } else if !Self::user_input_review_hidden(dialog) {
                dialog.state.focus_review(dialog.request.questions.len());
            } else {
                Self::focus_user_input_question(dialog, last_index);
            }
            return;
        }
        if dialog.state.selected_question() < last_index {
            Self::focus_user_input_question(dialog, dialog.state.selected_question() + 1);
            return;
        }
        if !Self::user_input_review_hidden(dialog) {
            dialog.state.focus_review(dialog.request.questions.len());
        } else {
            Self::focus_user_input_question(dialog, 0);
        }
    }

    pub(in crate::app) fn toggle_user_input_option(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return;
        };
        let is_custom = Self::selected_user_input_row_is_custom(dialog, question);
        let allow_custom = question.allow_custom;
        let question_id = question.id.clone();
        let multiple = question.multiple;
        if is_custom || question.options.is_empty() {
            if allow_custom {
                Self::begin_user_input_custom_edit(dialog);
            }
            return;
        }
        let draft = dialog.answers.entry(question_id).or_default();
        if multiple {
            if !draft.option_indexes.insert(dialog.state.selected_option()) {
                draft.option_indexes.remove(&dialog.state.selected_option());
            }
        } else {
            draft.option_indexes.clear();
            draft.option_indexes.insert(dialog.state.selected_option());
            draft.custom_values.clear();
        }
    }

    pub(in crate::app) fn select_user_input_option(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return;
        };
        if Self::selected_user_input_row_is_custom(dialog, question) {
            return;
        }
        let question_id = question.id.clone();
        let draft = dialog.answers.entry(question_id).or_default();
        draft.option_indexes.clear();
        draft.option_indexes.insert(dialog.state.selected_option());
        draft.custom_values.clear();
    }

    pub(in crate::app) fn begin_user_input_custom_edit(dialog: &mut UserInputOverlay) -> bool {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return false;
        };
        let allow_custom = question.allow_custom;
        let selected_option = question.options.len();
        let question_id = question.id.clone();
        if !allow_custom {
            return false;
        }
        dialog.state.focus_question(
            dialog.state.selected_question(),
            dialog.request.questions.len(),
        );
        dialog.state.set_selected_option(selected_option);
        let existing = dialog
            .answers
            .get(&question_id)
            .map(|draft| draft.custom_values.join(", "))
            .unwrap_or_default();
        dialog.custom_input.set_text(existing);
        dialog.editing_custom = true;
        true
    }

    pub(in crate::app) fn commit_user_input_custom_values(dialog: &mut UserInputOverlay) -> bool {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            dialog.editing_custom = false;
            return false;
        };
        let multiple = question.multiple;
        let question_id = question.id.clone();
        let custom_row = question.options.len();
        let parsed = dialog
            .custom_input
            .text()
            .split([',', '\n'])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        let draft = dialog.answers.entry(question_id).or_default();
        draft.custom_values = if multiple {
            parsed
        } else {
            parsed.into_iter().take(1).collect()
        };
        if !draft.custom_values.is_empty() && !multiple {
            draft.option_indexes.clear();
        }
        dialog.state.set_selected_option(custom_row);
        dialog.editing_custom = false;
        !draft.custom_values.is_empty()
    }

    pub(in crate::app) fn clear_user_input_answer(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return;
        };
        dialog.answers.remove(&question.id);
        dialog.custom_input.clear();
        dialog.editing_custom = false;
    }

    pub(in crate::app) fn build_structured_user_input_reply(
        i18n: &I18n,
        dialog: &mut UserInputOverlay,
    ) -> std::result::Result<UserInputReply, String> {
        if let Some(question) = Self::user_input_review_question(&dialog.request) {
            let Some(option) = question.options.get(dialog.review_option) else {
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
                .answers
                .get(&question.id)
                .map(|draft| user_input_answer_values(question, draft))
                .unwrap_or_default();
            if values.is_empty() {
                let label = user_input_question_label(question).to_string();
                Self::focus_user_input_question(dialog, index);
                return Err(i18n.text_args(
                    "overlay-user-input-missing-answer",
                    &crate::fl_args!("label" => label),
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

    pub(in crate::app) fn user_input_review_hidden(dialog: &UserInputOverlay) -> bool {
        dialog.request.questions.len() == 1
            && dialog
                .request
                .questions
                .first()
                .map(|question| !question.multiple)
                .unwrap_or(false)
    }

    pub(in crate::app) fn user_input_option_row_count(question: &UserInputQuestion) -> usize {
        question.options.len() + usize::from(question.allow_custom)
    }

    pub(in crate::app) fn preferred_user_input_option_row(
        question: &UserInputQuestion,
        draft: &UserInputAnswerDraft,
    ) -> usize {
        if let Some(index) = draft.option_indexes.iter().next().copied() {
            return index.min(question.options.len().saturating_sub(1));
        }
        if !draft.custom_values.is_empty() && question.allow_custom {
            return question.options.len();
        }
        0
    }

    pub(in crate::app) fn selected_user_input_row_is_custom(
        dialog: &UserInputOverlay,
        question: &UserInputQuestion,
    ) -> bool {
        question.allow_custom && dialog.state.selected_option() >= question.options.len()
    }
}
use crate::app::{
    App, BTreeMap, ConfirmOverlay, I18n, InputDialogKeyResult, KeyEvent, LineInputOverlay,
    OverlayCommit, QuestionFlowScreen, UserInputAnswerDraft, UserInputOverlay, UserInputQuestion,
    UserInputReply, UserInputReplyKind, drive_input_dialog_key, move_selected_index, ui_text,
    user_input_answer_values, user_input_question_label,
};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
