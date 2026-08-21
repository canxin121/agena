use super::super::{
    I18n, Modifier, RenderedLine, Style, first_non_empty_preview_line, push_label_value,
    push_section_heading, tool_execution_preview, transcript_part_content,
};
use crate::ui_text;
use crate::{
    TranscriptActivityContent, TranscriptAssistantReplyLifecycle, TranscriptEntryPart,
    TranscriptPartContent,
};

pub(crate) fn render_file_changes(
    changes: &[agena_domain::FileChangeRecord],
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
            &file_change_list_item_text(entry, i18n),
            file_change_style(entry.kind),
            width,
        );
    }
}

fn file_change_list_item_text(change: &agena_domain::FileChangeRecord, i18n: &I18n) -> String {
    let marker = match change.kind {
        agena_domain::FileChangeKind::Added => "A",
        agena_domain::FileChangeKind::Updated => "M",
        agena_domain::FileChangeKind::Deleted => "D",
        agena_domain::FileChangeKind::Moved => "R",
    };
    let label = match change.kind {
        agena_domain::FileChangeKind::Added => ui_text::t(i18n, "file-change-added"),
        agena_domain::FileChangeKind::Updated => ui_text::t(i18n, "file-change-updated"),
        agena_domain::FileChangeKind::Deleted => ui_text::t(i18n, "file-change-deleted"),
        agena_domain::FileChangeKind::Moved => ui_text::t(i18n, "file-change-moved"),
    };
    let path = change
        .from_path
        .as_ref()
        .map(|from| format!("{from} → {}", change.path))
        .unwrap_or_else(|| change.path.clone());
    format!("{marker} {path} ({label})")
}

fn file_change_style(kind: agena_domain::FileChangeKind) -> Style {
    match kind {
        agena_domain::FileChangeKind::Added => {
            Style::default().fg(agena_tui_components::theme::success_color())
        }
        agena_domain::FileChangeKind::Updated => {
            Style::default().fg(agena_tui_components::theme::warning_color())
        }
        agena_domain::FileChangeKind::Deleted => {
            Style::default().fg(agena_tui_components::theme::danger_color())
        }
        agena_domain::FileChangeKind::Moved => {
            Style::default().fg(agena_tui_components::theme::accent_color())
        }
    }
}

pub(crate) fn preview_for_part(part: &TranscriptEntryPart, i18n: &I18n) -> Option<String> {
    match transcript_part_content(part) {
        TranscriptPartContent::UserDocument(document) => {
            first_non_empty_preview_line(document.plain_text().as_str())
        }
        TranscriptPartContent::Text(text) => first_non_empty_preview_line(text.text.as_str()),
        TranscriptPartContent::Activity(TranscriptActivityContent::Canonical(payload)) => {
            Some(crate::activity_presentation::activity_presentation(payload).1)
        }
        TranscriptPartContent::Activity(TranscriptActivityContent::TextSegment(segment))
        | TranscriptPartContent::Activity(TranscriptActivityContent::Answer(segment)) => Some(
            crate::activity_presentation::activity_presentation(
                &agena_domain::ActivityPayload::TextSegment(segment.as_ref().clone()),
            )
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
        TranscriptPartContent::Activity(TranscriptActivityContent::Hook(hook)) => {
            Some(hook.summary.clone())
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
        TranscriptPartContent::Activity(TranscriptActivityContent::Fold { .. }) => None,
    }
}
