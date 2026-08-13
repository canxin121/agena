//! Live interaction content for pending user-input parts rendered inline in
//! the transcript.
//!
//! A pending interaction part (plan review or ask-user) renders as an
//! expandable Activity. When expanded and still awaiting a decision, the
//! transcript renders the plan body and the decision rows natively
//! ("everything is a part"): the plan body flows through the same Markdown
//! pipeline as every other part with the standard activity indent.
//!
//! The two flows drive their interaction the same way. Plan review keeps the
//! transcript cursor IS the review cursor — which decision row the cursor sits
//! on is the selected option. Ask-user renders every question as one continuous
//! body (plan + separator + all question blocks + footer, no paging, no summary
//! page) and the transcript cursor IS the option cursor too: the line it sits
//! on derives which question and option the Space/Enter keys act on. The whole
//! body is drawn at once and the cursor is a whole-line highlight, so Up/Down
//! are ordinary transcript motion and can always leave the part.
//!
//! This module owns the small projections the renderer and the app share so
//! they can never drift: the live selection snapshot ([`PendingInteractionView`])
//! handed from the App to the renderer, and the single-source layout helpers
//! ([`interaction_plan_body_lines`], [`classify_interaction_line`],
//! [`classify_ask_user_line`], …) that both the renderer and the App's key
//! routing derive from.

use std::collections::BTreeMap;

use crate::{
    RequestPartResource, TranscriptActivityContent, TranscriptEntryPart, TranscriptPartContent,
    renderer::push_markdown_document,
};

