//! Runtime-neutral session projections and text-search helpers.
//!
//! The application owns persistence, routing, and terminal effects. These
//! helpers only transform API/domain values or search text, so they belong to
//! the session presentation owner.

use std::ops::Range;

use agena_api::resource::{
    MessageResource, MessageRole, MessageStatus, SessionExecutionContextResource,
};
use agena_domain::{ModelRef, UserInputQuestion};
use agena_tui::user_input::UserInputAnswerDraft;
use unicode_segmentation::UnicodeSegmentation;

pub fn model_name_status_label(model: &ModelRef) -> String {
    model.model_id.to_string()
}

pub fn execution_model_status_label(execution: &SessionExecutionContextResource) -> Option<String> {
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

pub fn execution_model_name_status_label(
    execution: &SessionExecutionContextResource,
) -> Option<String> {
    execution
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn is_rewind_target_message(message: &MessageResource) -> bool {
    message.role == MessageRole::User && message.state == MessageStatus::Completed
}

pub fn user_input_answer_values(
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

pub fn user_input_question_label(question: &UserInputQuestion) -> &str {
    let header = question.header.trim();
    if !header.is_empty() {
        header
    } else if !question.question.trim().is_empty() {
        question.question.trim()
    } else {
        question.id.as_str()
    }
}

pub fn contains_case_insensitive(text: &str, query: &str) -> bool {
    let trimmed = query.trim();
    !trimmed.is_empty()
        && text
            .to_lowercase()
            .contains(trimmed.to_lowercase().as_str())
}

pub fn find_search_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
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

pub fn find_query_match_from(text: &str, query: &str, start_at: usize) -> Option<(usize, usize)> {
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

pub fn next_grapheme_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let grapheme = text[index..].graphemes(true).next().unwrap_or_default();
    index + grapheme.len()
}

#[cfg(test)]
mod tests {
    use super::{contains_case_insensitive, execution_model_status_label, find_search_ranges};
    use agena_api::resource::SessionExecutionContextResource;

    #[test]
    fn execution_status_uses_provider_adapter_and_model() {
        let execution = SessionExecutionContextResource {
            agent_id: "agena".to_owned(),
            execution_access: agena_api::resource::ExecutionAccess::Inherit,
            selected_permission: Default::default(),
            effective_permission: Default::default(),
            permission_ceiling: Default::default(),
            model_provider_id: Some(" provider ".into()),
            model_adapter_id: Some("adapter".into()),
            model_id: Some("model".into()),
            model_thinking_mode: None,
            model_speed_mode: None,
            model_verbosity: None,
            model_parallel_tool_calls: None,
            effective_workspace_root: None,
            task_id: None,
            subtask_status: None,
            subtask_started_at: None,
            subtask_finished_at: None,
            subtask_failure: None,
        };
        assert_eq!(
            execution_model_status_label(&execution).as_deref(),
            Some("provider/adapter/model")
        );
    }

    #[test]
    fn search_helpers_preserve_unicode_boundaries() {
        assert!(contains_case_insensitive("Hello World", "world"));
        assert_eq!(find_search_ranges("aB ab", "ab"), vec![0..2, 3..5]);
    }
}
