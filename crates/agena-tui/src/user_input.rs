//! Presentation state and reduction for interactive user-input overlays.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{ListItem, Paragraph},
};

use crate::i18n::I18n;
use crate::keymap::{KeyAction, KeyContext, resolve};
use agena_tui_components::{
    Editor, FramedSurfaceSpec, QuestionFlowCustomInputSpec, QuestionFlowDialogMode,
    QuestionFlowDialogSpec, QuestionFlowScreen, QuestionFlowState, SurfaceMode,
    framed_overlay_height, render_framed_surface, render_question_flow_dialog_scrollable,
    wrapped_text_height_for_text,
};
use tui_markdown::from_str as markdown_to_text;

/// A display-only option in an interactive question. Domain request mapping,
/// validation, reply construction, and submission remain outside the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInputOptionPresentation {
    pub label: String,
    pub description: String,
}

/// A display-only interactive question used by the TUI reducer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInputQuestionPresentation {
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
///
/// The review overlay is one flat document: the chat-style plan rows
/// (pre-rendered by the App), a separator row, and the decision rows. The
/// cursor moves through the whole document with vim-style keys, and the
/// decisions are only reachable at the very bottom, so the user reads the plan
/// before choosing.
#[derive(Debug, Clone, Default)]
pub struct UserInputReviewPresentation {
    /// Chat-style pre-rendered plan rows (wrapped to `content_width`), set by
    /// the App through [`UserInputPresentation::set_review_plan`].
    plan_lines: Vec<Line<'static>>,
    /// Width used to pre-render `plan_lines`; decision rows wrap at this width
    /// too so cursor arithmetic stays consistent.
    content_width: u16,
    /// Cursor row inside the flat review document.
    cursor_line: usize,
    /// First visible row of the review document.
    scroll: usize,
    /// Set while the user is between `g` and the completing `g`/`G`.
    pending_goto: bool,
    /// Set while the user is between `z` and the completing `z`/`t`/`b`.
    pending_viewport: bool,
    /// Last decision row selected while the cursor was inside the decision
    /// block. Used to map a submit back to the chosen option.
    selected_option: usize,
    custom_input: Editor,
    editing_custom: bool,
}

impl UserInputReviewPresentation {
    pub fn selected_option(&self) -> usize {
        self.selected_option
    }