/// The `request_id` of a pending user-input interaction part, or `None` when
/// the part is not an interaction or has already been answered. Only pending
/// parts are interactive in the transcript, so the key router and the inline
/// renderer agree on this boundary.
pub fn interaction_request_id_for_part<'a>(
    part: &'a TranscriptEntryPart<'a>,
) -> Option<&'a str> {
    match &part.content {
        TranscriptPartContent::Activity(TranscriptActivityContent::Request(request)) => {
            match request.as_ref() {
                RequestPartResource::UserInput { request, reply } => {
                    reply.is_none().then_some(request.request_id.as_str())
                }
            }
        }
        // The canonical single-activity shape: the ask lives inside the tool
        // operation's `user_input` records. Only a still-awaiting record makes
        // the part interactive — an answered operation's record has a reply,
        // so `awaiting()` is empty and the part is no longer an interaction
        // surface (it stays expandable, read-only).
        TranscriptPartContent::Activity(TranscriptActivityContent::Operation(operation)) => {
            operation
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
pub fn operation_has_awaiting_user_input(operation: &agena_api::part::OperationPartResource) -> bool {
    operation.user_input.awaiting().next().is_some()
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
    /// Ask-user: which question's custom slot is showing the inline editor (the
    /// single continuous body renders every question, so it needs to know which
    /// block to replace its detail row with). `None` when no editor is open.
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
/// every question's block in one continuous body, with its own row kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionLineKind {
    PlanBody,
    Separator,
    ReviewOption { option_index: usize },
    ReviewCustomLabel,
    ReviewEditor,
    AskPlanBody,
    AskSeparator,
    AskQuestionHeader { question_index: usize },
    AskQuestionText { question_index: usize },
    AskOption { question_index: usize, option_index: usize },
    AskCustomRow { question_index: usize },
    AskCustomEditor { question_index: usize },
    AskCustomDetail { question_index: usize },
    AskAnsweredPreview { question_index: usize },
    AskFooter,
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
        push_markdown_document(&mut out, "    ", body_markdown, width);
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

/// Body rows one ask-user question block occupies in the continuous body:
/// header row + question-text row + ONE row per option + 2 rows per custom slot
/// (label + detail) + an answered-preview row. The renderer draws with exactly
/// this budget and the reconciliation test asserts it, so the layout contract
/// can never drift.
pub fn ask_user_question_block_rows(layout: &InteractionQuestionLayout) -> usize {
    2 + layout.options_len
        + usize::from(layout.allow_custom) * 2
        + usize::from(layout.answered)
}

/// Total body rows of the continuous ask-user body: plan body + separator +
/// every question block + the footer key-hint row.
pub fn ask_user_body_rows(
    plan_body_lines: usize,
    layouts: &[InteractionQuestionLayout],
) -> usize {
    plan_body_lines
        + 1
        + layouts.iter().map(ask_user_question_block_rows).sum::<usize>()
        + 1
}

/// Body offset where a question's block begins: plan + separator + the blocks
/// of all earlier questions.
pub fn ask_user_question_body_start(
    plan_body_lines: usize,
    layouts: &[InteractionQuestionLayout],
    index: usize,
) -> usize {
    plan_body_lines
        + 1
        + layouts[..index].iter().map(ask_user_question_block_rows).sum::<usize>()
}

/// Body offset to land the cursor on for a question: its first option row, or
/// its header row when it has no options. Used by the Left/Right question jump
/// and the Enter validation jump.
pub fn ask_user_question_landing_offset(
    plan_body_lines: usize,
    layouts: &[InteractionQuestionLayout],
    index: usize,
) -> usize {
    let start = ask_user_question_body_start(plan_body_lines, layouts, index);
    if layouts[index].options_len > 0 {
        start + 2
    } else {
        start
    }
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
    let decision_offset = body_offset.saturating_sub(plan_body_lines).saturating_sub(1);
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

/// Ask-user classifier: given every question's layout, the plan row count and
/// the body offset, which semantic row of the continuous body the cursor is on
/// (plan → separator → each question's block → footer). Shares the exact row
/// arithmetic with the renderer via [`ask_user_question_block_rows`], so the
/// App's key routing can never drift from what the user sees.
pub fn classify_ask_user_line(
    questions: &[InteractionQuestionLayout],
    plan_body_lines: usize,
    body_offset: usize,
    editing_custom: bool,
) -> InteractionLineKind {
    if body_offset < plan_body_lines {
        return InteractionLineKind::AskPlanBody;
    }
    if body_offset == plan_body_lines {
        return InteractionLineKind::AskSeparator;
    }
    let mut remaining = body_offset.saturating_sub(plan_body_lines).saturating_sub(1);
    for (q, layout) in questions.iter().enumerate() {
        let block_rows = ask_user_question_block_rows(layout);
        if remaining >= block_rows {
            remaining -= block_rows;
            continue;
        }
        // Within a question block: header, text, options, custom label+detail,
        // answered preview.
        if remaining == 0 {
            return InteractionLineKind::AskQuestionHeader {
                question_index: q,
            };
        }
        remaining -= 1;
        if remaining == 0 {
            return InteractionLineKind::AskQuestionText { question_index: q };
        }
        remaining -= 1;
        if remaining < layout.options_len {
            return InteractionLineKind::AskOption {
                question_index: q,
                option_index: remaining,
            };
        }
        remaining -= layout.options_len;
        if layout.allow_custom {
            if remaining == 0 {
                return InteractionLineKind::AskCustomRow { question_index: q };
            }
            remaining -= 1;
            if remaining == 0 {
                return if editing_custom {
                    InteractionLineKind::AskCustomEditor { question_index: q }
                } else {
                    InteractionLineKind::AskCustomDetail { question_index: q }
                };
            }
        }
        return InteractionLineKind::AskAnsweredPreview { question_index: q };
    }
    // The footer key-hint row and anything beyond.
    InteractionLineKind::AskFooter
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
                | InteractionLineKind::AskOption { .. }
                | InteractionLineKind::AskCustomRow { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InteractionLineKind, InteractionQuestionLayout, ask_user_body_rows,
        ask_user_question_block_rows, ask_user_question_landing_offset,
        classify_ask_user_line, classify_interaction_line, interaction_plan_body_lines,
        review_decision_region_start, review_decision_rows_count, review_offset_is_custom_label,
        review_selected_option_for_offset,
    };

    fn question(options_len: usize, allow_custom: bool, multiple: bool, answered: bool) -> InteractionQuestionLayout {
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
        assert_eq!(ask_user_question_block_rows(&question(0, false, false, false)), 2);
        assert_eq!(ask_user_question_block_rows(&question(2, true, false, false)), 2 + 2 + 2);
        assert_eq!(ask_user_question_block_rows(&question(2, true, false, true)), 2 + 2 + 2 + 1);
        // Unanswered with no custom slot: 2 + options.
        assert_eq!(ask_user_question_block_rows(&question(3, false, true, false)), 2 + 3);
    }

    #[test]
    fn ask_user_body_rows_and_classifier_cover_the_full_continuous_budget() {
        // Two questions: block(q0) = 2+2+2 = 6 (2 opts + custom, unanswered),
        // block(q1) = 2+2 = 4 (2 opts, no custom). Total = plan + separator +
        // blocks + footer.
        let layouts = [question(2, true, false, false), question(2, false, true, false)];
        let plan = 3;
        let total = ask_user_body_rows(plan, &layouts);
        assert_eq!(total, 3 + 1 + 6 + 4 + 1);

        // Every offset maps to a concrete row kind (never the non-interactive
        // fallback within the budget), with correct question/option indices.
        assert_eq!(
            classify_ask_user_line(&layouts, plan, 0, false),
            InteractionLineKind::AskPlanBody
        );
        assert_eq!(
            classify_ask_user_line(&layouts, plan, plan, false),
            InteractionLineKind::AskSeparator
        );
        // q0 block starts at plan+1.
        let q0 = plan + 1;
        assert_eq!(
            classify_ask_user_line(&layouts, plan, q0, false),
            InteractionLineKind::AskQuestionHeader { question_index: 0 }
        );
        assert_eq!(
            classify_ask_user_line(&layouts, plan, q0 + 1, false),
            InteractionLineKind::AskQuestionText { question_index: 0 }
        );
        assert_eq!(
            classify_ask_user_line(&layouts, plan, q0 + 2, false),
            InteractionLineKind::AskOption { question_index: 0, option_index: 0 }
        );
        assert_eq!(
            classify_ask_user_line(&layouts, plan, q0 + 3, false),
            InteractionLineKind::AskOption { question_index: 0, option_index: 1 }
        );
        assert_eq!(
            classify_ask_user_line(&layouts, plan, q0 + 4, false),
            InteractionLineKind::AskCustomRow { question_index: 0 }
        );
        assert_eq!(
            classify_ask_user_line(&layouts, plan, q0 + 5, true),
            InteractionLineKind::AskCustomEditor { question_index: 0 }
        );
        assert_eq!(
            classify_ask_user_line(&layouts, plan, q0 + 5, false),
            InteractionLineKind::AskCustomDetail { question_index: 0 }
        );
        // q1 block starts right after q0's 6 rows.
        let q1 = q0 + 6;
        assert_eq!(
            classify_ask_user_line(&layouts, plan, q1, false),
            InteractionLineKind::AskQuestionHeader { question_index: 1 }
        );
        assert_eq!(
            classify_ask_user_line(&layouts, plan, q1 + 3, false),
            InteractionLineKind::AskOption { question_index: 1, option_index: 1 }
        );
        // The landing offset is the first option row (start + 2).
        assert_eq!(ask_user_question_landing_offset(plan, &layouts, 0), q0 + 2);
        assert_eq!(ask_user_question_landing_offset(plan, &layouts, 1), q1 + 2);
        // Footer is the last row of the budget.
        assert_eq!(
            classify_ask_user_line(&layouts, plan, total - 1, false),
            InteractionLineKind::AskFooter
        );
    }

    #[test]
    fn ask_user_landing_offset_falls_back_to_the_header_without_options() {
        let layouts = [question(0, false, false, false), question(2, false, false, false)];
        let plan = 0;
        // Question with no options: the landing row is its header.
        assert_eq!(ask_user_question_landing_offset(plan, &layouts, 0), 1);
        assert_eq!(ask_user_question_landing_offset(plan, &layouts, 1), 1 + 2 + 2);
    }

    #[test]
    fn submit_eligibility_is_restricted_to_decision_rows() {
        assert!(InteractionLineKind::ReviewOption { option_index: 0 }.is_submit_eligible());
        assert!(InteractionLineKind::ReviewCustomLabel.is_submit_eligible());
        assert!(InteractionLineKind::ReviewEditor.is_submit_eligible());
        assert!(InteractionLineKind::AskOption { question_index: 0, option_index: 0 }.is_submit_eligible());
        assert!(InteractionLineKind::AskCustomRow { question_index: 0 }.is_submit_eligible());
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
