//! Live interaction content for pending user-input parts rendered inline in
//! the transcript.
//!
//! A pending interaction part (plan review or ask-user) renders as an
//! expandable Activity. When expanded and still awaiting a decision, the
//! transcript renders the plan body and the decision rows natively
//! ("everything is a part"): the plan body flows through the same Markdown
//! pipeline as every other part with the standard activity indent.
//!
//! The two flows drive their interaction differently. Plan review keeps the
//! transcript cursor IS the review cursor — which decision row the cursor sits
//! on is the selected option. Ask-user is a **paged wizard**: the current
//! question page, the option cursor and the answers live in the presentation
//! ([`agena_tui::user_input::UserInputPresentation`]), and the transcript
//! cursor only marks which part is active. The renderer draws exactly one page
//! at a time (the plan body on the first page only), ending in a summary page
//! where Enter submits.
//!
//! This module owns the small projections the renderer and the app share so
//! they can never drift: the live selection snapshot ([`PendingInteractionView`])
//! handed from the App to the renderer, and the single-source layout helpers
//! ([`interaction_plan_body_lines`], [`classify_interaction_line`],
//! [`interaction_ask_user_page_body_rows`], …) that both the renderer and the
//! App's key routing derive from.

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
        _ => None,
    }
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
    /// Ask-user (paged wizard): the current question page — `Some(index)` is a
    /// question page, `None` is the final summary page. Projected from the
    /// presentation, NOT the transcript cursor.
    pub wizard_page: Option<usize>,
    /// Ask-user (paged wizard): the option cursor within the current question
    /// page (presentation-owned, projected for the renderer's highlight).
    pub wizard_option: usize,
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

/// Semantic kind of a review body line in an expanded pending interaction
/// part. Ask-user has no row kinds: its wizard is presentation-driven (page +
/// option cursor), so only the review decision rows need classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionLineKind {
    PlanBody,
    Separator,
    ReviewOption { option_index: usize },
    ReviewOptionDetail { option_index: usize },
    ReviewCustomLabel,
    ReviewCustomDetail,
    ReviewEditor,
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

/// Maps an offset within a review decision block (0 = first option label row)
/// to the selected option index: label rows at even offsets (index = offset/2);
/// the custom label maps to `options_len` when `allow_custom`.
pub fn review_selected_option_for_offset(
    options_len: usize,
    allow_custom: bool,
    decision_offset: usize,
) -> Option<usize> {
    if allow_custom && decision_offset == options_len * 2 {
        return Some(options_len);
    }
    if decision_offset % 2 == 0 {
        let index = decision_offset / 2;
        (index < options_len).then_some(index)
    } else {
        None
    }
}

/// Whether a decision-block offset is on the custom feedback label row.
pub fn review_offset_is_custom_label(
    options_len: usize,
    allow_custom: bool,
    decision_offset: usize,
) -> bool {
    allow_custom && decision_offset == options_len * 2
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

/// Body rows one ask-user question block occupies on its page: header row,
/// question-text row, 2 rows per option (label + detail), 2 per custom slot,
/// and an answered-preview row. The renderer draws with exactly this budget
/// and the reconciliation test asserts it, so the layout contract can never
/// drift (the single source the old AskUser row classifier lacked).
pub fn interaction_ask_user_question_block_rows(layout: &InteractionQuestionLayout) -> usize {
    2 + layout.options_len * 2
        + usize::from(layout.allow_custom) * 2
        + usize::from(layout.answered)
}

/// Whether the ask-user wizard's `page_index` renders the plan body above the
/// question block: only the first page does.
pub fn interaction_ask_user_page_has_plan(page_index: usize) -> bool {
    page_index == 0
}

/// Body rows of one ask-user question page: (plan body + separator on page 0
/// only) + the question block + the footer page-indicator row.
pub fn interaction_ask_user_page_body_rows(
    plan_body_lines: usize,
    page_index: usize,
    layout: &InteractionQuestionLayout,
) -> usize {
    let plan = if interaction_ask_user_page_has_plan(page_index) {
        plan_body_lines + 1
    } else {
        0
    };
    plan + interaction_ask_user_question_block_rows(layout) + 1
}

/// Body rows of the final ask-user summary page: title row + one row per
/// question + separator + submit row.
pub fn interaction_ask_user_summary_body_rows(question_count: usize) -> usize {
    1 + question_count + 1 + 1
}

/// Full review classifier: given the per-question layout, plan row count and
/// the body offset, which semantic row the cursor is on. Ask-user has no row
/// kinds (its wizard is presentation-driven), so this classifies review
/// decision rows only. Both the app (key routing) and the renderer's
/// reconciliation tests use this.
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
    if question.allow_custom {
        if decision_offset == question.options_len * 2 {
            return InteractionLineKind::ReviewCustomLabel;
        }
        if decision_offset == question.options_len * 2 + 1 {
            return InteractionLineKind::ReviewCustomDetail;
        }
        if decision_offset == question.options_len * 2 + 2 && editing_custom {
            return InteractionLineKind::ReviewEditor;
        }
    }
    let option_index = decision_offset / 2;
    if option_index >= question.options_len {
        return InteractionLineKind::PlanBody;
    }
    if decision_offset % 2 == 0 {
        InteractionLineKind::ReviewOption { option_index }
    } else {
        InteractionLineKind::ReviewOptionDetail { option_index }
    }
}

