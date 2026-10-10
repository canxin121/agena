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
//! Left/Right changes pages; the transcript cursor remains the option/action
//! cursor. Up/Down can still scroll through and leave the part.
//!
//! Shared layout helpers keep the renderer and input routing in agreement.

use std::collections::BTreeMap;

use agena_tui::i18n::I18n;

use crate::{
    ToolCallView, TranscriptActivityContent, TranscriptEntryPart, TranscriptPartContent,
    renderer::push_markdown_document,
};

/// The `request_id` of a pending user-input record on a canonical `tool_call`,
/// or `None` when the part is not awaiting input. Only pending records are
/// interactive in the transcript, so the key router and inline renderer share
/// the same boundary.
pub fn interaction_request_id_for_part<'a>(part: &'a TranscriptEntryPart<'a>) -> Option<&'a str> {
    match &part.content {
        TranscriptPartContent::Activity(TranscriptActivityContent::Operation(operation)) => {
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
    operation.operation.user_input.awaiting().next().is_some()
}

/// The minimal request facts the layout helpers need. Implemented for both the
/// wire resource the renderer draws from and the Domain request the App holds,
/// so the single-source layout contract holds on both sides of the adapter.
pub trait InteractionRequestFacts {
    fn request_kind_is_review(&self) -> bool;
    fn question_count(&self) -> usize;
    fn options_len(&self, index: usize) -> usize;
    fn allow_custom(&self, index: usize) -> bool;
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
        self.questions.get(index).is_some_and(|q| q.allow_custom)
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
        self.questions.get(index).is_some_and(|q| q.allow_custom)
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
    /// Wrapped summary row counts at `plan_width`, used for cursor routing.
    pub summary_rows: Vec<usize>,
    /// Review: index of the selected decision option (label under the cursor).
    /// `None` while the cursor is on the plan body.
    pub selected_option: Option<usize>,
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
    /// Cached plan-body row count at `plan_width` (single source for the app's
    /// key routing).
    pub plan_body_lines: usize,
    /// Width the plan body was measured at.
    pub plan_width: u16,
}

/// Layout facts of one question used by the row classifier and the wizard
/// layout helpers: how many rows its block occupies and whether it can carry
/// an answer marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionQuestionLayout {
    pub options_len: usize,
    pub allow_custom: bool,
    pub multiple: bool,
    pub answered: bool,
}

/// Semantic kind of a body line in an expanded pending interaction part, used
/// by the App's thin key layer to decide whether a key acts specially on the
/// line under the cursor. Review keeps one row per option; ask-user renders
/// one question page or the summary page, with explicit navigation actions.
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
/// This same function measures the app layout and supplies the rendered rows.
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
                .map(|&o| request.option_label(index, o))
                .chain(a.custom_values.iter().map(String::as_str))
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

/// The current page has its own row budget; summaries use measured rows.
pub fn ask_user_page_rows(
    plan: usize,
    layouts: &[InteractionQuestionLayout],
    page: usize,
    summary_rows: &[usize],
) -> usize {
    plan + 2
        + layouts
            .get(page)
            .map_or(summary_rows.iter().sum::<usize>() + 3, |layout| {
                ask_user_question_block_rows(layout) + 2
            })
}

pub fn ask_user_page_landing_offset(
    plan: usize,
    layouts: &[InteractionQuestionLayout],
    page: usize,
) -> usize {
    plan + 2
        + usize::from(
            layouts
                .get(page)
                .is_some_and(|q| q.options_len > 0 || q.allow_custom),
        ) * 2
}

pub fn classify_ask_user_page(
    layouts: &[InteractionQuestionLayout],
    plan: usize,
    page: usize,
    body_offset: usize,
    editing: bool,
    summary_rows: &[usize],
) -> InteractionLineKind {
    use InteractionLineKind::*;
    if body_offset < plan {
        return AskPlanBody;
    }
    if body_offset == plan {
        return AskSeparator;
    }
    if body_offset == plan + 1 {
        return AskPageHeader;
    }
    let offset = body_offset - plan - 2;
    if let Some(layout) = layouts.get(page) {
        let rows = ask_user_question_block_rows(layout);
        if offset < rows {
            return classify_ask_question_line(layout, page, offset, editing);
        }
        return match offset - rows {
            0 => AskPrevious,
            1 => AskNext,
            _ => AskFooter,
        };
    }
    let mut remaining = offset;
    for (question_index, &rows) in summary_rows.iter().enumerate() {
        if remaining < rows {
            return AskSummary { question_index };
        }
        remaining -= rows;
    }
    match remaining {
        0 => AskPrevious,
        1 => AskSubmit,
        2 => AskCancel,
        _ => AskFooter,
    }
}

/// Plan-body row count at `width` using the EXACT renderer path
/// (`push_markdown_document` with the `"    "` body prefix). The renderer and
/// the app both derive layout from this, so they can never drift. The plan
/// body can contain math, which needs a render context; the app calls this
/// outside the transcript's render context, so it establishes the same
/// text-math fallback the export path uses.
pub fn interaction_plan_body_lines(body_markdown: &str, width: u16) -> usize {
    agena_tui_media::with_text_math_rendering(|| {
        let mut out = Vec::new();
        push_markdown_document(&mut out, "    ", body_markdown, width, &I18n::english());
        out.len()
    })
}

/// Body offset (0 = first body line after the activity headline) where the
/// review decision block begins: plan body + separator.
pub fn review_decision_region_start(plan_body_lines: usize) -> usize {
    plan_body_lines.saturating_add(1)
}

/// Number of decision-block rows the review renders: one row per option plus
/// the custom-feedback label row when allowed. The review keeps ONE row per
/// option (marker + label, no description detail line) and no custom-detail
/// row, so the classifier and the renderer share this exact budget.
pub fn review_decision_rows_count(options_len: usize, allow_custom: bool) -> usize {
    options_len + usize::from(allow_custom)
}

/// Maps an offset within a review decision block (0 = first option label row)
/// to the selected option index: each offset IS one option row (index =
/// decision_offset); the custom label maps to `options_len` when `allow_custom`.
pub fn review_selected_option_for_offset(
    options_len: usize,
    allow_custom: bool,
    decision_offset: usize,
) -> Option<usize> {
    if allow_custom && decision_offset == options_len {
        return Some(options_len);
    }
    (decision_offset < options_len).then_some(decision_offset)
}

/// Whether a decision-block offset is on the custom feedback label row.
pub fn review_offset_is_custom_label(
    options_len: usize,
    allow_custom: bool,
    decision_offset: usize,
) -> bool {
    allow_custom && decision_offset == options_len
}

/// The per-question layout facts the classifier needs, derived from the
/// request and the live answer snapshot. Both the renderer (which draws the
/// answered-preview rows) and the App (which routes keys) build this the same
/// way, so the classifier's row arithmetic always matches the rendered body.
pub fn interaction_question_layouts<R: InteractionRequestFacts>(
    request: &R,
    answers: &BTreeMap<usize, PendingInteractionAnswerView>,
) -> Vec<InteractionQuestionLayout> {
    (0..request.question_count())
        .map(|index| InteractionQuestionLayout {
            options_len: request.options_len(index),
            allow_custom: request.allow_custom(index),
            multiple: request.multiple(index),
            answered: answers
                .get(&index)
                .is_some_and(PendingInteractionAnswerView::is_answered),
        })
        .collect()
}

/// Body rows one ask-user question block occupies on its page:
/// header row + question-text row + ONE row per option + 2 rows per custom slot
/// (label + detail) + an answered-preview row. The renderer draws with exactly
/// this budget and the reconciliation test asserts it, so the layout contract
/// can never drift.
pub fn ask_user_question_block_rows(layout: &InteractionQuestionLayout) -> usize {
    2 + layout.options_len + usize::from(layout.allow_custom) * 2 + usize::from(layout.answered)
}

/// Full review classifier: given the per-question layout, plan row count and
/// the body offset, which semantic row the cursor is on. Both the app (key
/// routing) and the renderer's reconciliation tests use this.
pub fn classify_interaction_line(
    questions: &[InteractionQuestionLayout],
    plan_body_lines: usize,
    body_offset: usize,
    editing_custom: bool,
) -> InteractionLineKind {
    let question = match questions.first() {
        Some(question) => *question,
        None => return InteractionLineKind::PlanBody,
    };
    if body_offset < plan_body_lines {
        return InteractionLineKind::PlanBody;
    }
    if body_offset == plan_body_lines {
        return InteractionLineKind::Separator;
    }
    let decision_offset = body_offset
        .saturating_sub(plan_body_lines)
        .saturating_sub(1);
    // Review renders ONE row per option (marker + label) plus the custom
    // label row; the trailing footer-hint row and anything beyond classify as
    // PlanBody so Enter there never submits.
    if question.allow_custom && decision_offset == question.options_len {
        return if editing_custom {
            InteractionLineKind::ReviewEditor
        } else {
            InteractionLineKind::ReviewCustomLabel
        };
    }
    if decision_offset < question.options_len {
        return InteractionLineKind::ReviewOption {
            option_index: decision_offset,
        };
    }
    InteractionLineKind::PlanBody
}

/// Classify a row inside one question block (header, text, options, custom).
fn classify_ask_question_line(
    layout: &InteractionQuestionLayout,
    q: usize,
    mut offset: usize,
    editing: bool,
) -> InteractionLineKind {
    use InteractionLineKind::*;
    if offset == 0 {
        return AskQuestionHeader { question_index: q };
    }
    if offset == 1 {
        return AskQuestionText { question_index: q };
    }
    offset -= 2;
    if offset < layout.options_len {
        return AskOption {
            question_index: q,
            option_index: offset,
        };
    }
    offset -= layout.options_len;
    if layout.allow_custom {
        if offset == 0 {
            return AskCustomRow { question_index: q };
        }
        if offset == 1 {
            return if editing {
                AskCustomEditor { question_index: q }
            } else {
                AskCustomDetail { question_index: q }
            };
        }
    }
    AskAnsweredPreview { question_index: q }
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

#[cfg(test)]
mod tests {
    use super::{
        InteractionLineKind, InteractionQuestionLayout, ask_user_page_landing_offset,
        ask_user_page_rows, ask_user_question_block_rows, classify_ask_user_page,
        classify_interaction_line, interaction_plan_body_lines, review_decision_region_start,
        review_decision_rows_count, review_offset_is_custom_label,
        review_selected_option_for_offset,
    };

    fn question(
        options_len: usize,
        allow_custom: bool,
        multiple: bool,
        answered: bool,
    ) -> InteractionQuestionLayout {
        InteractionQuestionLayout {
            options_len,
            allow_custom,
            multiple,
            answered,
        }
    }

    #[test]
    fn review_decision_region_starts_after_the_plan_and_separator() {
        assert_eq!(review_decision_region_start(3), 4);
        assert_eq!(review_decision_region_start(0), 1);
    }

    #[test]
    fn review_selected_option_maps_each_row_and_the_custom_slot() {
        // One row per option: offsets 0, 1 are the options, 2 is custom.
        assert_eq!(review_selected_option_for_offset(2, true, 0), Some(0));
        assert_eq!(review_selected_option_for_offset(2, true, 1), Some(1));
        assert_eq!(review_selected_option_for_offset(2, true, 2), Some(2));
        // Without custom, offset 2 is out of range.
        assert_eq!(review_selected_option_for_offset(2, false, 2), None);
        assert_eq!(review_selected_option_for_offset(2, false, 3), None);
        assert!(review_offset_is_custom_label(2, true, 2));
        assert!(!review_offset_is_custom_label(2, true, 1));
        assert!(!review_offset_is_custom_label(2, false, 2));
        // The row budget is one per option plus the optional custom label.
        assert_eq!(review_decision_rows_count(2, true), 3);
        assert_eq!(review_decision_rows_count(2, false), 2);
        assert_eq!(review_decision_rows_count(0, true), 1);
    }

    #[test]
    fn classify_review_rows_covers_every_decision_row() {
        let layouts = [question(2, true, false, false)];
        let plan = 3;
        // Plan rows and separator.
        assert_eq!(
            classify_interaction_line(&layouts, plan, 0, false),
            InteractionLineKind::PlanBody
        );
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan, false),
            InteractionLineKind::Separator
        );
        // One row per option: option 0 at plan+1, option 1 at plan+2.
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 1, false),
            InteractionLineKind::ReviewOption { option_index: 0 }
        );
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 2, false),
            InteractionLineKind::ReviewOption { option_index: 1 }
        );
        // Custom label at plan+3 (editor while editing), then the footer hint
        // row and beyond classify as PlanBody (never submit-eligible).
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 3, false),
            InteractionLineKind::ReviewCustomLabel
        );
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 3, true),
            InteractionLineKind::ReviewEditor
        );
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 4, false),
            InteractionLineKind::PlanBody
        );
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 5, false),
            InteractionLineKind::PlanBody
        );
    }

    #[test]
    fn ask_user_block_rows_covers_the_full_block_budget() {
        // One question: header + text + options (ONE row per option) + 2 per
        // custom slot + answered preview.
        assert_eq!(
            ask_user_question_block_rows(&question(0, false, false, false)),
            2
        );
        assert_eq!(
            ask_user_question_block_rows(&question(2, true, false, false)),
            2 + 2 + 2
        );
        assert_eq!(
            ask_user_question_block_rows(&question(2, true, false, true)),
            2 + 2 + 2 + 1
        );
        // Unanswered with no custom slot: 2 + options.
        assert_eq!(
            ask_user_question_block_rows(&question(3, false, true, false)),
            2 + 3
        );
    }

    #[test]
    fn ask_pages_expose_only_the_current_question_and_summary_actions() {
        use InteractionLineKind::*;
        let layouts = [
            question(2, true, false, true),
            question(30, false, true, false),
        ];
        let summaries = [5, 3];
        let first = ask_user_page_rows(2, &layouts, 0, &summaries);
        assert_eq!(first, 13);
        assert_eq!(
            classify_ask_user_page(&layouts, 2, 0, 6, false, &summaries),
            AskOption {
                question_index: 0,
                option_index: 0
            }
        );
        assert_eq!(
            classify_ask_user_page(&layouts, 2, 0, first - 1, false, &summaries),
            AskNext
        );
        assert_eq!(ask_user_page_landing_offset(0, &layouts, 1), 4);
        assert_eq!(
            classify_ask_user_page(&layouts, 0, 1, 33, false, &summaries),
            AskOption {
                question_index: 1,
                option_index: 29
            }
        );
        assert_eq!(ask_user_page_rows(0, &layouts, 2, &summaries), 13);
        assert_eq!(
            classify_ask_user_page(&layouts, 0, 2, 6, false, &summaries),
            AskSummary { question_index: 0 }
        );
        assert_eq!(
            classify_ask_user_page(&layouts, 0, 2, 7, false, &summaries),
            AskSummary { question_index: 1 }
        );
        assert_eq!(
            classify_ask_user_page(&layouts, 0, 2, 11, false, &summaries),
            AskSubmit
        );
        assert_eq!(
            classify_ask_user_page(&layouts, 0, 2, 12, false, &summaries),
            AskCancel
        );
    }

    #[test]
    fn submit_eligibility_is_restricted_to_decision_rows() {
        assert!(InteractionLineKind::ReviewOption { option_index: 0 }.is_submit_eligible());
        assert!(InteractionLineKind::ReviewCustomLabel.is_submit_eligible());
        assert!(InteractionLineKind::ReviewEditor.is_submit_eligible());
        assert!(
            !InteractionLineKind::AskOption {
                question_index: 0,
                option_index: 0
            }
            .is_submit_eligible()
        );
        assert!(!InteractionLineKind::AskCustomRow { question_index: 0 }.is_submit_eligible());
        assert!(InteractionLineKind::AskSubmit.is_submit_eligible());
        assert!(!InteractionLineKind::PlanBody.is_submit_eligible());
        assert!(!InteractionLineKind::Separator.is_submit_eligible());
        assert!(!InteractionLineKind::AskPlanBody.is_submit_eligible());
        assert!(!InteractionLineKind::AskQuestionHeader { question_index: 0 }.is_submit_eligible());
        assert!(!InteractionLineKind::AskCustomDetail { question_index: 0 }.is_submit_eligible());
        assert!(!InteractionLineKind::AskFooter.is_submit_eligible());
    }

    #[test]
    fn plan_body_lines_counts_exactly_what_the_renderer_draws() {
        // A single heading renders as one row; a long paragraph wraps at the
        // available content width (width minus the 4-char body indent).
        assert_eq!(interaction_plan_body_lines("## One", 40), 1);
        let long = "word ".repeat(20); // 100 chars
        let rows = interaction_plan_body_lines(&long, 10);
        // 5 chars per "word " and a 6-char content width → one word per row.
        assert_eq!(rows, 20, "each 5-char word occupies one 6-char row");
    }
}
