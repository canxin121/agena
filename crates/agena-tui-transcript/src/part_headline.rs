//! Semantic headlines shared by transcript rows and history previews. A
//! headline is a bounded preview; it never replaces or rewrites the body.

use agena_api::resource::{PartAttachment, PartAttachmentSource};
use agena_domain::{ActivityPayload, NoticeActivity};
use agena_runtime_contracts::part_content::{
    CompactionContent, HookContent, SystemNotificationContent,
};
use agena_tui::i18n::I18n;

use crate::{
    PartExecutionStatusResource, TranscriptActivityContent as Activity,
    TranscriptAssistantReplyLifecycle, TranscriptEntryPart, TranscriptPartContent,
    TranscriptPartHeadline, ui_text,
};

/// First meaningful source line for a compact part summary. The full source
/// remains authoritative for body rendering and copying.
pub fn first_content_line(text: &str) -> Option<String> {
    crate::sanitize_terminal_text(text)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find_map(|line| {
            // Markdown headings are useful summaries, but their block marker
            // does not belong in a one-line headline.
            let heading = line.trim_start_matches('#');
            let line = if line.len() > heading.len()
                && line.len() - heading.len() <= 6
                && (heading.is_empty() || heading.starts_with(' '))
            {
                heading.trim()
            } else {
                line
            };
            // Fence and rule lines describe Markdown structure, not the
            // snippet's subject. Preview the first content line after them.
            let rule = line
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            if line.is_empty()
                || line.starts_with("```")
                || line.starts_with("~~~")
                || (rule.len() >= 3
                    && ['-', '*', '_']
                        .iter()
                        .any(|marker| rule.chars().all(|character| character == *marker)))
            {
                None
            } else {
                Some(line.to_owned())
            }
        })
}

fn first_content(texts: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    texts
        .into_iter()
        .find_map(|text| first_content_line(text.as_ref()))
        .unwrap_or_default()
}

fn first_literal(texts: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    texts
        .into_iter()
        .map(|text| crate::sanitize_terminal_text(text.as_ref()))
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|text| !text.is_empty())
        .unwrap_or_default()
}

fn status_summary(status: PartExecutionStatusResource, i18n: &I18n) -> String {
    ui_text::t(
        i18n,
        match status {
            PartExecutionStatusResource::Pending => "activity-summary-pending",
            PartExecutionStatusResource::InProgress => "activity-summary-streaming",
            PartExecutionStatusResource::Completed => "activity-summary-empty",
            PartExecutionStatusResource::Failed => "message-activity-response-failed",
            PartExecutionStatusResource::Cancelled => "message-activity-response-cancelled",
            PartExecutionStatusResource::PolicyDenied => "activity-summary-policy-denied",
            PartExecutionStatusResource::UserDeclined => "activity-summary-user-declined",
            PartExecutionStatusResource::CapabilityUnavailable
            | PartExecutionStatusResource::ToolUnavailable => "activity-summary-unavailable",
        },
    )
}

pub(crate) fn attachment_label(item: &PartAttachment, i18n: &I18n) -> String {
    let source = match &item.source {
        PartAttachmentSource::Url { url } => url.as_str(),
        PartAttachmentSource::LocalPath { path } => path.as_str(),
        PartAttachmentSource::FileId { file_id } => file_id.as_str(),
        // Opaque/base64 data must never become a headline.
        _ => "",
    };
    let label = first_literal([
        item.title.as_deref().unwrap_or_default(),
        item.filename.as_deref().unwrap_or_default(),
        source,
        item.mime.as_str(),
    ]);
    if label.is_empty() {
        ui_text::t(i18n, "message-input-activity-attachment")
    } else {
        label
    }
}

pub(crate) fn hook_notice(hook: &HookContent) -> NoticeActivity {
    NoticeActivity {
        kind: "hook".into(),
        title: None,
        summary: if !hook.summary.trim().is_empty() {
            hook.summary.clone()
        } else {
            first_content([
                hook.summary.as_str(),
                hook.message.as_deref().unwrap_or_default(),
                hook.detail.as_deref().unwrap_or_default(),
                hook.hook.as_str(),
                hook.plugin_id.as_deref().unwrap_or_default(),
            ])
        },
        detail: hook
            .message
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .or(hook.detail.as_ref())
            .cloned(),
        occurred_at_ms: None,
    }
}

pub(crate) fn notification_notice(notification: &SystemNotificationContent) -> NoticeActivity {
    NoticeActivity {
        kind: "system_notification".into(),
        title: None,
        summary: notification.summary.clone(),
        detail: notification
            .detail
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .cloned()
            .or_else(|| (!notification.body.trim().is_empty()).then(|| notification.body.clone())),
        occurred_at_ms: None,
    }
}

