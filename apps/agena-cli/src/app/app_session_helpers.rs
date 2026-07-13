pub(in crate::app) fn build_visible_session_items(
    items: &[SessionResource],
    mode: SessionViewMode,
    query: &str,
) -> Vec<SessionResource> {
    let trimmed_query = query.trim();
    match mode {
        SessionViewMode::Roots => {
            let mut roots = items
                .iter()
                .filter(|session| session.parent_id.is_none())
                .cloned()
                .collect::<Vec<_>>();
            roots.sort_by(session_sort_recent);
            if trimmed_query.is_empty() {
                roots
            } else {
                roots
                    .into_iter()
                    .filter(|session| session_matches_query(session, trimmed_query))
                    .collect()
            }
        }
        SessionViewMode::All | SessionViewMode::Subtree => {
            let by_id = items
                .iter()
                .cloned()
                .map(|session| (session.id, session))
                .collect::<BTreeMap<_, _>>();
            let mut children = BTreeMap::<Option<i64>, Vec<i64>>::new();
            for session in items {
                let parent_id = session
                    .parent_id
                    .filter(|parent_id| by_id.contains_key(parent_id));
                children.entry(parent_id).or_default().push(session.id);
            }
            for child_ids in children.values_mut() {
                child_ids.sort_by(|left, right| session_sort_recent(&by_id[left], &by_id[right]));
            }

            let kept_ids = if trimmed_query.is_empty() {
                by_id.keys().copied().collect::<HashSet<_>>()
            } else {
                let mut kept = HashSet::new();
                for session in items
                    .iter()
                    .filter(|session| session_matches_query(session, trimmed_query))
                {
                    let mut current = Some(session.id);
                    while let Some(id) = current {
                        if !kept.insert(id) {
                            break;
                        }
                        current = by_id.get(&id).and_then(|item| item.parent_id);
                    }
                }
                kept
            };

            let root_ids = children.get(&None).cloned().unwrap_or_default();
            let mut out = Vec::new();
            for root_id in root_ids {
                append_session_subtree(root_id, &children, &by_id, &kept_ids, &mut out);
            }
            out
        }
    }
}

pub(in crate::app) fn append_session_subtree(
    session_id: i64,
    children: &BTreeMap<Option<i64>, Vec<i64>>,
    by_id: &BTreeMap<i64, SessionResource>,
    kept_ids: &HashSet<i64>,
    out: &mut Vec<SessionResource>,
) {
    if !kept_ids.contains(&session_id) {
        return;
    }
    if let Some(session) = by_id.get(&session_id) {
        out.push(session.clone());
    }
    if let Some(child_ids) = children.get(&Some(session_id)) {
        for child_id in child_ids {
            append_session_subtree(*child_id, children, by_id, kept_ids, out);
        }
    }
}

pub(in crate::app) fn lineage_relation_tag_key(relation: LineageRelation) -> &'static str {
    match relation {
        LineageRelation::Ancestor => "session-tag-ancestor",
        LineageRelation::Current => "session-tag-current",
        LineageRelation::Sibling => "session-tag-sibling",
        LineageRelation::Child => "session-tag-child",
    }
}

pub(in crate::app) fn build_lineage_session_items(
    items: &[SessionResource],
    current_session_id: i64,
) -> Vec<LineageSessionItem> {
    let by_id = items
        .iter()
        .cloned()
        .map(|session| (session.id, session))
        .collect::<BTreeMap<_, _>>();
    if !by_id.contains_key(&current_session_id) {
        return Vec::new();
    }

    let lineage_chain = session_lineage_chain(current_session_id, &by_id);
    let lineage_ids = lineage_chain.iter().copied().collect::<HashSet<_>>();

    let mut children = BTreeMap::<Option<i64>, Vec<i64>>::new();
    for session in items {
        let parent_id = session
            .parent_id
            .filter(|parent_id| by_id.contains_key(parent_id));
        children.entry(parent_id).or_default().push(session.id);
    }
    for child_ids in children.values_mut() {
        child_ids.sort_by(|left, right| {
            let left_on_path = lineage_ids.contains(left);
            let right_on_path = lineage_ids.contains(right);
            right_on_path
                .cmp(&left_on_path)
                .then_with(|| session_sort_recent(&by_id[left], &by_id[right]))
        });
    }

    let Some(root_id) = lineage_chain.first().copied() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut visited = HashSet::new();
    append_lineage_items(
        root_id,
        0,
        false,
        current_session_id,
        &lineage_ids,
        &children,
        &by_id,
        &mut visited,
        &mut out,
    );
    out
}