    pub fn cursor_line(&self) -> usize {
        self.cursor_line
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Number of pre-rendered plan rows (at least one placeholder so the
    /// document always has content to scroll before the decision block).
    pub fn plan_rows(&self) -> usize {
        self.plan_lines.len().max(1)
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
    answers: BTreeMap<usize, UserInputAnswerDraft>,
    state: QuestionFlowState,
    editing_custom: bool,
    custom_input: Editor,
    review: UserInputReviewPresentation,
    review_decision: bool,
    /// Whole-dialog scroll for the question-flow overlays.
    flow_scroll: u16,
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
            flow_scroll: 0,
        }
    }

    pub fn overlay(&self) -> &UserInputOverlayPresentation {
        &self.overlay
    }

    pub fn questions(&self) -> &[UserInputQuestionPresentation] {
        &self.questions
    }

    pub fn answers(&self) -> &BTreeMap<usize, UserInputAnswerDraft> {
        &self.answers
    }

    pub fn answer(&self, index: usize) -> Option<&UserInputAnswerDraft> {
        self.answers.get(&index)
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

    /// Whole-dialog scroll of the question-flow overlays.
    pub fn flow_scroll(&self) -> u16 {
        self.flow_scroll
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

    /// Stores chat-style pre-rendered plan rows for a review-decision overlay.
    /// The App renders `body_markdown` through the transcript Markdown pipeline
    /// (inside `with_text_math_rendering`) at `content_width` columns.
    pub fn set_review_plan(&mut self, lines: Vec<Line<'static>>, content_width: u16) {
        self.review.plan_lines = lines;
        self.review.content_width = content_width;
    }

    /// Number of plan rows (at least one placeholder row so an empty body
    /// still gives the cursor a document to move through).
    fn review_plan_rows(&self) -> usize {
        self.review.plan_lines.len().max(1)
    }

    /// First row of the decision block inside the flat review document.
    fn review_decision_start(&self) -> usize {
        self.review_plan_rows().saturating_add(1)
    }

    /// Number of decision rows (label + detail per option, plus the feedback
    /// row and its inline editor while editing).
    fn review_decision_rows(&self) -> usize {
        let option_rows = self
            .questions
            .first()
            .map(|question| question.options.len() + usize::from(question.allow_custom))
            .unwrap_or(0)
            .saturating_mul(2);
        option_rows.saturating_add(usize::from(self.review.editing_custom))
    }

    /// Total rows of the flat review document: plan + separator + decisions.
    fn review_total_lines(&self) -> usize {
        self.review_decision_start()
            .saturating_add(self.review_decision_rows())
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
            QuestionFlowScreen::Question => self.handle_question_key(key, page_size),
            QuestionFlowScreen::Review => self.handle_review_key(key, page_size),
        }
    }

    fn handle_review_decision_key(&mut self, key: KeyEvent, page_size: usize) -> UserInputEffect {
        if self.review.editing_custom {
            return self.handle_review_custom_key(key);
        }
        let body_height = page_size.max(1);
        // `gg`/`gG` and `zz`/`zt`/`zb` complete in two keystrokes; consume the
        // completing key before normal dispatch.
        if self.review.pending_goto {
            self.review.pending_goto = false;
            match resolve(KeyContext::UserInputReview, key) {
                Some(KeyAction::GotoPrefix) => self.review_move_cursor_to(0, body_height),
                Some(KeyAction::End) => {
                    self.review_move_cursor_to(self.review_total_lines(), body_height)
                }
                _ => {}
            }
            return UserInputEffect::KeepOpen;
        }
        if self.review.pending_viewport {
            self.review.pending_viewport = false;
            match resolve(KeyContext::UserInputReview, key) {
                Some(KeyAction::ViewportPrefix) => {
                    // zz: center the cursor row in the viewport.
                    self.review.scroll = self.review.cursor_line.saturating_sub(body_height / 2);
                }
                Some(KeyAction::ViewportTop) => {
                    // zt: scroll the cursor row to the top.
                    self.review.scroll = self.review.cursor_line;
                }
                Some(KeyAction::ViewportBottom) => {
                    // zb: scroll the cursor row to the bottom.
                    self.review.scroll = self
                        .review
                        .cursor_line
                        .saturating_sub(body_height.saturating_sub(1));
                }
                _ => {}
            }
            self.review_clamp_scroll(body_height);
            return UserInputEffect::KeepOpen;
        }
        match resolve(KeyContext::UserInputReview, key) {
            Some(KeyAction::Close) => UserInputEffect::Close,
            Some(KeyAction::Accept) => {
                if self.review_cursor_is_custom() {
                    if self.review.custom_text().is_empty() {
                        self.begin_review_custom_edit();
                        UserInputEffect::KeepOpen
                    } else {
                        UserInputEffect::Submit
                    }
                } else if self.review_cursor_in_decision_block() {
                    UserInputEffect::Submit
                } else {
                    // Still reading the plan: Enter walks down instead of
                    // submitting the first decision.
                    self.review_move_cursor_by(1, body_height);
                    UserInputEffect::KeepOpen
                }
            }
            Some(KeyAction::CancelRequest) => UserInputEffect::Cancel,
            Some(KeyAction::MoveUp) => {
                self.review_move_cursor_by(-1, body_height);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::MoveDown) => {
                self.review_move_cursor_by(1, body_height);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::PageUp) => {
                self.review_move_cursor_by(-(body_height as isize), body_height);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::PageDown) => {
                self.review_move_cursor_by(body_height as isize, body_height);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::HalfPageUp) => {
                self.review_move_cursor_by(-(body_height as isize / 2), body_height);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::HalfPageDown) => {
                self.review_move_cursor_by(body_height as isize / 2, body_height);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::GotoPrefix) => {
                self.review.pending_goto = true;
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Home) => {
                self.review_move_cursor_to(0, body_height);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::End) => {
                self.review_move_cursor_to(self.review_total_lines(), body_height);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::ViewportPrefix) => {
                self.review.pending_viewport = true;
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::ViewTop) => {
                // H: cursor to the first visible row.
                self.review.cursor_line = self.review.scroll;
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::ViewBottom) => {
                // L: cursor to the last visible row.
                self.review.cursor_line = self
                    .review
                    .scroll
                    .saturating_add(body_height.saturating_sub(1))
                    .min(self.review_total_lines());
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::ViewportTop) => {
                // t: scroll the cursor row to the top.
                self.review.scroll = self.review.cursor_line;
                self.review_clamp_scroll(body_height);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::ViewportBottom) => {
                // b: scroll the cursor row to the bottom.
                self.review.scroll = self
                    .review
                    .cursor_line
                    .saturating_sub(body_height.saturating_sub(1));
                self.review_clamp_scroll(body_height);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Edit) => {
                if self.review_cursor_is_custom() {
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

    fn review_cursor_in_decision_block(&self) -> bool {
        self.review.cursor_line >= self.review_decision_start()
    }

    fn review_cursor_is_custom(&self) -> bool {
        let Some(question) = self.questions.first() else {
            return false;
        };
        if !question.allow_custom {
            return false;
        }
        let decision_start = self.review_decision_start();
        self.review.cursor_line >= decision_start
            && self.review.cursor_line.saturating_sub(decision_start) / 2 == question.options.len()
    }

    fn review_move_cursor_by(&mut self, delta: isize, body_height: usize) {
        let body_height = body_height.max(1);
        let last = self.review_total_lines().saturating_sub(1);
        let next = (self.review.cursor_line as isize + delta).clamp(0, last as isize);
        self.review.cursor_line = next as usize;
        self.review_sync_selection();
        if delta.unsigned_abs() >= body_height {
            // Page jumps land on the page edge like the main transcript pager:
            // forward jumps place the cursor at the top of the new viewport,
            // backward jumps at the bottom.
            if delta > 0 {
                self.review.scroll = self.review.cursor_line;
            } else {
                self.review.scroll = self
                    .review
                    .cursor_line
                    .saturating_sub(body_height.saturating_sub(1));
            }
        }
        self.review_follow_cursor(body_height);
    }

    fn review_move_cursor_to(&mut self, line: usize, body_height: usize) {
        let last = self.review_total_lines().saturating_sub(1);
        self.review.cursor_line = line.min(last);
        self.review_sync_selection();
        self.review_follow_cursor(body_height);
    }

    fn review_sync_selection(&mut self) {
        let Some(question) = self.questions.first() else {
            return;
        };
        let decision_start = self.review_decision_start();
        if self.review.cursor_line < decision_start {
            return;
        }
        let option_count = question.options.len() + usize::from(question.allow_custom);
        if option_count == 0 {
            return;
        }
        let index = self.review.cursor_line.saturating_sub(decision_start) / 2;
        self.review.selected_option = index.min(option_count.saturating_sub(1));
    }

    fn review_follow_cursor(&mut self, body_height: usize) {
        let body_height = body_height.max(1);
        self.review_clamp_scroll(body_height);
        if self.review.cursor_line < self.review.scroll {
            self.review.scroll = self.review.cursor_line;
        }
        if self.review.cursor_line >= self.review.scroll.saturating_add(body_height) {
            self.review.scroll = self
                .review
                .cursor_line
                .saturating_add(1)
                .saturating_sub(body_height);
        }
    }

    fn review_clamp_scroll(&mut self, body_height: usize) {
        let total = self.review_total_lines();
        self.review.scroll = self
            .review
            .scroll
            .min(total.saturating_sub(body_height.max(1)));
    }

    fn begin_review_custom_edit(&mut self) {
        let Some(question) = self.questions.first() else {
            return;
        };
        if !question.allow_custom {
            return;
        }
        self.review.selected_option = question.options.len();
        let custom_label_row = self
            .review_decision_start()
            .saturating_add(question.options.len().saturating_mul(2));
        self.review.cursor_line = custom_label_row;
        self.review.editing_custom = true;
    }

    fn review_snap_custom_cursor(&mut self) {
        let Some(question) = self.questions.first() else {
            return;
        };
        let custom_label_row = self
            .review_decision_start()
            .saturating_add(question.options.len().saturating_mul(2));
        self.review.cursor_line = custom_label_row.min(self.review_total_lines());
    }

    fn handle_review_custom_key(&mut self, key: KeyEvent) -> UserInputEffect {
        match resolve(KeyContext::UserInputQuestion, key) {
            Some(KeyAction::Close) => {
                self.review.editing_custom = false;
                self.review_snap_custom_cursor();
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::Accept) => {
                let has_text = !self.review.custom_input.text().trim().is_empty();
                self.review.editing_custom = false;
                if has_text {
                    UserInputEffect::Submit
                } else {
                    self.review_snap_custom_cursor();
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

    fn handle_question_key(&mut self, key: KeyEvent, page_size: usize) -> UserInputEffect {
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
            Some(KeyAction::PageUp) => {
                self.move_flow_scroll(-(page_size.max(1) as i16));
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::PageDown) => {
                self.move_flow_scroll(page_size.max(1) as i16);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::HalfPageUp) => {
                self.move_flow_scroll(-(page_size.max(1) as i16 / 2));
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::HalfPageDown) => {
                self.move_flow_scroll(page_size.max(1) as i16 / 2);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::ScrollLineUp) => {
                self.move_flow_scroll(-1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::ScrollLineDown) => {
                self.move_flow_scroll(1);
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

    fn move_flow_scroll(&mut self, delta: i16) {
        self.flow_scroll = if delta < 0 {
            self.flow_scroll.saturating_sub(delta.unsigned_abs())
        } else {
            self.flow_scroll.saturating_add(delta as u16)
        };
    }

    fn handle_review_key(&mut self, key: KeyEvent, page_size: usize) -> UserInputEffect {
        match resolve(KeyContext::UserInputReview, key) {
            Some(KeyAction::Close) => UserInputEffect::Close,
            Some(KeyAction::Accept) => UserInputEffect::Submit,
            Some(KeyAction::CancelRequest) => UserInputEffect::Cancel,
            Some(KeyAction::PageUp) => {
                self.move_flow_scroll(-(page_size.max(1) as i16));
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::PageDown) => {
                self.move_flow_scroll(page_size.max(1) as i16);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::HalfPageUp) => {
                self.move_flow_scroll(-(page_size.max(1) as i16 / 2));
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::HalfPageDown) => {
                self.move_flow_scroll(page_size.max(1) as i16 / 2);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::ScrollLineUp) => {
                self.move_flow_scroll(-1);
                UserInputEffect::KeepOpen
            }
            Some(KeyAction::ScrollLineDown) => {
                self.move_flow_scroll(1);
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
            .get(&self.state.selected_question())
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
        let (is_custom, allow_custom, multiple, option_count) = {
            let Some(question) = self.questions.get(self.state.selected_question()) else {
                return;
            };
            (
                self.selected_row_is_custom(question),
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
        let draft = self
            .answers
            .entry(self.state.selected_question())
            .or_default();
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
        let is_custom = {
            let Some(question) = self.questions.get(self.state.selected_question()) else {
                return;
            };
            self.selected_row_is_custom(question)
        };
        if is_custom {
            return;
        }
        let draft = self
            .answers
            .entry(self.state.selected_question())
            .or_default();
        draft.option_indexes.clear();
        draft.option_indexes.insert(self.state.selected_option());
        draft.custom_values.clear();
    }

    fn begin_custom_edit(&mut self) -> bool {
        let Some(question) = self.questions.get(self.state.selected_question()) else {
            return false;
        };
        let allow_custom = question.allow_custom;
        let question_index = self.state.selected_question();
        let selected_option = question.options.len();
        if !allow_custom {
            return false;
        }
        self.state
            .focus_question(question_index, self.questions.len());
        self.state.set_selected_option(selected_option);
        let existing = self
            .answers
            .get(&question_index)
            .map(|draft| draft.custom_values.join(", "))
            .unwrap_or_default();
        self.custom_input.set_text(existing);
        self.editing_custom = true;
        true
    }

    fn commit_custom_values(&mut self) -> bool {
        let (question_index, multiple, custom_row) = {
            let Some(question) = self.questions.get(self.state.selected_question()) else {
                self.editing_custom = false;
                return false;
            };
            (
                self.state.selected_question(),
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
        let draft = self.answers.entry(question_index).or_default();
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
        if self.questions.get(self.state.selected_question()).is_none() {
            return;
        }
        self.answers.remove(&self.state.selected_question());
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
        let mut review_lines: Vec<Line<'static>> = Vec::new();
        review_lines.push(Line::from(Span::styled(
            i18n.text("overlay-user-input-review-intro"),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (index, question) in presentation.questions().iter().enumerate() {
            let values = answer_values(question, presentation.answer(index));
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
        render_question_flow_dialog_scrollable(
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
            presentation.flow_scroll,
        );
        return;
    }

    let Some(question) = presentation
        .questions()
        .get(presentation.selected_question())
    else {
        let body = Text::from(vec![Line::from(
            i18n.text("overlay-user-input-no-questions"),
        )]);
        render_question_flow_dialog_scrollable(
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
            presentation.flow_scroll,
        );
        return;
    };

    let nav_body = navigation_body(presentation, i18n);
    let draft = presentation
        .answer(presentation.selected_question())
        .cloned()
        .unwrap_or_default();
    let prompt_lines = vec![Line::from(Span::styled(
        sanitize_display_text(question.question.as_str()),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    let prompt_body = Text::from(prompt_lines);
    let choice_width = area.width.saturating_sub(8);
    let mut choice_items = Vec::new();
    let selected_row = presentation.selected_option();
    for (index, option) in question.options.iter().enumerate() {
        let picked = draft.option_indexes.contains(&index);
        let marker = if question.multiple {
            if picked { "[x]" } else { "[ ]" }
        } else if picked {
            "(x)"
        } else {
            "( )"
        };
        let mut lines = vec![Line::from(vec![
            Span::styled(format!("{marker} "), Style::default()),
            Span::styled(
                sanitize_display_text(option.label.as_str()),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ])];
        if !option.description.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                format!(
                    "    {}",
                    truncate_display_text(option.description.as_str(), choice_width)
                ),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )));
        }
        choice_items.push(ListItem::new(lines));
    }
    if question.allow_custom {
        let picked = !draft.custom_values.is_empty();
        let marker = if question.multiple {
            if picked { "[x]" } else { "[ ]" }
        } else if picked {
            "(x)"
        } else {
            "( )"
        };
        let mut lines = vec![Line::from(vec![
            Span::styled(format!("{marker} "), Style::default()),
            Span::styled(
                i18n.text("overlay-user-input-other"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ])];
        if !presentation.is_editing_custom() {
            let values = if draft.custom_values.is_empty() {
                i18n.text("overlay-user-input-custom-empty")
            } else {
                truncate_display_text(draft.custom_values.join(", ").as_str(), choice_width)
            };
            lines.push(Line::from(Span::styled(
                format!("    {values}"),
                if draft.custom_values.is_empty() {
                    Style::default().fg(agena_tui_components::theme::muted_color())
                } else {
                    Style::default().fg(agena_tui_components::theme::info_color())
                },
            )));
        }
        choice_items.push(ListItem::new(lines));
    }
    let footer = Text::from(footer_text(
        overlay,
        i18n,
        "overlay-user-input-footer-question",
    ));
    // The custom reply is typed inline inside the choices panel, directly under
    // the custom option row, while the custom editor is active.
    let custom_input = question
        .allow_custom
        .then_some(QuestionFlowCustomInputSpec::new(
            i18n.text("overlay-user-input-custom-input").into(),
            presentation.custom_input(),
            presentation.is_editing_custom(),
        ))
        .filter(|_| presentation.is_editing_custom());
    let result = render_question_flow_dialog_scrollable(
        frame,
        area,
        SurfaceMode::Overlay,
        &QuestionFlowDialogSpec::new(
            title.into(),
            92,
            i18n.text("overlay-user-input-questions").into(),
            Some(&nav_body),
            QuestionFlowDialogMode::question(
                format!(
                    "{} ({})",
                    i18n.text("overlay-user-input-prompt-panel"),
                    i18n.text(if question.multiple {
                        "overlay-user-input-choice-multiple-short"
                    } else {
                        "overlay-user-input-choice-single-short"
                    })
                )
                .into(),
                &prompt_body,
                i18n.text("overlay-user-input-choices").into(),
                choice_items.as_slice(),
                Some(selected_row),
                None,
                None,
                custom_input,
                &footer,
            ),
        ),
        presentation.flow_scroll,
    );
    if let Some(cursor) = result.cursor {
        frame.set_cursor_position(cursor);
    }
}

/// Target width (columns) for the review-decision overlay, comparable to the
/// skill-studio detail dialog (104). Wide enough for plan documents to read
/// comfortably; `adaptive_modal_width` still caps it to fit small terminals.
const REVIEW_DECISION_TARGET_WIDTH: u16 = 108;

/// Minimum dialog height for the review-decision overlay, so short plans still
/// open as a substantial modal instead of hugging a handful of rows.
const REVIEW_DECISION_MIN_HEIGHT: usize = 18;

/// The width at which the review-decision document is laid out. The App
/// pre-renders the plan body at this width so cursor arithmetic matches the
/// rendered rows.
pub fn user_input_review_content_width(area: Rect) -> u16 {
    SurfaceMode::Overlay
        .content_width(area, REVIEW_DECISION_TARGET_WIDTH)
        .max(1)
}

/// Layout metrics of the review-decision overlay, shared by the renderer and
/// the paging logic so PageUp/PageDown step by exactly one visible screen
/// instead of a hard-coded number of lines.
#[derive(Debug, Clone, Copy)]
struct ReviewDecisionLayout {
    natural_height: usize,
    body_height: usize,
}

fn review_decision_layout(
    presentation: &UserInputPresentation,
    i18n: &I18n,
    area: Rect,
) -> ReviewDecisionLayout {
    let overlay = presentation.overlay();
    let content_width = presentation
        .review()
        .content_width
        .max(user_input_review_content_width(area));
    let footer = Text::from(review_footer_lines(overlay, i18n));
    let footer_height =
        usize::from(wrapped_text_height_for_text(&footer, content_width).clamp(1, 2));
    let natural_height = presentation
        .review_total_lines()
        .max(REVIEW_DECISION_MIN_HEIGHT);
    let target_height = framed_overlay_height(u16::try_from(natural_height).unwrap_or(u16::MAX));
    let outer_height = SurfaceMode::Overlay
        .outer_rect(area, content_width.saturating_add(2), target_height)
        .height;
    let inner_height = usize::from(outer_height.saturating_sub(2).max(1));
    let body_height = inner_height.saturating_sub(footer_height).max(1);
    ReviewDecisionLayout {
        natural_height,
        body_height,
    }
}

/// The number of lines one PageUp/PageDown moves the review-decision cursor,
/// matching the document's visible height.
pub fn review_decision_page_size(
    presentation: &UserInputPresentation,
    i18n: &I18n,
    area: Rect,
) -> usize {
    review_decision_layout(presentation, i18n, area)
        .body_height
        .max(1)
}

fn review_footer_lines(overlay: &UserInputOverlayPresentation, i18n: &I18n) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(footer_text(
        overlay,
        i18n,
        "overlay-user-input-footer-review",
    ))];
    if let Some(timeout) = timeout_text(overlay, i18n) {
        lines.push(Line::from(Span::styled(
            format!("◷ {timeout}"),
            Style::default().fg(agena_tui_components::theme::warning_color()),
        )));
    }
    lines
}

/// Builds the flat review document: chat-style plan rows, a separator, and the
/// decision rows. The cursor row is highlighted; the selected decision shows a
/// `(x)` marker on its label row.
fn build_review_document(
    presentation: &UserInputPresentation,
    question: &UserInputQuestionPresentation,
    i18n: &I18n,
    content_width: u16,
) -> Vec<Line<'static>> {
    let mut document = Vec::new();
    let plan = presentation.review().plan_lines.clone();
    if plan.is_empty() {
        document.push(Line::from(Span::styled(
            i18n.text("overlay-user-input-no-questions"),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));
    } else {
        document.extend(plan);
    }
    document.push(Line::from(Span::styled(
        "─".repeat(usize::from(content_width.max(1))),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    )));

    let decision_start = presentation.review_decision_start();
    let cursor_line = presentation.review().cursor_line;
    let selected_index = if cursor_line >= decision_start {
        Some(cursor_line.saturating_sub(decision_start) / 2)
    } else {
        None
    };
    for (index, option) in question.options.iter().enumerate() {
        let marker = if selected_index == Some(index) {
            "(x)"
        } else {
            "( )"
        };
        document.push(Line::from(vec![
            Span::styled(format!("{marker} "), Style::default()),
            Span::styled(
                truncate_display_text(option.label.as_str(), content_width.saturating_sub(4)),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        document.push(Line::from(Span::styled(
            if option.description.trim().is_empty() {
                String::new()
            } else {
                format!(
                    "    {}",
                    truncate_display_text(
                        option.description.as_str(),
                        content_width.saturating_sub(4)
                    )
                )
            },
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));
    }
    if question.allow_custom {
        let custom_index = question.options.len();
        let marker = if selected_index == Some(custom_index) {
            "(x)"
        } else {
            "( )"
        };
        document.push(Line::from(vec![
            Span::styled(format!("{marker} "), Style::default()),
            Span::styled(
                i18n.text("overlay-user-input-review-feedback"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        if presentation.review().is_editing_custom() {
            document.push(Line::from(Span::styled(
                format!(
                    "    {}",
                    truncate_display_text(
                        presentation.review().custom_input().text(),
                        content_width.saturating_sub(4)
                    )
                ),
                Style::default(),
            )));
        } else {
            let feedback = presentation.review().custom_text();
            document.push(Line::from(Span::styled(
                format!(
                    "    {}",
                    if feedback.is_empty() {
                        i18n.text("overlay-user-input-review-feedback-empty")
                    } else {
                        truncate_display_text(feedback.as_str(), content_width.saturating_sub(4))
                    }
                ),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )));
        }
    }
    let last = document.len().saturating_sub(1);
    if let Some(row) = document.get_mut(cursor_line.min(last)) {
        row.style = selection_style();
    }
    document
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
    let content_width = presentation
        .review()
        .content_width
        .max(user_input_review_content_width(area));
    let layout = review_decision_layout(presentation, i18n, area);
    let scroll = presentation
        .review()
        .scroll()
        .min(layout.natural_height.saturating_sub(layout.body_height));
    let footer = Text::from(review_footer_lines(overlay, i18n));
    let frame_surface = render_framed_surface(
        frame,
        area,
        SurfaceMode::Overlay,
        &FramedSurfaceSpec {
            title: display_title(overlay, i18n).into(),
            target_width: content_width.saturating_add(2),
            target_height: framed_overlay_height(
                u16::try_from(layout.natural_height.max(1)).unwrap_or(u16::MAX),
            ),
        },
    );
    let inner = frame_surface.inner;
    let footer_height =
        usize::from(wrapped_text_height_for_text(&footer, content_width).clamp(1, 2));
    let body_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(footer_height as u16).max(1),
    };

    let document = build_review_document(presentation, question, i18n, content_width);
    let total = document.len().max(1);
    let start = scroll.min(total);
    let end = total.min(start.saturating_add(usize::from(body_area.height)));
    let visible = &document[start..end];
    frame.render_widget(
        Paragraph::new(Text::from(visible.to_vec())).wrap(ratatui::widgets::Wrap { trim: false }),
        body_area,
    );
    frame.render_widget(
        Paragraph::new(footer).wrap(ratatui::widgets::Wrap { trim: false }),
        Rect {
            x: inner.x,
            y: inner.y.saturating_add(body_area.height),
            width: inner.width,
            height: inner.height.saturating_sub(body_area.height).max(1),
        },
    );

    if presentation.review().is_editing_custom() {
        let editor_row = presentation
            .review_decision_start()
            .saturating_add(question.options.len().saturating_mul(2))
            .saturating_add(1);
        if editor_row >= start && editor_row < end {
            let text = presentation.review().custom_input().text();
            let column = unicode_width::UnicodeWidthStr::width(
                &text[..text.len().min(
                    presentation
                        .review()
                        .custom_input()
                        .cursor()
                        .min(text.len()),
                )],
            );
            let cursor_x = body_area
                .x
                .saturating_add(4)
                .saturating_add(column as u16)
                .min(
                    body_area
                        .x
                        .saturating_add(body_area.width.saturating_sub(1)),
                );
            frame.set_cursor_position((
                cursor_x,
                body_area.y.saturating_add((editor_row - start) as u16),
            ));
        }
    }
}

fn navigation_body(presentation: &UserInputPresentation, i18n: &I18n) -> Text<'static> {
    let overlay = presentation.overlay();
    let mut spans = Vec::new();
    for (index, question) in presentation.questions().iter().enumerate() {
        let answered = !answer_values(question, presentation.answer(index)).is_empty();
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
            format!(" [>] {} ", i18n.text("overlay-user-input-submit")),
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

fn footer_text(_overlay: &UserInputOverlayPresentation, i18n: &I18n, key: &str) -> String {
    i18n.text(key)
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
    } else {
        sanitize_display_text(question.question.as_str())
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
        I18n, Line, Rect, UserInputEffect, UserInputOptionPresentation, UserInputPresentation,
        UserInputQuestionPresentation,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn question(multiple: bool, allow_custom: bool) -> UserInputQuestionPresentation {
        UserInputQuestionPresentation {
            header: String::new(),
            question: "Choose".into(),
            options: vec![UserInputOptionPresentation {
                label: "One".into(),
                description: String::new(),
            }],
            multiple,
            allow_custom,
        }
    }

    fn overlay(review_decision: bool) -> super::UserInputOverlayPresentation {
        super::UserInputOverlayPresentation {
            request_id: "request".into(),
            title: String::new(),
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
                .answer(0)
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
            presentation.answer(0).expect("custom answer").custom_values,
            vec!["custom value".to_string()]
        );
    }

    #[test]
    fn review_decision_starts_on_the_plan_and_requires_scrolling_to_decisions() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, false)]);
        presentation.set_review_plan(
            (0..4)
                .map(|index| Line::from(format!("plan line {index}")))
                .collect(),
            60,
        );

        assert_eq!(presentation.review().cursor_line(), 0);
        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::KeepOpen,
            "Enter on the plan must not submit the first decision"
        );
        // The first Enter walked the cursor off the first plan row; keep
        // walking down through the remaining plan rows to the separator.
        assert_eq!(presentation.review().cursor_line(), 1);
        for _ in 0..3 {
            presentation.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10);
        }
        assert_eq!(presentation.review().cursor_line(), 4);
        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::KeepOpen,
            "Enter on the separator must not submit a decision"
        );
        // The Enter walked down into the decision block, which starts right
        // after the four plan rows and the separator.
        assert_eq!(presentation.review().cursor_line(), 5);
        assert_eq!(presentation.review().selected_option(), 0);
        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::Submit
        );
    }

    #[test]
    fn review_decision_vim_keys_navigate_the_document() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, false)]);

        presentation.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().cursor_line(), 1);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().cursor_line(), 0);

        // G jumps to the bottom of the document.
        presentation.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT), 10);
        assert_eq!(presentation.review().cursor_line(), 3);
        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::Submit
        );
    }

    #[test]
    fn review_decision_gg_returns_to_the_top() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, false)]);

        presentation.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().cursor_line(), 0);
    }

    #[test]
    fn review_decision_page_keys_step_by_page_size() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, false)]);
        presentation.set_review_plan(
            (0..40)
                .map(|index| Line::from(format!("line {index}")))
                .collect(),
            60,
        );

        // total = 40 plan rows + separator + 2 decision rows = 43.
        presentation.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().cursor_line(), 10);
        assert_eq!(presentation.review().scroll(), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().cursor_line(), 20);
        assert_eq!(presentation.review().scroll(), 20);
        presentation.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().cursor_line(), 10);
        assert_eq!(presentation.review().scroll(), 1);
        presentation.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), 10);
        assert_eq!(presentation.review().cursor_line(), 0);
        assert_eq!(presentation.review().scroll(), 0);
    }

    #[test]
    fn review_decision_feedback_row_edits_and_submits_text() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, true)]);

