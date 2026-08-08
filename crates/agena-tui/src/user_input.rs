//! Presentation state and reduction for interactive user-input overlays.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Borders, ListItem},
};
use tui_markdown::from_str as markdown_to_text;

use crate::i18n::I18n;
use crate::keymap::{KeyAction, KeyContext, resolve};
use agena_tui_components::{
    Editor, EditorPanelSpec, EditorSection, ListPanelSection, ListPanelSpec, ParagraphSection,
    QuestionFlowCustomInputSpec, QuestionFlowDialogMode, QuestionFlowDialogSpec,
    QuestionFlowScreen, QuestionFlowState, StackedDialogSection, StackedDialogSectionHeight,
    StackedDialogSpec, SurfaceMode, TextPanelSection, TextPanelSpec,
    build_detail_two_line_list_item, list_panel_height, render_question_flow_dialog,
    render_stacked_dialog, wrapped_text_height_for_text,
};

/// A display-only option in an interactive question. Domain request mapping,
/// validation, reply construction, and submission remain outside the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInputOptionPresentation {
    pub label: String,
    pub description: String,
    pub preview_markdown: String,
}

/// A display-only interactive question used by the TUI reducer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInputQuestionPresentation {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<UserInputOptionPresentation>,
    pub multiple: bool,
    pub allow_custom: bool,
}

/// Request-level display data. The App maps the Domain request into this
/// opaque presentation value before the TUI renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInputOverlayPresentation {
    pub request_id: String,
    pub title: String,
    pub body_markdown: String,
    pub submit_label: String,
    pub cancel_label: String,
    pub auto_resolution_ms: Option<u64>,
    pub created_at_ms: i64,
    pub review_decision: bool,
}

/// The presentation-only answer draft for one interactive question.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserInputAnswerDraft {
    pub option_indexes: BTreeSet<usize>,
    pub custom_values: Vec<String>,
}

/// Presentation-only state for a single-question review decision.
#[derive(Debug, Clone, Default)]
pub struct UserInputReviewPresentation {
    selected_option: usize,
    scroll: u16,
    custom_input: Editor,
    editing_custom: bool,
}

impl UserInputReviewPresentation {
    pub fn selected_option(&self) -> usize {
        self.selected_option
    }

    pub fn scroll(&self) -> u16 {
        self.scroll
    }

    pub fn custom_input(&self) -> &Editor {
        &self.custom_input
    }

    pub fn is_editing_custom(&self) -> bool {
        self.editing_custom
    }

    /// The trimmed free-text feedback the user typed, empty when none.
    pub fn custom_text(&self) -> String {
        self.custom_input.text().trim().to_string()
    }
}

/// The complete TUI state for a user-input overlay. It intentionally owns no
/// Domain request or Runtime effect: the App maps request data in, validates a
/// submission, and maps these terminal intents to its concrete effects.
#[derive(Debug, Clone)]
pub struct UserInputPresentation {
    overlay: UserInputOverlayPresentation,
    questions: Vec<UserInputQuestionPresentation>,
    answers: BTreeMap<String, UserInputAnswerDraft>,
    state: QuestionFlowState,
    editing_custom: bool,
    custom_input: Editor,
    review: UserInputReviewPresentation,
    review_decision: bool,
}

impl UserInputPresentation {
    pub fn new(
        overlay: UserInputOverlayPresentation,
        questions: Vec<UserInputQuestionPresentation>,
    ) -> Self {
        Self {
            review_decision: overlay.review_decision,
            overlay,
            questions,
            answers: BTreeMap::new(),
            state: QuestionFlowState::default(),
            editing_custom: false,
            custom_input: Editor::default(),
            review: UserInputReviewPresentation::default(),
        }
    }

    pub fn overlay(&self) -> &UserInputOverlayPresentation {
        &self.overlay
    }

    pub fn questions(&self) -> &[UserInputQuestionPresentation] {
        &self.questions
    }

    pub fn answers(&self) -> &BTreeMap<String, UserInputAnswerDraft> {
        &self.answers
    }

    pub fn answer(&self, question_id: &str) -> Option<&UserInputAnswerDraft> {
        self.answers.get(question_id)
    }

    pub fn screen(&self) -> QuestionFlowScreen {
        self.state.screen()
    }

    pub fn selected_question(&self) -> usize {
        self.state.selected_question()
    }

    pub fn selected_option(&self) -> usize {
        self.state.selected_option()
    }

    pub fn is_editing_custom(&self) -> bool {
        self.editing_custom
    }

    pub fn custom_input(&self) -> &Editor {
        &self.custom_input
    }

    pub fn review(&self) -> &UserInputReviewPresentation {
        &self.review
    }

    pub fn is_review_decision(&self) -> bool {
        self.review_decision
    }

    pub fn review_is_hidden(&self) -> bool {
        self.questions.len() == 1
            && self
                .questions
                .first()
                .map(|question| !question.multiple)
                .unwrap_or(false)
    }

    pub fn focus_question(&mut self, index: usize) {
        if self.questions.is_empty() {
            self.state.clear();
            return;
        }
        self.state.focus_question(index, self.questions.len());
        self.sync_option_selection();
    }

