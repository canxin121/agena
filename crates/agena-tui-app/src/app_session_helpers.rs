pub(crate) fn model_name_status_label(model: &ModelRef) -> String {
    model.model_id.to_string()
}

pub(crate) fn execution_model_status_label(
    execution: &SessionExecutionContextResource,
) -> Option<String> {
    let provider_id = execution
        .model_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let adapter_id = execution
        .model_adapter_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let model_id = execution
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if provider_id.is_none() && model_id.is_none() {
        return None;
    }

    Some(match adapter_id {
        Some(adapter_id) => format!(
            "{}/{}/{}",
            provider_id.unwrap_or("auto"),
            adapter_id,
            model_id.unwrap_or("default")
        ),
        None => format!(
            "{}/{}",
            provider_id.unwrap_or("auto"),
            model_id.unwrap_or("default")
        ),
    })
}

pub(crate) fn execution_model_name_status_label(
    execution: &SessionExecutionContextResource,
) -> Option<String> {
    execution
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(crate) fn is_rewind_target_message(message: &MessageResource) -> bool {
    message.role == MessageRole::User && message.state == MessageStatus::Completed
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
    let mut labels = draft
        .items
        .iter()
        .map(|item| {
            (
                item.placeholder().to_string(),
                item.short_label().to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut elements = draft.elements.clone();
    elements.sort_by_key(|element| element.range.start);

    let mut preview = String::new();
    let mut cursor = 0;
    for element in elements {
        let start = min(element.range.start, draft.text.len());
        let end = min(element.range.end, draft.text.len());
        if cursor < start {
            preview.push_str(&draft.text[cursor..start]);
        }
        if let Some(label) = labels.remove(element.placeholder.as_str()) {
            preview.push_str(label.as_str());
        }
        cursor = end;
    }
    if cursor < draft.text.len() {
        preview.push_str(&draft.text[cursor..]);
    }

    if preview.trim().is_empty() {
        draft
            .items
            .first()
            .map(ComposerItem::short_label)
            .map(str::to_owned)
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
    let mut values = draft
        .option_indexes
        .iter()
        .filter_map(|index| question.options.get(*index))
        .map(|option| option.label.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.extend(
        draft
            .custom_values
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    );
    if question.multiple {
        values
    } else {
        values.into_iter().take(1).collect()
    }
}

pub(crate) fn user_input_question_label(question: &UserInputQuestion) -> &str {
    let header = question.header.trim();
    if !header.is_empty() {
        header
    } else if !question.question.trim().is_empty() {
        question.question.trim()
    } else {
        question.id.as_str()
    }
}

pub(crate) fn contains_case_insensitive(text: &str, query: &str) -> bool {
    let trimmed = query.trim();
    !trimmed.is_empty()
        && text
            .to_lowercase()
            .contains(trimmed.to_lowercase().as_str())
}

pub(crate) fn find_search_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    let mut search_start = 0;
    while search_start < text.len() {
        let Some((start, end)) = find_query_match_from(text, query, search_start) else {
            break;
        };
        ranges.push(start..end);
        search_start = if end > start {
            end
        } else {
            next_grapheme_boundary(text, start)
        };
    }
    ranges
}

pub(crate) fn find_query_match_from(
    text: &str,
    query: &str,
    start_at: usize,
) -> Option<(usize, usize)> {
    if query.is_ascii() {
        let mut starts = text[start_at..]
            .char_indices()
            .map(|(offset, _)| start_at + offset)
            .collect::<Vec<_>>();
        if !starts.contains(&text.len()) {
            starts.push(text.len());
        }
        for start in starts {
            let end = start.saturating_add(query.len());
            if let Some(slice) = text.get(start..end)
                && slice.eq_ignore_ascii_case(query)
            {
                return Some((start, end));
            }
        }
        None
    } else {
        text[start_at..]
            .find(query)
            .map(|offset| start_at + offset)
            .map(|start| (start, start + query.len()))
    }
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
    if index >= text.len() {
        return text.len();
    }
    let grapheme = text[index..].graphemes(true).next().unwrap_or_default();
    index + grapheme.len()
}

pub(crate) fn attachment_chip_label(
    i18n: &I18n,
    path: &Path,
    kind: AttachmentKind,
    width: Option<u32>,
    height: Option<u32>,
    size_bytes: u64,
) -> String {
    ui_text::attachment_chip_label(i18n, path, kind, width, height, size_bytes)
}

pub(crate) fn cleanup_temporary_composer_items(items: &[ComposerItem]) {
    for item in items {
        cleanup_temporary_composer_item(item);
    }
}

pub(crate) fn cleanup_temporary_composer_item(item: &ComposerItem) {
    if let ComposerItem::Attachment(attachment) = item
        && attachment.is_temp
    {
        let _ = std::fs::remove_file(&attachment.path);
        if let Some(root) = attachment.cleanup_root.as_ref() {
            let _ = std::fs::remove_dir(root);
        }
    }
}

pub(crate) fn push_submission_text(parts: &mut Vec<MessagePartContent>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(MessagePartContent::Text(last)) = parts.last_mut() {
        last.text.push_str(text);
        return;
    }
    parts.push(MessagePartContent::Text(MessageTextPart {
        text: text.to_owned(),
        synthetic: false,
        ignored: false,
    }));
}

pub(crate) fn attachment_placeholder_base(
    i18n: &I18n,
    path: &Path,
    kind: AttachmentKind,
) -> String {
    ui_text::attachment_placeholder_base(i18n, path, kind)
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
    AttachmentKind, BTreeMap, Command, ComposerDraft, ComposerItem, I18n, MessageResource,
    MessageRole, MessageStatus, ModelRef, Path, Range, SessionExecutionContextResource, Stdio,
    UnicodeWidthChar, UserInputQuestion, min, sanitize_terminal_text, ui_text,
};
use agena_api::resource::{MessagePartContent, MessageTextPart};
use agena_tui::user_input::UserInputAnswerDraft;
use unicode_segmentation::UnicodeSegmentation;

#[cfg(test)]
mod tests {
    use super::{ModelRef, model_name_status_label};

    #[test]
    fn compact_model_status_hides_provider_and_adapter() {
        let model = ModelRef::new_with_adapter("provider-a", "adapter-b", "model-c");

        assert_eq!(model_name_status_label(&model), "model-c");
    }
}