        // Walk to the custom feedback row (placeholder + separator + option).
        for _ in 0..4 {
            presentation.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10);
        }
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

        for _ in 0..4 {
            presentation.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10);
        }
        presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10);
        assert!(presentation.review().is_editing_custom());

        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 10),
            UserInputEffect::KeepOpen
        );
        assert!(!presentation.review().is_editing_custom());
        assert_eq!(presentation.review().selected_option(), 1);
        assert_eq!(presentation.review().cursor_line(), 4);
    }

    #[test]
    fn review_decision_enter_on_filled_feedback_row_submits() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, true)]);

        for _ in 0..4 {
            presentation.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 10);
        }
        presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE), 10);
        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::Submit
        );
        assert_eq!(presentation.review().custom_text(), "o");
        assert!(!presentation.review().is_editing_custom());
    }

    #[test]
    fn question_flow_page_keys_scroll_the_dialog() {
        let mut presentation =
            UserInputPresentation::new(overlay(false), vec![question(false, false)]);

        presentation.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), 10);
        assert_eq!(presentation.flow_scroll(), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), 5);
        assert_eq!(presentation.flow_scroll(), 5);
    }

    #[test]
    fn question_flow_ctrl_e_y_scroll_the_dialog_one_line_at_a_time() {
        let mut presentation =
            UserInputPresentation::new(overlay(false), vec![question(false, false)]);

        presentation.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL), 10);
        assert_eq!(presentation.flow_scroll(), 1);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL), 10);
        assert_eq!(presentation.flow_scroll(), 3);
        // Ctrl+Y scrolls back up one line at a time, never below zero.
        presentation.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL), 10);
        assert_eq!(presentation.flow_scroll(), 2);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL), 10);
        presentation.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL), 10);
        assert_eq!(presentation.flow_scroll(), 0);
    }

    #[test]
    fn review_overlay_opens_as_a_roomy_modal() {
        // The review dialog is meant to read a whole plan document: it targets
        // a wide column count and a minimum height even for short plans.
        let width = super::user_input_review_content_width(Rect::new(0, 0, 220, 60));
        assert!(
            width >= 100,
            "review width should be comfortably wide, got {width}"
        );
        assert_eq!(width, super::REVIEW_DECISION_TARGET_WIDTH - 2);

        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, false)]);
        presentation.set_review_plan(vec![Line::from("short plan")], width);
        let layout =
            super::review_decision_layout(&presentation, &I18n::default(), Rect::new(0, 0, 220, 60));
        assert!(
            layout.natural_height >= super::REVIEW_DECISION_MIN_HEIGHT,
            "a short plan should still open at least {} rows tall, got {}",
            super::REVIEW_DECISION_MIN_HEIGHT,
            layout.natural_height
        );
    }
}
