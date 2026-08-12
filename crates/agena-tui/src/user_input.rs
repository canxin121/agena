//! Presentation state and reduction for interactive user-input overlays.

use std::collections::{BTreeMap, BTreeSet};

use crossterm::event::KeyEvent;
use ratatui::text::{Line, Span};

use crate::keymap::{KeyAction, KeyContext, resolve};
use agena_tui_components::{Editor, QuestionFlowScreen, QuestionFlowState};
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
        if self.review_decision {
            // Plan-review custom feedback lives in the review editor, not the
            // question-flow one; route pasted text there directly so a
            // subsequent submit picks it up via `review().custom_text()`.
            let Some(question) = self.questions.first() else {
                return false;
            };
            if !question.allow_custom {
                return false;
            }
            if !self.review.editing_custom {
                self.begin_review_custom_edit();
            }
            self.review.custom_input.insert_str(text);
            return true;
        }
        if self.state.screen() == QuestionFlowScreen::Review {
            self.focus_question(self.state.selected_question());
        }
        if !self.editing_custom && !self.begin_custom_edit() {
            return false;
        }
        self.custom_input.insert_str(text);
        true
    }

    /// Route a key into the inline custom editor (native part path). The
    /// editor is already open — the App routed the transcript key stream here
    /// via `interaction_editing` — so every key drives the underlying editor
    /// directly; Close/Submit are reported back for the App to manage the
    /// editor lifecycle. Mirrors the overlay's `handle_review_custom_key` /
    /// `handle_custom_input_key` arms without a page-size parameter.
    pub fn handle_custom_edit_key(&mut self, key: KeyEvent) -> UserInputEffect {
        if self.review_decision {
            return self.handle_review_custom_key(key);
        }
        self.handle_custom_input_key(key)
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

    /// Open the review custom-feedback editor, positioning the review cursor
    /// on the custom label row. Returns whether editing actually began (the
    /// review question must allow custom feedback). Called by the overlay's
    /// Enter-on-empty-custom path and by the native part's inline editor.
    pub fn begin_review_custom_edit(&mut self) -> bool {
        let Some(question) = self.questions.first() else {
            return false;
        };
        if !question.allow_custom {
            return false;
        }
        self.review.selected_option = question.options.len();
        let custom_label_row = self
            .review_decision_start()
            .saturating_add(question.options.len().saturating_mul(2));
        self.review.cursor_line = custom_label_row;
        self.review.editing_custom = true;
        true
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

    /// Toggle option `option_index` of question `question_index` in the
    /// ask-user flow, as if the overlay cursor were on that row. Public for
    /// the native part's decision-row Enter, which derives the option from the
    /// transcript cursor instead of a presentation cursor.
    pub fn toggle_option_at(&mut self, question_index: usize, option_index: usize) {
        let Some(question) = self.questions.get(question_index) else {
            return;
        };
        if option_index >= question.options.len() {
            return;
        }
        self.state.focus_question(question_index, self.questions.len());
        self.state.set_selected_option(option_index);
        self.toggle_option();
    }

    /// Select option `option_index` of question `question_index` in the
    /// ask-user flow, clearing any previously picked option/custom value of
    /// that question (single-pick semantics).
    pub fn select_option_at(&mut self, question_index: usize, option_index: usize) {
        let Some(question) = self.questions.get(question_index) else {
            return;
        };
        if option_index >= question.options.len() {
            return;
        }
        self.state.focus_question(question_index, self.questions.len());
        self.state.set_selected_option(option_index);
        self.select_option();
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
        Line, UserInputEffect, UserInputOptionPresentation, UserInputPresentation,
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
    fn review_paste_lands_in_the_review_editor_and_submits() {
        // The review-decision flow keeps its custom feedback in the review
        // editor, not the question-flow one. Pasting onto an expanded review
        // part must populate `review().custom_text()` so a later submit reads
        // it back. (Regression: paste wrote to the question-flow editor and
        // the feedback was silently dropped.)
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, true)]);

        assert!(presentation.insert_custom_text("looks good"));
        assert_eq!(presentation.review().custom_text(), "looks good");
        assert_eq!(
            presentation.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10),
            UserInputEffect::Submit
        );
    }

    #[test]
    fn review_paste_is_rejected_when_custom_feedback_is_not_allowed() {
        let mut presentation =
            UserInputPresentation::new(overlay(true), vec![question(false, false)]);

        assert!(!presentation.insert_custom_text("nope"));
        assert!(presentation.review().custom_text().is_empty());
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
}