pub(crate) fn compaction_notice(compaction: &CompactionContent, i18n: &I18n) -> NoticeActivity {
    let summary = if let Some(label) = match compaction.outcome.as_deref() {
        Some("running") => Some("message-activity-compact-running"),
        Some("unchanged") => Some("message-compaction-unchanged"),
        Some("failed") => Some("message-activity-compact-failed"),
        Some("cancelled") => Some("message-activity-compact-cancelled"),
        _ => None,
    } {
        i18n.text(label)
    } else if let Some(text) = compaction
        .summary_text()
        .filter(|text| !text.trim().is_empty())
    {
        text.to_owned()
    } else if let Some((before, after)) = compaction.checkpoint.as_ref().and_then(|checkpoint| {
        Some((
            checkpoint.get("before_tokens")?.as_u64()?,
            checkpoint.get("after_tokens")?.as_u64()?,
        ))
    }) {
        let percent = before
            .saturating_sub(after)
            .saturating_mul(100)
            .checked_div(before)
            .unwrap_or_default();
        i18n.text_args(
            "message-compaction-token-summary",
            &agena_tui::fl_args!("before" => before, "after" => after, "percent" => percent),
        )
    } else {
        first_content([compaction.detail.as_deref().unwrap_or_default()])
    };
    NoticeActivity {
        kind: "compaction".into(),
        title: None,
        summary: if summary.is_empty() {
            i18n.text("message-compaction-title-manual")
        } else {
            summary
        },
        // Failure details and the completed summary remain inspectable even
        // when the outcome supplies the headline.
        detail: compaction
            .detail
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .cloned()
            .or_else(|| {
                compaction
                    .problem
                    .as_ref()
                    .map(|problem| problem.user.fallback.clone())
            })
            .or_else(|| {
                matches!(
                    compaction.outcome.as_deref(),
                    Some("running" | "unchanged" | "failed" | "cancelled")
                )
                .then(|| compaction.summary_text().map(str::to_owned))
                .flatten()
            }),
        occurred_at_ms: None,
    }
}

fn canonical_headline(payload: &ActivityPayload, i18n: &I18n) -> TranscriptPartHeadline {
    let (_, title, summary, _) = crate::activity_presentation::activity_presentation(payload);
    let summary = if summary.trim().is_empty() {
        match payload {
            ActivityPayload::Notice(notice) => {
                first_content([notice.detail.as_deref().unwrap_or_default()])
            }
            _ => summary,
        }
    } else {
        summary
    };
    TranscriptPartHeadline {
        title: ui_text::localized_activity_title(i18n, &title),
        summary,
    }
}