    pub fn insert_custom_text(&mut self, text: &str) -> bool {
        if self.state.screen() == QuestionFlowScreen::Review {
            self.focus_question(self.state.selected_question());
        }
        if !self.editing_custom && !self.begin_custom_edit() {
            return false;
        }
        self.custom_input.insert_str(text);
        true
    }

    pub fn handle_key(&mut self, key: KeyEvent, page_size: usize) -> UserInputEffect {
        if self.review_decision {
            return self.handle_review_decision_key(key, page_size);
        }
        if self.editing_custom {
            return self.handle_custom_input_key(key);
        }
        match self.state.screen() {
            QuestionFlowScreen::Question => self.handle_question_key(key),
            QuestionFlowScreen::Review => self.handle_review_key(key),
        }
    }

    fn handle_review_decision_key(&mut self, key: KeyEvent, page_size: usize) -> UserInputEffect {
        if self.review.editing_custom {
            return self.handle_review_custom_key(key);
        }
        let option_count = self
            .questions
            .first()
            .map(|question| question.options.len() + usize::from(question.allow_custom))
            .unwrap_or(0);
        match resolve(KeyContext::UserInputReview, key) {
            Some(KeyAction::Close) => UserInputEffect::Close,
            Some(KeyAction::Accept) => {
                if self.review_selected_row_is_custom() {
                    if self.review.custom_text().is_empty() {
                        self.begin_review_custom_edit();
                        UserInputEffect::KeepOpen
                    } else {
                        UserInputEffect::Submit
                    }
                } else {
                    UserInputEffect::Submit
                }
            }
            Some(KeyAction::CancelRequest) => UserInputEffect::Cancel,
            Some(KeyAction::MoveUp) => {
                move_selected_index(&mut self.review.selected_option, option_count, -1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::MoveDown) => {
                move_selected_index(&mut self.review.selected_option, option_count, 1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::PageUp) => {
                self.review.scroll = self.review.scroll.saturating_sub(page_size.max(1) as u16);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::PageDown) => {
                self.review.scroll = self.review.scroll.saturating_add(page_size.max(1) as u16);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Edit) => {
                if self.review_selected_row_is_custom() {
                    self.begin_review_custom_edit();
                }
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Clear) => {
                self.review.custom_input.clear();
                UserInputEffect::KeepOpen
            }
            _ => UserInputEffect::KeepOpen,
        }
    }

    fn review_selected_row_is_custom(&self) -> bool {
        let Some(question) = self.questions.first() else {
            return false;
        };
        question.allow_custom && self.review.selected_option >= question.options.len()
    }

    fn begin_review_custom_edit(&mut self) {
        let Some(question) = self.questions.first() else {
            return;
        };
        if !question.allow_custom {
            return;
        }
        self.review.selected_option = question.options.len();
        self.review.editing_custom = true;
    }

    fn handle_review_custom_key(&mut self, key: KeyEvent) -> UserInputEffect {
        match resolve(KeyContext::UserInputQuestion, key) {
            Some(KeyAction::Close) => {
                self.review.editing_custom = false;
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Accept) => {
                let has_text = !self.review.custom_input.text().trim().is_empty();
                self.review.editing_custom = false;
                if has_text {
                    UserInputEffect::Submit
                } else {
                    UserInputEffect::KeepOpen
                }
            }
            Some(KeyAction::CancelRequest) => UserInputEffect::Cancel,
            _ => {
                self.review.custom_input.handle_line_input_key(key);
                UserInputEffect::KeepOpen
            }
        }
    }

    fn handle_custom_input_key(&mut self, key: KeyEvent) -> UserInputEffect {
        match resolve(KeyContext::UserInputQuestion, key) {
            Some(KeyAction::Close) => {
                self.editing_custom = false;
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Accept) => {
                let committed = self.commit_custom_values();
                let should_advance = self
                    .questions
                    .get(self.state.selected_question())
                    .map(|question| committed && !question.multiple)
                    .unwrap_or(false);
                if !should_advance {
                    return UserInputEffect::KeepOpen;
                }
                if self.review_is_hidden() {
                    UserInputEffect::Submit
                } else {
                    self.move_tab(1);
                    UserInputEffect::KeepOpen
                }
            }
            Some(KeyAction::CancelRequest) => UserInputEffect::Cancel,
            _ => {
                self.custom_input.handle_line_input_key(key);
                UserInputEffect::KeepOpen
            }
        }
    }

    fn handle_question_key(&mut self, key: KeyEvent) -> UserInputEffect {
        match resolve(KeyContext::UserInputQuestion, key) {
            Some(KeyAction::Close) => UserInputEffect::Close,
            Some(KeyAction::Accept) => self.commit_question(),
            Some(KeyAction::CancelRequest) => UserInputEffect::Cancel,
            Some(KeyAction::MoveUp) => {
                self.move_option(-1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::MoveDown) => {
                self.move_option(1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::NextTab) => {
                self.move_tab(1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::PreviousTab) => {
                self.move_tab(-1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Toggle) => {
                self.toggle_option();
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Edit) => {
                self.begin_custom_edit();
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Clear) => {
                self.clear_answer();
                UserInputEffect::KeepOpen
            }
            _ => UserInputEffect::KeepOpen,
        }
    }

    fn handle_review_key(&mut self, key: KeyEvent) -> UserInputEffect {
        match resolve(KeyContext::UserInputReview, key) {
            Some(KeyAction::Close) => UserInputEffect::Close,
            Some(KeyAction::Accept) => UserInputEffect::Submit,
            Some(KeyAction::CancelRequest) => UserInputEffect::Cancel,
            Some(KeyAction::NextTab) => {
                self.move_tab(1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::PreviousTab) => {
                self.move_tab(-1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::MoveUp) => {
                self.move_question(-1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::MoveDown) => {
                self.move_question(1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Edit) => {
                self.focus_question(self.state.selected_question());
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Clear) => {
                self.clear_answer();
                UserInputEffect::KeepOpen
            }
            _ => UserInputEffect::KeepOpen,
        }
    }

    fn commit_question(&mut self) -> UserInputEffect {
        let (is_custom, multiple) = {
            let Some(question) = self.questions.get(self.state.selected_question()) else {
                return UserInputEffect::KeepOpen;
            };
            (self.selected_row_is_custom(question), question.multiple)
        };
        if is_custom {
            self.begin_custom_edit();
            return UserInputEffect::KeepOpen;
        }
        if multiple {
            self.move_tab(1);
            return UserInputEffect::KeepOpen;
        }
        self.select_option();
        if self.review_is_hidden() {
            UserInputEffect::Submit
        } else {
            self.move_tab(1);
            UserInputEffect::KeepOpen
        }
    }

    fn move_question(&mut self, delta: isize) {
        if self.questions.is_empty() {
            self.state.clear();
            return;
        }
        let review_mode = self.state.screen() == QuestionFlowScreen::Review;
        self.state.move_question(self.questions.len(), delta);
        if !review_mode {
            self.sync_option_selection();
        }
    }

    fn sync_option_selection(&mut self) {
        let Some(question) = self.questions.get(self.state.selected_question()) else {
            self.state.set_selected_option(0);
            return;
        };
        let row_count = option_row_count(question);
        if row_count == 0 {
            self.state.set_selected_option(0);
            return;
        }
        let preferred = self
            .answers
            .get(&question.id)
            .map(|draft| preferred_option_row(question, draft))
            .unwrap_or(0);
        self.state.set_selected_option(preferred);
        self.state.clamp_options(row_count);
    }

    fn move_option(&mut self, delta: isize) {
        let Some(question) = self.questions.get(self.state.selected_question()) else {
            return;
        };
        let row_count = option_row_count(question);
        if row_count != 0 {
            self.state.move_option(row_count, delta);
        }
    }

    fn move_tab(&mut self, delta: isize) {
        if self.questions.is_empty() {
            self.state.clear();
            return;
        }
        if self.state.screen() == QuestionFlowScreen::Review {
            if delta < 0 {
                self.focus_question(self.state.selected_question());
            } else {
                self.focus_question(0);
            }
            return;
        }
        let last_index = self.questions.len().saturating_sub(1);
        if delta < 0 {
            if self.state.selected_question() > 0 {
                self.focus_question(self.state.selected_question() - 1);
            } else if !self.review_is_hidden() {
                self.state.focus_review(self.questions.len());
            } else {
                self.focus_question(last_index);
            }
        } else if self.state.selected_question() < last_index {
            self.focus_question(self.state.selected_question() + 1);
        } else if !self.review_is_hidden() {
            self.state.focus_review(self.questions.len());
        } else {
            self.focus_question(0);
        }
    }

    fn toggle_option(&mut self) {
        let (is_custom, question_id, allow_custom, multiple, option_count) = {
            let Some(question) = self.questions.get(self.state.selected_question()) else {
                return;
            };
            (
                self.selected_row_is_custom(question),
                question.id.clone(),
                question.allow_custom,
                question.multiple,
                question.options.len(),
            )
        };
        if is_custom || option_count == 0 {
            if allow_custom {
                self.begin_custom_edit();
            }
            return;
        }
        let draft = self.answers.entry(question_id).or_default();
        if multiple {
            if !draft.option_indexes.insert(self.state.selected_option()) {
                draft.option_indexes.remove(&self.state.selected_option());
            }
        } else {
            draft.option_indexes.clear();
            draft.option_indexes.insert(self.state.selected_option());
            draft.custom_values.clear();
        }
    }

    fn select_option(&mut self) {
        let (is_custom, question_id) = {
            let Some(question) = self.questions.get(self.state.selected_question()) else {
                return;
            };
            (self.selected_row_is_custom(question), question.id.clone())
        };
        if is_custom {
            return;
        }
        let draft = self.answers.entry(question_id).or_default();
        draft.option_indexes.clear();
        draft.option_indexes.insert(self.state.selected_option());
        draft.custom_values.clear();
    }

    fn begin_custom_edit(&mut self) -> bool {
        let Some(question) = self.questions.get(self.state.selected_question()) else {
            return false;
        };
        let allow_custom = question.allow_custom;
        let question_id = question.id.clone();
        let selected_option = question.options.len();
        if !allow_custom {
            return false;
        }
        self.state
            .focus_question(self.state.selected_question(), self.questions.len());
        self.state.set_selected_option(selected_option);
        let existing = self
            .answers
            .get(&question_id)
            .map(|draft| draft.custom_values.join(", "))
            .unwrap_or_default();
        self.custom_input.set_text(existing);
        self.editing_custom = true;
        true
    }

    fn commit_custom_values(&mut self) -> bool {
        let (question_id, multiple, custom_row) = {
            let Some(question) = self.questions.get(self.state.selected_question()) else {
                self.editing_custom = false;
                return false;
            };
            (
                question.id.clone(),
                question.multiple,
                question.options.len(),
            )
        };
        let parsed = self
            .custom_input
            .text()
            .split([',', '\n'])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        let draft = self.answers.entry(question_id).or_default();
        draft.custom_values = if multiple {
            parsed
        } else {
            parsed.into_iter().take(1).collect()
        };
        if !draft.custom_values.is_empty() && !multiple {
            draft.option_indexes.clear();
        }
        self.state.set_selected_option(custom_row);
        self.editing_custom = false;
        !draft.custom_values.is_empty()
    }

    fn clear_answer(&mut self) {
        let Some(question_id) = self
            .questions
            .get(self.state.selected_question())
            .map(|question| question.id.clone())
        else {
            return;
        };
        self.answers.remove(&question_id);
        self.custom_input.clear();
        self.editing_custom = false;
    }

    fn selected_row_is_custom(&self, question: &UserInputQuestionPresentation) -> bool {
        question.allow_custom && self.state.selected_option() >= question.options.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Effect produced by the user input flow.
pub enum UserInputEffect {
    KeepOpen,
    Close,
    Submit,
    Cancel,
}

fn option_row_count(question: &UserInputQuestionPresentation) -> usize {
    question.options.len() + usize::from(question.allow_custom)
}

fn preferred_option_row(
    question: &UserInputQuestionPresentation,
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

fn move_selected_index(selected: &mut usize, count: usize, delta: isize) {
    if count == 0 {
        *selected = 0;
        return;
    }
    let next = (*selected as isize + delta).rem_euclid(count as isize);
    *selected = next as usize;
}

/// Renders the full User Input overlay from its TUI-owned request projection
/// and presentation state. Domain reply construction and Runtime effects stay
/// in the App adapter.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &UserInputPresentation,
    i18n: &I18n,
) {
    if presentation.is_review_decision() {
        render_review_decision(frame, area, presentation, i18n);
        return;
    }

    let overlay = presentation.overlay();
    let title = display_title(overlay, i18n);
    if presentation.screen() == QuestionFlowScreen::Review {
        let nav_body = navigation_body(presentation, i18n);
        let mut review_lines = markdown_lines(overlay.body_markdown.as_str());
        if !review_lines.is_empty() {
            review_lines.push(Line::default());
        }
        review_lines.push(Line::from(Span::styled(
            i18n.text("overlay-user-input-review-intro"),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (index, question) in presentation.questions().iter().enumerate() {
            let values = answer_values(question, presentation.answer(question.id.as_str()));
            let answered = !values.is_empty();
            let style = if index == presentation.selected_question() {
                selection_style()
            } else {
                Style::default()
            };
            review_lines.push(Line::from(vec![
                Span::styled(format!("{} ", if answered { "[x]" } else { "[ ]" }), style),
                Span::styled(question_label(question), style.add_modifier(Modifier::BOLD)),
            ]));
            review_lines.push(Line::from(Span::styled(
                format!("    {}", answer_preview(values.as_slice(), i18n)),
                if answered {
                    Style::default().fg(agena_tui_components::theme::info_color())
                } else {
                    Style::default().fg(agena_tui_components::theme::muted_color())
                },
            )));
        }
        let review_body = Text::from(review_lines);
        let footer = Text::from(footer_text(
            overlay,
            i18n,
            "overlay-user-input-footer-review",
        ));
        render_question_flow_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            &QuestionFlowDialogSpec::new(
                title.into(),
                92,
                i18n.text("overlay-user-input-questions").into(),
                Some(&nav_body),
                QuestionFlowDialogMode::review(
                    i18n.text("overlay-user-input-summary").into(),
                    &review_body,
                    &footer,
                ),
            ),
        );
        return;
    }

    let Some(question) = presentation
        .questions()
        .get(presentation.selected_question())
    else {
        let body = if overlay.body_markdown.trim().is_empty() {
            Text::from(vec![Line::from(
                i18n.text("overlay-user-input-no-questions"),
            )])
        } else {
            Text::from(markdown_lines(overlay.body_markdown.as_str()))
        };
        render_question_flow_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            &QuestionFlowDialogSpec::new(
                title.into(),
                92,
                i18n.text("overlay-user-input-questions").into(),
                None,
                QuestionFlowDialogMode::empty(
                    i18n.text("overlay-user-input-detail").into(),
                    &body,
                    12,
                ),
            ),
        );
        return;
    };

    let nav_body = navigation_body(presentation, i18n);
    let draft = presentation
        .answer(question.id.as_str())
        .cloned()
        .unwrap_or_default();
    let values = answer_values(question, Some(&draft));
    let unanswered = i18n.text("overlay-user-input-unanswered");
    let answer_summary = if values.is_empty() {
        unanswered.clone()
    } else {
        values.join(", ")
    };
    let mut prompt_lines = markdown_lines(overlay.body_markdown.as_str());
    if !prompt_lines.is_empty() {
        prompt_lines.push(Line::default());
    }
    prompt_lines.extend([
        Line::from(Span::styled(
            sanitize_display_text(question.question.as_str()),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "{} · id={}",
                i18n.text(if question.multiple {
                    "overlay-user-input-choice-multiple"
                } else {
                    "overlay-user-input-choice-single"
                }),
                question.id
            ),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )),
        Line::from(vec![
            Span::styled(
                format!("{} ", i18n.text("overlay-user-input-current-answer")),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                sanitize_display_text(answer_summary.as_str()),
                if answer_summary == unanswered {
                    Style::default().fg(agena_tui_components::theme::muted_color())
                } else {
                    Style::default().fg(agena_tui_components::theme::info_color())
                },
            ),
        ]),
    ]);
    let prompt_body = Text::from(prompt_lines);
    let choice_width = area.width.saturating_sub(8);
    let mut choice_lines = Vec::new();
    for (index, option) in question.options.iter().enumerate() {
        let selected = index == presentation.selected_option() && !presentation.is_editing_custom();
        let style = selected.then(selection_style).unwrap_or_default();
        let picked = draft.option_indexes.contains(&index);
        let marker = if question.multiple {
            if picked { "[x]" } else { "[ ]" }
        } else if picked {
            "(x)"
        } else {
            "( )"
        };
        choice_lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), style),
            Span::styled(
                sanitize_display_text(option.label.as_str()),
                style.add_modifier(Modifier::BOLD),
            ),
        ]));
        if !option.description.trim().is_empty() {
            choice_lines.push(Line::from(Span::styled(
                format!(
                    "    {}",
                    truncate_display_text(option.description.as_str(), choice_width)
                ),
                if selected {
                    style
                } else {
                    Style::default().fg(agena_tui_components::theme::muted_color())
                },
            )));
        }
    }
    if question.allow_custom {
        let selected = question.options.len() == presentation.selected_option()
            && !presentation.is_editing_custom();
        let style = selected.then(selection_style).unwrap_or_default();
        let picked = !draft.custom_values.is_empty();
        let marker = if question.multiple {
            if picked { "[x]" } else { "[ ]" }
        } else if picked {
            "(x)"
        } else {
            "( )"
        };
        choice_lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), style),
            Span::styled(
                i18n.text("overlay-user-input-other"),
                style.add_modifier(Modifier::BOLD),
            ),
        ]));
        let values = if draft.custom_values.is_empty() {
            i18n.text("overlay-user-input-custom-empty")
        } else {
            truncate_display_text(draft.custom_values.join(", ").as_str(), choice_width)
        };
        choice_lines.push(Line::from(Span::styled(
            format!("    {values}"),
            if draft.custom_values.is_empty() {
                if selected {
                    style
                } else {
                    Style::default().fg(agena_tui_components::theme::muted_color())
                }
            } else if selected {
                style
            } else {
                Style::default().fg(agena_tui_components::theme::info_color())
            },
        )));
    }
    let choices_body = Text::from(choice_lines);
    let preview = question
        .options
        .get(presentation.selected_option())
        .filter(|option| !option.preview_markdown.trim().is_empty());
    let preview_title = preview.map(|option| {
        i18n.text_args(
            "overlay-user-input-preview",
            &crate::fl_args!("label" => sanitize_display_text(option.label.as_str())),
        )
    });
    let preview_body =
        preview.map(|option| Text::from(markdown_lines(option.preview_markdown.as_str())));
    let footer = Text::from(footer_text(
        overlay,
        i18n,
        "overlay-user-input-footer-question",
    ));
    let custom_input = question.allow_custom.then(|| {
        QuestionFlowCustomInputSpec::new(
            i18n.text(if presentation.is_editing_custom() {
                "overlay-user-input-custom-input"
            } else {
                "overlay-user-input-custom-input-hint"
            })
            .into(),
            presentation.custom_input(),
            presentation.is_editing_custom(),
        )
    });
    let result = render_question_flow_dialog(
        frame,
        area,
        SurfaceMode::Overlay,
        &QuestionFlowDialogSpec::new(
            title.into(),
            92,
            i18n.text("overlay-user-input-questions").into(),
            Some(&nav_body),
            QuestionFlowDialogMode::question(
                i18n.text("overlay-user-input-prompt-panel").into(),
                &prompt_body,
                i18n.text("overlay-user-input-choices").into(),
                &choices_body,
                preview_title.map(Into::into),
                preview_body.as_ref(),
                custom_input,
                &footer,
            ),
        ),
    );
    if let Some(cursor) = result.cursor {
        frame.set_cursor_position(cursor);
    }
}

/// Layout metrics of the review-decision overlay, shared by the renderer and
/// the paging logic so PageUp/PageDown step by exactly one visible screen
/// instead of a hard-coded number of lines.
#[derive(Debug, Clone, Copy)]
struct ReviewDecisionLayout {
    natural_height: u16,
    body_height: u16,
}

fn review_decision_layout(
    presentation: &UserInputPresentation,
    i18n: &I18n,
    area: Rect,
) -> ReviewDecisionLayout {
    let overlay = presentation.overlay();
    let content_width = SurfaceMode::Overlay
        .content_width(area, area.width)
        .saturating_sub(2)
        .max(1);
    let plan_body = Text::from(markdown_lines(overlay.body_markdown.as_str()));
    let natural_height =
        wrapped_text_height_for_text(&plan_body, content_width.saturating_sub(2).max(1));
    let footer = Text::from(vec![
        Line::from(Span::styled(
            format!(
                "Enter {} · Ctrl+X {} · ↑/↓ choose · PgUp/PgDn scroll",
                submit_label(overlay, i18n),
                cancel_label(overlay, i18n)
            ),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )),
        timeout_text(overlay, i18n)
            .map(|text| {
                Line::from(Span::styled(
                    format!("◷ {text}"),
                    Style::default().fg(agena_tui_components::theme::warning_color()),
                ))
            })
            .unwrap_or_default(),
    ]);
    let decision_height = list_panel_height(
        presentation
            .questions()
            .first()
            .map(|question| question.options.len() + usize::from(question.allow_custom))
            .unwrap_or(0),
        2,
        4,
        10,
    );
    let editor_height = if presentation.review().is_editing_custom() {
        3
    } else {
        0
    };
    let footer_height = wrapped_text_height_for_text(&footer, content_width).clamp(1, 2);
    let body_height = area
        .height
        .saturating_sub(4)
        .saturating_sub(decision_height)
        .saturating_sub(editor_height)
        .saturating_sub(footer_height)
        .max(1);
    ReviewDecisionLayout {
        natural_height,
        body_height,
    }
}

/// The number of lines one PageUp/PageDown moves the review-decision Plan
/// panel, matching the panel's visible height.
pub fn review_decision_page_size(
    presentation: &UserInputPresentation,
    i18n: &I18n,
    area: Rect,
) -> usize {
    usize::from(
        review_decision_layout(presentation, i18n, area)
            .body_height
            .max(1),
    )
}

fn render_review_decision(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &UserInputPresentation,
    i18n: &I18n,
) {
    let overlay = presentation.overlay();
    let Some(question) = presentation.questions().first() else {
        return;
    };
    let plan_body = Text::from(markdown_lines(overlay.body_markdown.as_str()));
    let mut items = question
        .options
        .iter()
        .map(|option| {
            build_detail_two_line_list_item(
                sanitize_display_text(option.label.as_str()).into(),
                (!option.description.trim().is_empty())
                    .then(|| sanitize_display_text(option.description.as_str()).into()),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )
        })
        .collect::<Vec<ListItem<'static>>>();
    if question.allow_custom {
        let feedback = presentation.review().custom_text();
        items.push(build_detail_two_line_list_item(
            i18n.text("overlay-user-input-review-feedback").into(),
            if feedback.is_empty() {
                Some(i18n.text("overlay-user-input-review-feedback-empty").into())
            } else {
                Some(sanitize_display_text(feedback.as_str()).into())
            },
            Style::default().fg(agena_tui_components::theme::muted_color()),
        ));
    }
    let footer = Text::from(vec![
        Line::from(Span::styled(
            format!(
                "Enter {} · Ctrl+X {} · ↑/↓ choose · PgUp/PgDn scroll",
                submit_label(overlay, i18n),
                cancel_label(overlay, i18n)
            ),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )),
        timeout_text(overlay, i18n)
            .map(|text| {
                Line::from(Span::styled(
                    format!("◷ {text}"),
                    Style::default().fg(agena_tui_components::theme::warning_color()),
                ))
            })
            .unwrap_or_default(),
    ]);
    let layout = review_decision_layout(presentation, i18n, area);
    let scroll = presentation.review().scroll().min(
        layout
            .natural_height
            .saturating_sub(layout.body_height.saturating_sub(2)),
    );
    let mut sections = Vec::with_capacity(3);
    if presentation.review().is_editing_custom() {
        sections.push(StackedDialogSection::EditorPanel(EditorSection {
            height: StackedDialogSectionHeight::AutoEditor { multiline: false },
            spec: EditorPanelSpec {
                title: Some(i18n.text("overlay-user-input-review-feedback").into()),
                borders: Borders::ALL,
            },
            input: presentation.review().custom_input(),
            set_cursor: true,
        }));
    } else {
        sections.push(StackedDialogSection::ListPanel(ListPanelSection {
            height: StackedDialogSectionHeight::AutoList {
                lines_per_item: 2,
                min_body: 4,
                max_body: 10,
            },
            spec: ListPanelSpec::new(
                Some("Decisions".into()),
                items.as_slice(),
                Some(presentation.review().selected_option()),
                selection_style(),
                ">> ".into(),
            ),
        }));
    }
    sections.push(StackedDialogSection::TextPanel(TextPanelSection {
        height: StackedDialogSectionHeight::Fixed(layout.body_height),
        spec: TextPanelSpec {
            title: Some("Plan".into()),
            body: &plan_body,
            wrap: true,
            scroll: Some((scroll, 0)),
            alignment: None,
        },
    }));
    sections.push(StackedDialogSection::Paragraph(ParagraphSection {
        height: StackedDialogSectionHeight::AutoText { min: 1, max: 2 },
        title: None,
        borders: Borders::NONE,
        body: footer,
        wrap: true,
        scroll: None,
        alignment: None,
    }));
    let result = render_stacked_dialog(
        frame,
        area,
        SurfaceMode::Overlay,
        &StackedDialogSpec {
            title: display_title(overlay, i18n).into(),
            target_width: area.width,
            sections,
        },
    );
    if let Some(cursor) = result.cursor {
        frame.set_cursor_position(cursor);
    }
}

fn navigation_body(presentation: &UserInputPresentation, i18n: &I18n) -> Text<'static> {
    let overlay = presentation.overlay();
    let mut spans = Vec::new();
    for (index, question) in presentation.questions().iter().enumerate() {
        let answered =
            !answer_values(question, presentation.answer(question.id.as_str())).is_empty();
        let label = if question.header.trim().is_empty() {
            format!("Q{}", index + 1)
        } else {
            question.header.clone()
        };
        let style = if index == presentation.selected_question()
            && presentation.screen() == QuestionFlowScreen::Question
        {
            selection_style()
        } else if answered {
            Style::default()
                .fg(agena_tui_components::theme::info_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        spans.push(Span::styled(
            format!(
                " {} {} ",
                if answered { "[x]" } else { "[ ]" },
                truncate_display_text(label.as_str(), 12)
            ),
            style,
        ));
        spans.push(Span::raw(" "));
    }
    if !presentation.review_is_hidden() {
        spans.push(Span::styled(
            format!(" [>] {} ", submit_label(overlay, i18n)),
            if presentation.screen() == QuestionFlowScreen::Review {
                selection_style()
            } else {
                Style::default()
            },
        ));
    }
    let mut lines = vec![
        Line::from(Span::styled(
            i18n.text_args(
                "overlay-user-input-request-id",
                &crate::fl_args!("request_id" => overlay.request_id.clone()),
            ),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )),
        Line::from(spans),
    ];
    if let Some(timeout) = timeout_text(overlay, i18n) {
        lines.push(Line::from(Span::styled(
            format!("◷ {timeout}"),
            Style::default().fg(agena_tui_components::theme::warning_color()),
        )));
    }
    Text::from(lines)
}

fn display_title(overlay: &UserInputOverlayPresentation, i18n: &I18n) -> String {
    if !overlay.title.trim().is_empty() {
        sanitize_display_text(overlay.title.as_str())
    } else {
        i18n.text("overlay-user-input-title")
    }
}

fn submit_label(overlay: &UserInputOverlayPresentation, i18n: &I18n) -> String {
    if !overlay.submit_label.trim().is_empty() {
        sanitize_display_text(overlay.submit_label.as_str())
    } else {
        i18n.text("overlay-user-input-submit")
    }
}

fn cancel_label(overlay: &UserInputOverlayPresentation, _i18n: &I18n) -> String {
    if !overlay.cancel_label.trim().is_empty() {
        sanitize_display_text(overlay.cancel_label.as_str())
    } else {
        "cancel".to_owned()
    }
}

fn footer_text(overlay: &UserInputOverlayPresentation, i18n: &I18n, key: &str) -> String {
    let mut footer = i18n.text(key);
    if !overlay.cancel_label.trim().is_empty() {
        footer.push_str(" · Esc ");
        footer.push_str(sanitize_display_text(overlay.cancel_label.as_str()).as_str());
    }
    footer
}

fn timeout_text(overlay: &UserInputOverlayPresentation, i18n: &I18n) -> Option<String> {
    let timeout = overlay.auto_resolution_ms?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    let remaining = overlay
        .created_at_ms
        .saturating_add(timeout as i64)
        .saturating_sub(now)
        .max(0) as u64;
    let seconds = remaining.div_ceil(1000);
    Some(i18n.text_args(
        "overlay-user-input-auto-resolve",
        &crate::fl_args!("remaining" => format!("{}:{:02}", seconds / 60, seconds % 60)),
    ))
}

fn answer_values(
    question: &UserInputQuestionPresentation,
    draft: Option<&UserInputAnswerDraft>,
) -> Vec<String> {
    let Some(draft) = draft else {
        return Vec::new();
    };
    let mut values = draft
        .option_indexes
        .iter()
        .filter_map(|index| question.options.get(*index))
        .map(|option| option.label.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.extend(
        draft
            .custom_values
            .iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
    );
    if question.multiple {
        values
    } else {
        values.into_iter().take(1).collect()
    }
}

fn question_label(question: &UserInputQuestionPresentation) -> String {
    if !question.header.trim().is_empty() {
        sanitize_display_text(question.header.as_str())
    } else if !question.question.trim().is_empty() {
        sanitize_display_text(question.question.as_str())
    } else {
        sanitize_display_text(question.id.as_str())
    }
}

fn answer_preview(values: &[String], i18n: &I18n) -> String {
    if values.is_empty() {
        i18n.text("overlay-user-input-unanswered")
    } else {
        truncate_display_text(values.join(", ").as_str(), 72)
    }
}

pub(crate) fn markdown_lines(markdown: &str) -> Vec<Line<'static>> {
    let markdown = markdown.trim();
    if markdown.is_empty() {
        return vec![Line::from("")];
    }
    let rendered = markdown_to_text(markdown);
    rendered
        .lines
        .into_iter()
        .map(|line| {
            Line::from(
                line.spans
                    .into_iter()
                    .map(|span| Span::styled(sanitize_display_text(&span.content), span.style))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn selection_style() -> Style {
    agena_tui_components::theme::selection_style()
}

fn truncate_display_text(text: &str, max_width: u16) -> String {
    let max_width = max_width as usize;
    let mut width = 0_usize;
    let mut out = String::new();
    for ch in sanitize_display_text(text).chars() {
        let char_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width.saturating_add(char_width) > max_width {
            break;
        }
        out.push(ch);
        width = width.saturating_add(char_width);
    }
    if out.chars().count() < text.chars().count() && max_width > 0 {
        out.push('…');
    }
    out
}

fn sanitize_display_text(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        match ch {
            '\r' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
            '\n' | '\t' => out.push(ch),
            value if value.is_control() => out.push(' '),
            value => out.push(value),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        UserInputEffect, UserInputOptionPresentation, UserInputPresentation,
        UserInputQuestionPresentation,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn question(multiple: bool, allow_custom: bool) -> UserInputQuestionPresentation {
        UserInputQuestionPresentation {
            id: "question".into(),
            header: String::new(),
            question: "Choose".into(),
            options: vec![UserInputOptionPresentation {
                label: "One".into(),
                description: String::new(),
                preview_markdown: String::new(),
            }],
            multiple,
            allow_custom,
        }
    }

    fn overlay(review_decision: bool) -> super::UserInputOverlayPresentation {
        super::UserInputOverlayPresentation {
            request_id: "request".into(),
            title: String::new(),
            body_markdown: String::new(),
            submit_label: String::new(),
            cancel_label: String::new(),
            auto_resolution_ms: None,
            created_at_ms: 0,
            review_decision,
        }
    }

    #[test]
    fn single_question_accept_emits_submit_and_records_answer() {
        let mut presentation =
            UserInputPresentation::new(overlay(false), vec![question(false, false)]);

        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::Submit
        );
        assert!(
            presentation
                .answer("question")
                .is_some_and(|draft| draft.option_indexes.contains(&0))
        );
    }

    #[test]
    fn custom_text_is_owned_and_committed_by_the_presentation() {
        let mut presentation =
            UserInputPresentation::new(overlay(false), vec![question(false, true)]);

        assert!(presentation.insert_custom_text("custom value"));
        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::Submit
        );
        assert_eq!(
            presentation
                .answer("question")
                .expect("custom answer")
                .custom_values,
            vec!["custom value".to_string()]
        );
    }

    #[test]
    fn review_decision_wraps_selection_and_emits_submit() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, false)]);

        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), 10),
            UserInputEffect::KeepOpen
        );
        assert_eq!(presentation.review().selected_option(), 0);
        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::Submit
        );
    }

    #[test]
    fn review_decision_page_keys_step_by_page_size() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, false)]);

        presentation.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().scroll(), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), 7);
        assert_eq!(presentation.review().scroll(), 17);
        presentation.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), 7);
        assert_eq!(presentation.review().scroll(), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().scroll(), 0);
    }

    #[test]
    fn review_decision_feedback_row_edits_and_submits_text() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, true)]);

        presentation.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().selected_option(), 1);

        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::KeepOpen
        );
        assert!(presentation.review().is_editing_custom());

        presentation.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().custom_text(), "hi");

        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::Submit
        );
        assert_eq!(presentation.review().custom_text(), "hi");
    }

    #[test]
    fn review_decision_feedback_esc_exits_editor_without_submitting() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, true)]);

        presentation.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10);
        assert!(presentation.review().is_editing_custom());

        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 10),
            UserInputEffect::KeepOpen
        );
        assert!(!presentation.review().is_editing_custom());
        assert_eq!(presentation.review().selected_option(), 1);
    }

    #[test]
    fn review_decision_enter_on_filled_feedback_row_submits() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, true)]);

        presentation.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().custom_text(), "o");
        assert!(!presentation.review().is_editing_custom());

        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::Submit
        );
    }
}
