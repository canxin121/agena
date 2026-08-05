use super::super::{
    I18n, Modifier, RenderedLine, Style, first_non_empty_preview_line, push_label_value,
    push_multiline, push_section_heading, tool_execution_preview, transcript_part_content,
};
use crate::ui_text;
use crate::{
    MessageRequestPartResource, TranscriptActivityContent, TranscriptAssistantReplyLifecycle,
    TranscriptEntryPart, TranscriptPartContent,
};

pub(crate) fn render_file_changes(
    changes: &[agena_api::message_part::FileChangeRecordResource],
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    if changes.is_empty() {
        return;
    }
    push_section_heading(
        out,
        &format!("    {}", ui_text::t(i18n, "message-file-changes")),
        Style::default()
            .fg(agena_tui_components::theme::special_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    for entry in changes {
        push_label_value(
            out,
            "      - ",
            &file_change_resource_list_item_text(entry, i18n),
            file_change_resource_style(entry.kind),
            width,
        );
    }
}

pub(crate) fn render_checklist(
    items: &[agena_api::message_part::TodoItemResource],
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    if items.is_empty() {
        return;
    }
    push_section_heading(
        out,
        &format!("    {}", ui_text::t(i18n, "message-todo-list")),
        Style::default()
            .fg(agena_tui_components::theme::accent_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    for item in items {
        push_label_value(
            out,
            "      - ",
            &format!(
                "[{}|{}] {}",
                todo_status_resource_label(i18n, item.status),
                todo_priority_resource_label(i18n, item.priority),
                item.content
            ),
            Style::default(),
            width,
        );
    }
}

fn file_change_resource_list_item_text(
    change: &agena_api::message_part::FileChangeRecordResource,
    i18n: &I18n,
) -> String {
    use agena_api::message_part::FileChangeKindResource;
    let marker = match change.kind {
        FileChangeKindResource::Added => "A",
        FileChangeKindResource::Updated => "M",
        FileChangeKindResource::Deleted => "D",
        FileChangeKindResource::Moved => "R",
    };
    let label = match change.kind {
        FileChangeKindResource::Added => ui_text::t(i18n, "file-change-added"),
        FileChangeKindResource::Updated => ui_text::t(i18n, "file-change-updated"),
        FileChangeKindResource::Deleted => ui_text::t(i18n, "file-change-deleted"),
        FileChangeKindResource::Moved => ui_text::t(i18n, "file-change-moved"),
    };
    let path = change
        .from_path
        .as_ref()
        .map(|from| format!("{from} → {}", change.path))
        .unwrap_or_else(|| change.path.clone());
    format!("{marker} {path} ({label})")
}

fn file_change_resource_style(kind: agena_api::message_part::FileChangeKindResource) -> Style {
    match kind {
        agena_api::message_part::FileChangeKindResource::Added => {
            Style::default().fg(agena_tui_components::theme::success_color())
        }
        agena_api::message_part::FileChangeKindResource::Updated => {
            Style::default().fg(agena_tui_components::theme::warning_color())
        }
        agena_api::message_part::FileChangeKindResource::Deleted => {
            Style::default().fg(agena_tui_components::theme::danger_color())
        }
        agena_api::message_part::FileChangeKindResource::Moved => {
            Style::default().fg(agena_tui_components::theme::accent_color())
        }
    }
}

fn todo_status_resource_label(
    i18n: &I18n,
    status: agena_api::message_part::TodoStatusResource,
) -> String {
    let key = match status {
        agena_api::message_part::TodoStatusResource::Pending => "todo-pending",
        agena_api::message_part::TodoStatusResource::InProgress => "todo-in-progress",
        agena_api::message_part::TodoStatusResource::Completed => "todo-completed",
        agena_api::message_part::TodoStatusResource::Cancelled => "todo-cancelled",
    };
    ui_text::t(i18n, key)
}

fn todo_priority_resource_label(
    i18n: &I18n,
    priority: agena_api::message_part::TodoPriorityResource,
) -> String {
    let key = match priority {
        agena_api::message_part::TodoPriorityResource::High => "todo-priority-high",
        agena_api::message_part::TodoPriorityResource::Medium => "todo-priority-medium",
        agena_api::message_part::TodoPriorityResource::Low => "todo-priority-low",
    };
    ui_text::t(i18n, key)
}

pub(crate) fn render_user_input_request(
    request: &agena_api::resource::UserInputRequest,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    push_multiline(
        out,
        "  ▸ ",
        &i18n.text_args(
            "message-awaiting-user-input",
            &agena_tui::fl_args!("request_id" => request.request_id.as_str()),
        ),
        Style::default().fg(agena_tui_components::theme::special_color()),
        width,
    );
    for question in &request.questions {
        push_multiline(
            out,
            "    ",
            &ui_text::message_question_line(i18n, question.question.as_str(), question.id.as_str()),
            Style::default(),
            width,
        );
    }
}

pub(crate) fn preview_for_part(part: &TranscriptEntryPart, i18n: &I18n) -> Option<String> {
    match transcript_part_content(part) {
        TranscriptPartContent::UserDocument(document) => {
            first_non_empty_preview_line(document.plain_text().as_str())
        }
        TranscriptPartContent::Text(text) => first_non_empty_preview_line(text.text.as_str()),
        TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload)) => {
            Some(crate::snapshot::activity_presentation(payload).1)
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::TextSegment(segment)) => Some(
            crate::snapshot::activity_presentation(&agena_domain::ActivityPayload::TextSegment(
                segment.as_ref().clone(),
            ))
            .1,
        ),
        TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(reasoning)) => {
            let summary = reasoning.preferred_text();
            first_non_empty_preview_line(summary.as_str())
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Operation(tool)) => {
            Some(tool_execution_preview(part, tool, i18n))
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Error(error)) => {
            Some(error.problem.user.fallback.clone())
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::AssistantReplyLifecycle(
            status,
        )) => Some(match status {
            TranscriptAssistantReplyLifecycle::Running => {
                ui_text::t(i18n, "message-activity-response-running")
            }
            TranscriptAssistantReplyLifecycle::Completed => {
                ui_text::t(i18n, "message-activity-response-completed")
            }
            TranscriptAssistantReplyLifecycle::Failed { problem } => match problem {
                Some(problem) => format!(
                    "{}: {}",
                    ui_text::t(i18n, "message-activity-response-failed"),
                    problem.user.fallback
                ),
                None => ui_text::t(i18n, "message-activity-response-failed"),
            },
            TranscriptAssistantReplyLifecycle::Cancelled => {
                ui_text::t(i18n, "message-activity-response-cancelled")
            }
        }),
        TranscriptPartContent::Activity(TranscriptActivityContent::Attachment(attachment)) => {
            attachment.attachments.first().map(|item| {
                item.title
                    .as_ref()
                    .or(item.filename.as_ref())
                    .cloned()
                    .unwrap_or_else(|| item.mime.clone())
            })
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::SkillReference(reference)) => {
            reference
                .skills
                .first()
                .map(|skill| format!("Skill: {}", skill.name))
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::Request(request)) => {
            match request.as_ref() {
                MessageRequestPartResource::UserInput { request, .. } => request
                    .questions
                    .first()
                    .map(|question| question.question.clone()),
            }
        }
    }
}