impl InteractionLineKind {
    /// Whether Enter on this line is a review decision (an option row, the
    /// custom feedback label or its editor) rather than a plain node toggle.
    pub fn is_submit_eligible(self) -> bool {
        matches!(
            self,
            InteractionLineKind::ReviewOption { .. }
                | InteractionLineKind::ReviewCustomLabel
                | InteractionLineKind::ReviewEditor
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InteractionLineKind, InteractionQuestionLayout, classify_interaction_line,
        interaction_ask_user_page_body_rows, interaction_ask_user_page_has_plan,
        interaction_ask_user_question_block_rows, interaction_ask_user_summary_body_rows,
        interaction_plan_body_lines,
        review_decision_region_start, review_offset_is_custom_label,
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
    fn review_selected_option_maps_even_offsets_and_the_custom_slot() {
        // 2 options + custom: label rows at 0, 2 (options), 4 (custom).
        assert_eq!(review_selected_option_for_offset(2, true, 0), Some(0));
        assert_eq!(review_selected_option_for_offset(2, true, 2), Some(1));
        assert_eq!(review_selected_option_for_offset(2, true, 4), Some(2));
        // Detail rows (odd offsets) are not selections.
        assert_eq!(review_selected_option_for_offset(2, true, 1), None);
        // Without custom, offset 4 is out of range.
        assert_eq!(review_selected_option_for_offset(2, false, 4), None);
        assert_eq!(review_selected_option_for_offset(2, false, 3), None);
        assert!(review_offset_is_custom_label(2, true, 4));
        assert!(!review_offset_is_custom_label(2, true, 3));
        assert!(!review_offset_is_custom_label(2, false, 4));
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
        // Option 0: label at offset plan+1, detail at plan+2.
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 1, false),
            InteractionLineKind::ReviewOption { option_index: 0 }
        );
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 2, false),
            InteractionLineKind::ReviewOptionDetail { option_index: 0 }
        );
        // Option 1: label at plan+3, detail at plan+4.
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 3, false),
            InteractionLineKind::ReviewOption { option_index: 1 }
        );
        // Custom label at plan+5, detail at plan+6, editor at plan+7 when open.
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 5, false),
            InteractionLineKind::ReviewCustomLabel
        );
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 6, false),
            InteractionLineKind::ReviewCustomDetail
        );
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 7, false),
            InteractionLineKind::PlanBody
        );
        assert_eq!(
            classify_interaction_line(&layouts, plan, plan + 7, true),
            InteractionLineKind::ReviewEditor
        );
    }

    #[test]
    fn ask_user_block_rows_covers_the_full_block_budget() {
        // One question: header, text, 2 options x2 rows, 1 custom x2 rows,
        // answered preview.
        assert_eq!(interaction_ask_user_question_block_rows(&question(0, false, false, false)), 2);
        assert_eq!(interaction_ask_user_question_block_rows(&question(2, true, false, false)), 2 + 4 + 2);
        assert_eq!(interaction_ask_user_question_block_rows(&question(2, true, false, true)), 2 + 4 + 2 + 1);
        // Unanswered with no custom slot: 2 + 2*options.
        assert_eq!(interaction_ask_user_question_block_rows(&question(3, false, true, false)), 2 + 6);
    }

    #[test]
    fn ask_user_page_rows_include_plan_on_page_zero_and_the_footer() {
        let layout = question(2, true, false, false); // block = 2 + 4 + 2 = 8
        // Page 0: plan + separator + block + footer.
        assert_eq!(
            interaction_ask_user_page_body_rows(3, 0, &layout),
            3 + 1 + 8 + 1
        );
        // Later pages: no plan/separator, just block + footer.
        assert_eq!(interaction_ask_user_page_body_rows(3, 1, &layout), 8 + 1);
        assert_eq!(interaction_ask_user_page_body_rows(3, 2, &layout), 8 + 1);
        assert!(interaction_ask_user_page_has_plan(0));
        assert!(!interaction_ask_user_page_has_plan(1));
    }

    #[test]
    fn ask_user_summary_rows_are_title_plus_one_row_per_question() {
        assert_eq!(interaction_ask_user_summary_body_rows(2), 1 + 2 + 1 + 1);
        assert_eq!(interaction_ask_user_summary_body_rows(1), 4);
        assert_eq!(interaction_ask_user_summary_body_rows(0), 3);
    }

    #[test]
    fn submit_eligibility_is_restricted_to_review_decision_rows() {
        assert!(InteractionLineKind::ReviewOption { option_index: 0 }.is_submit_eligible());
        assert!(InteractionLineKind::ReviewCustomLabel.is_submit_eligible());
        assert!(InteractionLineKind::ReviewEditor.is_submit_eligible());
        assert!(!InteractionLineKind::PlanBody.is_submit_eligible());
        assert!(!InteractionLineKind::Separator.is_submit_eligible());
        assert!(!InteractionLineKind::ReviewOptionDetail { option_index: 0 }.is_submit_eligible());
        assert!(!InteractionLineKind::ReviewCustomDetail.is_submit_eligible());
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
