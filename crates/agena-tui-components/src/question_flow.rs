//! Question flow state machine (multi-step prompts).

use crate::selection::SelectionCursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionFlowScreen {
    Question,
    Review,
}

#[derive(Debug, Clone)]
pub struct QuestionFlowState {
    question: SelectionCursor,
    option: SelectionCursor,
    screen: QuestionFlowScreen,
}

impl Default for QuestionFlowState {
    fn default() -> Self {
        Self {
            question: SelectionCursor::default(),
            option: SelectionCursor::default(),
            screen: QuestionFlowScreen::Question,
        }
    }
}

impl QuestionFlowState {
    pub fn new(
        selected_question: usize,
        selected_option: usize,
        screen: QuestionFlowScreen,
    ) -> Self {
        Self {
            question: SelectionCursor::new(selected_question),
            option: SelectionCursor::new(selected_option),
            screen,
        }
    }

    pub fn screen(&self) -> QuestionFlowScreen {
        self.screen
    }

    pub fn set_screen(&mut self, screen: QuestionFlowScreen) {
        self.screen = screen;
    }

    pub fn selected_question(&self) -> usize {
        self.question.selected
    }

    pub fn selected_option(&self) -> usize {
        self.option.selected
    }

    pub fn set_selected_question(&mut self, index: usize) {
        self.question.selected = index;
    }

    pub fn set_selected_option(&mut self, index: usize) {
        self.option.selected = index;
    }

    pub fn clear(&mut self) {
        self.question.selected = 0;
        self.option.selected = 0;
        self.screen = QuestionFlowScreen::Question;
    }

    pub fn focus_question(&mut self, index: usize, question_count: usize) {
        if question_count == 0 {
            self.clear();
            return;
        }
        self.screen = QuestionFlowScreen::Question;
        self.question.selected = index;
        self.question.clamp(question_count);
    }

    pub fn focus_review(&mut self, question_count: usize) {
        if question_count == 0 {
            self.clear();
            return;
        }
        self.screen = QuestionFlowScreen::Review;
        self.question.clamp(question_count);
    }

    pub fn move_question(&mut self, question_count: usize, delta: isize) {
        if question_count == 0 {
            self.clear();
            return;
        }
        self.question.move_by(question_count, delta);
    }

    pub fn move_question_page(&mut self, question_count: usize, delta: isize, page_size: usize) {
        if question_count == 0 {
            self.clear();
            return;
        }
        self.question.move_page(question_count, delta, page_size);
    }

    pub fn move_question_home(&mut self, question_count: usize) {
        if question_count == 0 {
            self.clear();
            return;
        }
        self.question.move_home();
    }

    pub fn move_question_end(&mut self, question_count: usize) {
        if question_count == 0 {
            self.clear();
            return;
        }
        self.question.move_end(question_count);
    }

    pub fn clamp_questions(&mut self, question_count: usize) {
        if question_count == 0 {
            self.clear();
            return;
        }
        self.question.clamp(question_count);
    }

    pub fn move_option(&mut self, option_count: usize, delta: isize) {
        self.option.move_by(option_count, delta);
    }

    pub fn move_option_home(&mut self) {
        self.option.move_home();
    }

    pub fn move_option_end(&mut self, option_count: usize) {
        self.option.move_end(option_count);
    }

    pub fn clamp_options(&mut self, option_count: usize) {
        self.option.clamp(option_count);
    }
}
