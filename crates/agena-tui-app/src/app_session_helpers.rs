pub(crate) fn model_name_status_label(model: &ModelRef) -> String {
    agena_tui_session::session_helpers::model_name_status_label(model)
}

pub(crate) fn execution_model_status_label(
    execution: &SessionExecutionContextResource,
) -> Option<String> {
    agena_tui_session::session_helpers::execution_model_status_label(execution)
}

pub(crate) fn execution_model_name_status_label(
    execution: &SessionExecutionContextResource,
) -> Option<String> {
    agena_tui_session::session_helpers::execution_model_name_status_label(execution)
}

pub(crate) fn derive_session_title(i18n: &I18n, text: &str) -> String {
    let fallback = ui_text::t(i18n, "composer-session-new");
    let first_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback.as_str());
    truncate_display_width(first_line, 60)
}

pub(crate) fn draft_title_source(draft: &ComposerDraft) -> Option<String> {
    let mut preview = String::new();
    for node in &draft.document.0 {
        match node {
            agena_domain::ComposerNode::Text { text } => preview.push_str(text),
            agena_domain::ComposerNode::Activity { activity } => preview.push_str(
                &crate::composer_state_impls::composer_activity_presentation(&activity.payload).1,
            ),
        }
    }

    if preview.trim().is_empty() {
        None
    } else {
        Some(preview)
    }
}

pub(crate) fn truncate_display_width(text: &str, max_width: usize) -> String {
    let text = sanitize_terminal_text(text);
    let mut width = 0_usize;
    let mut out = String::new();
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width.saturating_add(ch_width) > max_width {
            break;
        }
        out.push(ch);
        width = width.saturating_add(ch_width);
    }
    if out.is_empty() {
        text.chars().take(max_width).collect()
    } else {
        out
    }
}

pub(crate) fn user_input_answer_values(
    question: &UserInputQuestion,
    draft: &UserInputAnswerDraft,
) -> Vec<String> {
    agena_tui_session::session_helpers::user_input_answer_values(question, draft)
}

pub(crate) fn user_input_question_label(question: &UserInputQuestion) -> &str {
    agena_tui_session::session_helpers::user_input_question_label(question)
}

pub(crate) fn contains_case_insensitive(text: &str, query: &str) -> bool {
    agena_tui_session::session_helpers::contains_case_insensitive(text, query)
}

pub(crate) fn find_search_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    agena_tui_session::session_helpers::find_search_ranges(text, query)
}

pub(crate) fn run_status_line_command(
    command: String,
    session_id: Option<String>,
    focus: String,
) -> Option<String> {
    let mut cmd = if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/d", "/s", "/c", command.as_str()]);
        cmd
    } else {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-lc", command.as_str()]);
        cmd
    };
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    cmd.env("AGENA_TUI_FOCUS", focus);
    if let Some(session_id) = session_id {
        cmd.env("AGENA_SESSION_ID", session_id);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next().unwrap_or_default().trim();
    (!line.is_empty()).then(|| line.to_string())
}

pub(crate) fn next_grapheme_boundary(text: &str, index: usize) -> usize {
    agena_tui_session::session_helpers::next_grapheme_boundary(text, index)
}

pub(crate) fn attachment_chip_label(
    i18n: &I18n,
    path: &Path,
    kind: AttachmentKind,
    is_directory: bool,
    width: Option<u32>,
    height: Option<u32>,
    size_bytes: u64,
) -> String {
    ui_text::attachment_chip_label(i18n, path, kind, is_directory, width, height, size_bytes)
}

pub(crate) fn cleanup_temporary_composer_items(items: &[ComposerItem]) {
    let _ = items;
}

pub(crate) fn cleanup_temporary_composer_item(item: &ComposerItem) {
    let _ = item;
}

pub(crate) fn attachment_placeholder_base(
    i18n: &I18n,
    path: &Path,
    kind: AttachmentKind,
    is_directory: bool,
) -> String {
    ui_text::attachment_placeholder_base(i18n, path, kind, is_directory)
}

pub(crate) fn find_placeholder_occurrence(
    text: &str,
    placeholder: &str,
    occupied: &[Range<usize>],
) -> Option<Range<usize>> {
    if placeholder.is_empty() {
        return None;
    }

    let mut search_start = 0;
    while search_start < text.len() {
        let relative = text.get(search_start..)?.find(placeholder)?;
        let start = search_start + relative;
        let end = start + placeholder.len();
        let candidate = start..end;
        if occupied
            .iter()
            .all(|range| range.end <= candidate.start || range.start >= candidate.end)
        {
            return Some(candidate);
        }
        search_start = next_grapheme_boundary(text, start);
    }
    None
}
use crate::{
    AttachmentKind, Command, ComposerDraft, ComposerItem, I18n, ModelRef, Path, Range,
    SessionExecutionContextResource, Stdio, UnicodeWidthChar, UserInputQuestion,
    sanitize_terminal_text, ui_text,
};
use agena_tui::user_input::UserInputAnswerDraft;

#[cfg(test)]
mod tests {
    use super::{ModelRef, model_name_status_label};

    #[test]
    fn compact_model_status_hides_provider_and_adapter() {
        let model = ModelRef::new_with_adapter("provider-a", "adapter-b", "model-c");

        assert_eq!(model_name_status_label(&model), "model-c");
    }
}