pub(crate) fn part_headline(part: &TranscriptEntryPart, i18n: &I18n) -> TranscriptPartHeadline {
    let (title, summary) = match &part.content {
        TranscriptPartContent::UserDocument(document) => (String::new(), document.plain_text()),
        TranscriptPartContent::Text(text) => (String::new(), text.text.clone()),
        TranscriptPartContent::Activity(activity) => match activity {
            Activity::Canonical(payload) => {
                let headline = canonical_headline(payload, i18n);
                (headline.title, headline.summary)
            }
            Activity::TextArtifact(artifact) => {
                let headline = canonical_headline(
                    &ActivityPayload::TextArtifact(artifact.as_ref().clone()),
                    i18n,
                );
                (headline.title, headline.summary)
            }
            Activity::TextSegment(segment) | Activity::Answer(segment) => (
                ui_text::localized_activity_title(
                    i18n,
                    if matches!(activity, Activity::Answer(_)) {
                        "Answer"
                    } else {
                        "Text"
                    },
                ),
                segment.text.clone(),
            ),
            Activity::Reasoning(reasoning) => (
                ui_text::localized_activity_title(i18n, "Thinking"),
                first_content_line(&reasoning.summary_text())
                    .or_else(|| first_content_line(&reasoning.raw_text()))
                    .unwrap_or_default(),
            ),
            Activity::Attachment(attachment) => (
                ui_text::t(i18n, "message-input-activity-attachment"),
                attachment
                    .attachments
                    .iter()
                    .map(|item| attachment_label(item, i18n))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Activity::CommandReference(reference) => (
                ui_text::t(i18n, "message-input-activity-skill"),
                reference
                    .commands
                    .iter()
                    .map(|command| {
                        first_literal([
                            command.name.as_str(),
                            command.description.as_str(),
                            command.source.as_str(),
                        ])
                    })
                    .filter(|label| !label.is_empty())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Activity::Error(error) => (
                ui_text::localized_activity_title(i18n, "Error"),
                error.problem.user.fallback.clone(),
            ),
            Activity::Operation(tool) => {
                let title = crate::renderer::tool_display_label_localized(tool, i18n);
                let title = if title.trim().is_empty() {
                    ui_text::t(i18n, "activity-title-tool-call")
                } else {
                    title
                };
                let input_label =
                    crate::renderer::tool_invocation_label(&tool.operation.invocation);
                let subject = input_label
                    .strip_prefix(tool.operation.invocation.name.as_str())
                    .unwrap_or_default()
                    .trim()
                    .trim_start_matches('·')
                    .trim();
                let records = &tool.operation.user_input.requests;
                let pending_id = crate::interaction_view::interaction_request_id_for_part(part);
                let primary = pending_id
                    .and_then(|id| {
                        records
                            .iter()
                            .find(|record| record.request.request_id == id)
                    })
                    .or_else(|| records.first());
                let question = primary
                    .and_then(|record| {
                        record
                            .request
                            .questions
                            .iter()
                            .find_map(|question| first_content_line(&question.question))
                    })
                    .unwrap_or_default();
                let request_title = primary
                    .and_then(|record| first_content_line(&record.request.title))
                    .unwrap_or_default();
                let summary = first_content([
                    question.as_str(),
                    request_title.as_str(),
                    tool.error_message().unwrap_or_default(),
                    tool.summary(),
                    if title.contains(subject) { "" } else { subject },
                ]);
                (title, summary)
            }
            Activity::Notice(notice) => {
                let headline = canonical_headline(
                    &ActivityPayload::Notice(NoticeActivity {
                        kind: notice.kind.clone(),
                        title: notice.title.clone(),
                        summary: notice.summary.clone(),
                        detail: notice.detail.clone(),
                        occurred_at_ms: None,
                    }),
                    i18n,
                );
                (headline.title, headline.summary)
            }
            Activity::Hook(hook) => {
                let headline =
                    canonical_headline(&ActivityPayload::Notice(hook_notice(hook)), i18n);
                (headline.title, headline.summary)
            }
            Activity::Compaction(compaction) => {
                let headline = canonical_headline(
                    &ActivityPayload::Notice(compaction_notice(compaction, i18n)),
                    i18n,
                );
                (headline.title, headline.summary)
            }
            Activity::SystemNotification(notification) => {
                let headline = canonical_headline(
                    &ActivityPayload::Notice(notification_notice(notification)),
                    i18n,
                );
                let summary = first_content([
                    headline.summary.as_str(),
                    notification.status.as_str(),
                    notification.operation_id.as_str(),
                ]);
                (headline.title, summary)
            }
            Activity::AssistantReplyLifecycle(status) => (
                ui_text::t(
                    i18n,
                    match status {
                        TranscriptAssistantReplyLifecycle::Running => {
                            "message-activity-response-running"
                        }
                        TranscriptAssistantReplyLifecycle::Completed => {
                            "message-activity-response-completed"
                        }
                        TranscriptAssistantReplyLifecycle::Cancelled => {
                            "message-activity-response-cancelled"
                        }
                    },
                ),
                String::new(),
            ),
            Activity::Fold { hidden_count } => (
                String::new(),
                crate::renderer::fold_marker_summary(i18n, *hidden_count),
            ),
        },
    };
    let supplied_title = crate::sanitize_terminal_text(&part.headline.title)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let title = if supplied_title.is_empty() {
        title
    } else {
        ui_text::localized_activity_title(i18n, &supplied_title)
    };
    let literal_summary = matches!(
        part.content,
        TranscriptPartContent::Activity(Activity::Attachment(_) | Activity::CommandReference(_))
    );
    let summary = if literal_summary {
        [part.headline.summary.as_str(), summary.as_str()]
            .into_iter()
            .find(|text| !text.trim().is_empty())
            .unwrap_or_default()
            .to_owned()
    } else {
        first_content([part.headline.summary.as_str(), summary.as_str()])
    };
    let summary = if summary.is_empty()
        && !matches!(
            part.content,
            TranscriptPartContent::Activity(Activity::AssistantReplyLifecycle(_))
        ) {
        if matches!(&part.content, TranscriptPartContent::Activity(Activity::Reasoning(reasoning))
            if reasoning.encrypted_content.as_ref().is_some_and(|text| !text.is_empty()))
        {
            ui_text::t(i18n, "activity-summary-reasoning-private")
        } else if part.status == PartExecutionStatusResource::Completed
            && matches!(
                part.content,
                TranscriptPartContent::Activity(
                    Activity::Operation(_)
                        | Activity::Attachment(_)
                        | Activity::CommandReference(_)
                )
            )
        {
            ui_text::t(i18n, "activity-summary-completed")
        } else {
            status_summary(part.status, i18n)
        }
    } else {
        summary
    };
    TranscriptPartHeadline { title, summary }
}
