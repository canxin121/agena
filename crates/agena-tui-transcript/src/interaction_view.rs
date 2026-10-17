//! Live interaction content for pending user-input parts rendered inline in
//! the transcript.
//!
//! A pending interaction part (plan review or ask-user) renders as an
//! expandable Activity. When expanded and still awaiting a decision, the
//! transcript renders the plan body and the decision rows natively
//! ("everything is a part"): the plan body flows through the same Markdown
//! pipeline as every other part with the standard activity indent.
//!
//! Ask-user renders one question per page and an explicit answer-summary page.
//! Up/Down move the shared cursor; Left/Right change questions at controls.
//! Custom answers are selectable options. Rendered metadata owns hit testing.

use std::collections::BTreeMap;

use crate::{ToolCallView, TranscriptActivityContent, TranscriptEntryPart, TranscriptPartContent};

/// The `request_id` of a pending user-input record on a canonical `tool_call`,
/// or `None` when the part is not awaiting input. Only pending records are
/// interactive in the transcript, so the key router and inline renderer share
/// the same boundary.
pub fn interaction_request_id_for_part<'a>(part: &'a TranscriptEntryPart<'a>) -> Option<&'a str> {
    if part.status.is_terminal() {
        return None;
    }
    match &part.content {
        TranscriptPartContent::Activity(TranscriptActivityContent::Operation(operation))
            if !operation.operation.status().is_terminal() =>
        {
            operation
                .operation
                .user_input
                .awaiting()
                .next()
                .map(|record| record.request.request_id.as_str())
        }
        _ => None,
    }
}

/// Whether a projected tool operation is currently awaiting a user-input
/// reply. This is the canonical "pending interaction part" predicate for the
/// single-activity shape (a tool_call activity IS the ask).
pub fn operation_has_awaiting_user_input(operation: &ToolCallView) -> bool {
    !operation.operation.status().is_terminal()
        && operation.operation.user_input.awaiting().next().is_some()
}

/// The minimal request facts the layout helpers need. Implemented for both the
/// wire resource the renderer draws from and the Domain request the App holds,
/// so the single-source layout contract holds on both sides of the adapter.
pub trait InteractionRequestFacts {
    fn request_kind_is_review(&self) -> bool;
    fn question_count(&self) -> usize;
    fn options_len(&self, index: usize) -> usize;
    fn allow_custom(&self, index: usize) -> bool;
    fn custom_option_index(&self, index: usize) -> Option<usize>;
    fn multiple(&self, index: usize) -> bool;
    fn question_header(&self, index: usize) -> &str;
    fn question_text(&self, index: usize) -> &str;
    fn option_label(&self, index: usize, option: usize) -> &str;
}

impl InteractionRequestFacts for agena_api::resource::UserInputRequest {
    fn request_kind_is_review(&self) -> bool {
        self.kind == "review"
    }

    fn question_count(&self) -> usize {
        self.questions.len()
    }

    fn options_len(&self, index: usize) -> usize {
        self.questions.get(index).map_or(0, |q| q.options.len())
    }

    fn allow_custom(&self, index: usize) -> bool {
        self.custom_option_index(index).is_some()
    }

    fn custom_option_index(&self, index: usize) -> Option<usize> {
        let question = self.questions.get(index)?;
        question
            .options
            .iter()
            .position(|option| option.custom)
            .or_else(|| question.allow_custom.then_some(question.options.len()))
    }

    fn multiple(&self, index: usize) -> bool {
        self.questions.get(index).is_some_and(|q| q.multiple)
    }

    fn question_header(&self, index: usize) -> &str {
        &self.questions[index].header
    }
    fn question_text(&self, index: usize) -> &str {
        &self.questions[index].question
    }
    fn option_label(&self, index: usize, option: usize) -> &str {
        self.questions[index]
            .options
            .get(option)
            .map_or("", |o| o.label.as_str())
    }
}

impl InteractionRequestFacts for agena_domain::UserInputRequest {
    fn request_kind_is_review(&self) -> bool {
        self.kind == agena_domain::UserInputKind::Review
    }

    fn question_count(&self) -> usize {
        self.questions.len()
    }

    fn options_len(&self, index: usize) -> usize {
        self.questions.get(index).map_or(0, |q| q.options.len())
    }

    fn allow_custom(&self, index: usize) -> bool {
        self.custom_option_index(index).is_some()
    }

    fn custom_option_index(&self, index: usize) -> Option<usize> {
        self.questions.get(index)?.custom_option_index()
    }

    fn multiple(&self, index: usize) -> bool {
        self.questions.get(index).is_some_and(|q| q.multiple)
    }

    fn question_header(&self, index: usize) -> &str {
        &self.questions[index].header
    }
    fn question_text(&self, index: usize) -> &str {
        &self.questions[index].question
    }
    fn option_label(&self, index: usize, option: usize) -> &str {
        self.questions[index]
            .options
            .get(option)
            .map_or("", |o| o.label.as_str())
    }
}