pub(in crate::app) fn summarize_lineage_session_items(
    items: &[LineageSessionItem],
) -> Option<SessionLineageSummary> {
    let root_id = items.first()?.session.id;
    let current = items
        .iter()
        .find(|item| item.relation == LineageRelation::Current)?;
    Some(SessionLineageSummary {
        root_id,
        depth: current.depth,
        side_branch_count: items
            .iter()
            .filter(|item| item.relation == LineageRelation::Sibling)
            .count(),
        descendant_count: items
            .iter()
            .filter(|item| item.relation == LineageRelation::Child)
            .count(),
    })
}

pub(in crate::app) fn model_name_status_label(model: &ModelRef) -> String {
    model.model_id.to_string()
}

pub(in crate::app) fn execution_model_status_label(
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

pub(in crate::app) fn execution_model_name_status_label(
    execution: &SessionExecutionContextResource,
) -> Option<String> {
    execution
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(in crate::app) fn session_summary_status_parts(
    model_part: Option<String>,
    agent: Option<String>,
    token_usage: Option<TokenUsageStatus>,
) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(model_part) = model_part
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        parts.push(model_part);
    }
    if let Some(agent) = agent
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        parts.push(agent);
    }
    if let Some(token_usage) = token_usage {
        parts.push(token_usage.label());
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::{ModelRef, model_name_status_label};

    #[test]
    fn compact_model_status_hides_provider_and_adapter() {
        let model = ModelRef::new_with_adapter("provider-a", "adapter-b", "model-c");

        assert_eq!(model_name_status_label(&model), "model-c");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum TokenUsageStatus {
    PercentUsed(u64),
    UsedTokens(u64),
}

impl TokenUsageStatus {
    pub(in crate::app) fn label(self) -> String {
        match self {
            Self::PercentUsed(percent_used) => format_token_progress_label(percent_used),
            Self::UsedTokens(tokens) => format!("{} used", format_tokens_k(tokens)),
        }
    }
}

pub(in crate::app) fn status_line_token_usage(
    usage: &SessionUsageResource,
) -> Option<TokenUsageStatus> {
    let current_tokens = usage.projected_tokens.unwrap_or(usage.current_tokens);
    if let Some(context_window_tokens) = usage.model_context_window_tokens {
        return Some(TokenUsageStatus::PercentUsed(
            agena::session::context_usage_percent_used(current_tokens, context_window_tokens),
        ));
    }

    Some(TokenUsageStatus::UsedTokens(current_tokens))
}

pub(in crate::app) fn format_token_progress_label(percent_used: u64) -> String {
    format!("{}%", percent_used.min(100))
}

pub(in crate::app) fn format_tokens_k(tokens: u64) -> String {
    if tokens == 0 {
        return "0k".to_string();
    }

    let value = tokens as f64 / 1_000.0;
    if value < 10.0 {
        return format!("{value:.1}k");
    }
    format!("{value:.0}k")
}

pub(in crate::app) fn session_lineage_chain(
    current_session_id: i64,
    by_id: &BTreeMap<i64, SessionResource>,
) -> Vec<i64> {
    let mut chain = Vec::new();
    let mut current = Some(current_session_id);
    let mut seen = HashSet::new();

    while let Some(session_id) = current {
        if !seen.insert(session_id) {
            break;
        }
        let Some(session) = by_id.get(&session_id) else {
            break;
        };
        chain.push(session_id);
        current = session
            .parent_id
            .filter(|parent_id| by_id.contains_key(parent_id));
    }

    chain.reverse();
    chain
}

pub(in crate::app) fn is_rewind_target_message(message: &MessageResource) -> bool {
    message.role == MessageRole::User && message.state == MessageStatus::Completed
}

#[allow(clippy::too_many_arguments)]
pub(in crate::app) fn append_lineage_items(
    session_id: i64,
    depth: usize,
    under_current_branch: bool,
    current_session_id: i64,
    lineage_ids: &HashSet<i64>,
    children: &BTreeMap<Option<i64>, Vec<i64>>,
    by_id: &BTreeMap<i64, SessionResource>,
    visited: &mut HashSet<i64>,
    out: &mut Vec<LineageSessionItem>,
) {
    if !visited.insert(session_id) {
        return;
    }
    let Some(session) = by_id.get(&session_id).cloned() else {
        return;
    };

    let child_ids = children.get(&Some(session_id)).cloned().unwrap_or_default();
    let relation = if session_id == current_session_id {
        LineageRelation::Current
    } else if lineage_ids.contains(&session_id) {
        LineageRelation::Ancestor
    } else if under_current_branch {
        LineageRelation::Child
    } else {
        LineageRelation::Sibling
    };

    out.push(LineageSessionItem {
        session,
        relation,
        depth,
        is_leaf: child_ids.is_empty(),
    });

    let next_under_current_branch = under_current_branch || session_id == current_session_id;
    for child_id in child_ids {
        append_lineage_items(
            child_id,
            depth.saturating_add(1),
            next_under_current_branch,
            current_session_id,
            lineage_ids,
            children,
            by_id,
            visited,
            out,
        );
    }
}

pub(in crate::app) fn session_matches_query(session: &SessionResource, query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    session.title.to_ascii_lowercase().contains(query.as_str())
        || session.id.to_string().contains(query.as_str())
}

pub(in crate::app) fn session_sort_recent(
    left: &SessionResource,
    right: &SessionResource,
) -> std::cmp::Ordering {
    right
        .updated_at
        .cmp(&left.updated_at)
        .then_with(|| right.id.cmp(&left.id))
}

pub(in crate::app) fn derive_session_title(i18n: &I18n, text: &str) -> String {
    let fallback = ui_text::t(i18n, "composer-session-new");
    let first_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback.as_str());
    truncate_display_width(first_line, 60)
}

pub(in crate::app) fn draft_title_source(draft: &ComposerDraft) -> Option<String> {
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

pub(in crate::app) fn truncate_display_width(text: &str, max_width: usize) -> String {
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

pub(in crate::app) fn user_input_answer_values(
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

pub(in crate::app) fn user_input_question_label(question: &UserInputQuestion) -> &str {
    let header = question.header.trim();
    if !header.is_empty() {
        header
    } else if !question.question.trim().is_empty() {
        question.question.trim()
    } else {
        question.id.as_str()
    }
}

pub(in crate::app) fn contains_case_insensitive(text: &str, query: &str) -> bool {
    let trimmed = query.trim();
    !trimmed.is_empty()
        && text
            .to_lowercase()
            .contains(trimmed.to_lowercase().as_str())
}

pub(in crate::app) fn find_search_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
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

pub(in crate::app) fn find_query_match_from(
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

pub(in crate::app) fn run_status_line_command(
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

pub(in crate::app) fn next_grapheme_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let grapheme = text[index..].graphemes(true).next().unwrap_or_default();
    index + grapheme.len()
}

pub(in crate::app) fn attachment_chip_label(
    i18n: &I18n,
    path: &Path,
    kind: AttachmentKind,
    width: Option<u32>,
    height: Option<u32>,
    size_bytes: u64,
) -> String {
    ui_text::attachment_chip_label(i18n, path, kind, width, height, size_bytes)
}

pub(in crate::app) fn cleanup_temporary_composer_items(items: &[ComposerItem]) {
    for item in items {
        cleanup_temporary_composer_item(item);
    }
}

pub(in crate::app) fn cleanup_temporary_composer_item(item: &ComposerItem) {
    if let ComposerItem::Attachment(attachment) = item
        && attachment.is_temp
    {
        let _ = std::fs::remove_file(&attachment.path);
        if let Some(root) = attachment.cleanup_root.as_ref() {
            let _ = std::fs::remove_dir(root);
        }
    }
}

pub(in crate::app) fn push_submission_text(parts: &mut Vec<PartContent>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = parts.last_mut()
        && last.append_text_delta(text)
    {
        return;
    }
    parts.push(PartContent::text(text.to_string()));
}

pub(in crate::app) fn attachment_placeholder_base(
    i18n: &I18n,
    path: &Path,
    kind: AttachmentKind,
) -> String {
    ui_text::attachment_placeholder_base(i18n, path, kind)
}

pub(in crate::app) fn find_placeholder_occurrence(
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
use crate::app::{
    AttachmentKind, BTreeMap, Command, ComposerDraft, ComposerItem, HashSet, I18n, LineageRelation,
    LineageSessionItem, MessageResource, MessageRole, MessageStatus, ModelRef, PartContent, Path,
    Range, SessionExecutionContextResource, SessionLineageSummary, SessionResource,
    SessionUsageResource, SessionViewMode, Stdio, UnicodeWidthChar, UserInputAnswerDraft,
    UserInputQuestion, min, sanitize_terminal_text, ui_text,
};
use unicode_segmentation::UnicodeSegmentation;