/// Whether a user-input request renders as a single-question review decision
/// (plan approval) rather than the multi-question ask-user flow. Both the
/// renderer and the App's key routing derive the layout kind from this, so
/// they always agree on which body shape a request renders.
pub fn request_is_review_decision<R: InteractionRequestFacts>(request: &R) -> bool {
    request.request_kind_is_review()
        && request.question_count() == 1
        && !request.multiple(0)
        && request.options_len(0) > 0
        && request
            .custom_option_index(0)
            .is_none_or(|index| index >= request.options_len(0))
}

/// Live per-question answer snapshot for the ask-user flow.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingInteractionAnswerView {
    /// Picked option indexes.
    pub picked: Vec<usize>,
    /// Committed custom values.
    pub custom_values: Vec<String>,
}

impl PendingInteractionAnswerView {
    /// Whether the question has any committed answer (a picked option or a
    /// custom value), which adds the answered-preview row to its block.
    pub fn is_answered(&self) -> bool {
        !self.picked.is_empty() || !self.custom_values.is_empty()
    }
}

/// Live selection/answer state the App hands the renderer for an expanded
/// pending interaction part. Carries ONLY selection state — the plan body and
/// decision labels come from the wire `request`, which the renderer already
/// has in scope, so no pre-rendered lines are needed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingInteractionView {
    /// Ask page: 0..question_count shows one question; question_count is review.
    pub question_page: usize,
    /// Review: trimmed custom feedback text.
    pub custom_text: String,
    /// Review: raw (untrimmed) editor draft, shown on the inline editor row.
    pub custom_draft: String,
    /// Review/ask-user: whether the inline custom editor is open.
    pub editing_custom: bool,
    /// Review/ask-user: editor cursor byte offset, for the inline caret.
    pub custom_cursor: usize,
    /// Ask-user: question whose custom slot is showing the inline editor.
    pub editing_question: Option<usize>,
    /// Ask-user: per-question answer markers.
    pub answers: BTreeMap<usize, PendingInteractionAnswerView>,
}

/// Semantic kind of a body line in an expanded pending interaction part, used
/// by the App's thin key layer to decide whether a key acts specially on the
/// control under the cursor. Wrapped labels and descriptions share one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionLineKind {
    PlanBody,
    Separator,
    ReviewOption {
        option_index: usize,
    },
    ReviewCustomLabel,
    ReviewEditor,
    AskPlanBody,
    AskSeparator,
    AskQuestionHeader {
        question_index: usize,
    },
    AskQuestionText {
        question_index: usize,
    },
    AskOption {
        question_index: usize,
        option_index: usize,
    },
    AskCustomRow {
        question_index: usize,
    },
    AskCustomEditor {
        question_index: usize,
    },
    AskCustomDetail {
        question_index: usize,
    },
    AskAnsweredPreview {
        question_index: usize,
    },
    AskFooter,
    AskPageHeader,
    AskSummary {
        question_index: usize,
    },
    AskPrevious,
    AskNext,
    AskSubmit,
    AskCancel,
}

/// Wrap every answer in full, including long custom values and line breaks.
/// Every wrapped row belongs to the same editable answer card.
pub fn ask_user_summary_lines<R: InteractionRequestFacts>(
    request: &R,
    index: usize,
    answer: Option<&PendingInteractionAnswerView>,
    width: u16,
) -> Vec<String> {
    let header = request.question_header(index);
    let question = request.question_text(index);
    let mut values: Vec<&str> = answer
        .into_iter()
        .flat_map(|a| {
            a.picked
                .iter()
                .filter(|&&option| Some(option) != request.custom_option_index(index))
                .map(|&o| request.option_label(index, o))
                .chain(
                    a.custom_values
                        .iter()
                        .filter(|_| {
                            request
                                .custom_option_index(index)
                                .is_some_and(|custom| a.picked.contains(&custom))
                        })
                        .map(String::as_str),
                )
        })
        .collect();
    if values.is_empty() {
        values.push("—");
    }
    let title = if header.trim().is_empty() {
        format!("{}. {question}", index + 1)
    } else {
        format!("{}. {header}\n{question}", index + 1)
    };
    let text = format!("{title}\n{}", values.join(", "));
    text.split('\n')
        .flat_map(|line| {
            textwrap::wrap(
                line,
                textwrap::Options::new(usize::from(width.saturating_sub(4)).max(1))
                    .break_words(true),
            )
            .into_iter()
            .map(|s| s.into_owned())
            .collect::<Vec<_>>()
        })
        .collect()
}

impl InteractionLineKind {
    /// Whether Enter on this line is an interaction decision (a review option
    /// row, the review custom label/editor, an ask option row or an ask custom
    /// row) rather than a plain node toggle.
    pub fn is_submit_eligible(self) -> bool {
        matches!(
            self,
            InteractionLineKind::ReviewOption { .. }
                | InteractionLineKind::ReviewCustomLabel
                | InteractionLineKind::ReviewEditor
                | InteractionLineKind::AskSubmit
        )
    }
}
