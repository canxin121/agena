use super::{app_detail_labeled_line, app_detail_plain_line};

pub(crate) fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    DateTime::<Local>::from(timestamp)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// Build the terminal item from one ordered session part.
pub(crate) fn build_timeline_item(
    i18n: &I18n,
    record: &crate::app_backend::SessionTimelineEntry,
) -> TimelineItem {
    let label = record
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| timeline_kind_label(i18n, record.kind.as_str()));
    let summary = format!(
        "#{}  {}/{}  {}  {}",
        record.part_id, record.role, record.kind, record.state, label
    );
    let created_at = DateTime::<Utc>::from_timestamp_millis(record.created_at_ms)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let mut detail_lines = vec![
        timeline_detail_labeled_line(i18n, "timeline-label-part-id", record.part_id.to_string()),
        timeline_detail_labeled_line(i18n, "timeline-label-created", format_timestamp(created_at)),
        timeline_detail_labeled_line(i18n, "timeline-label-kind", record.kind.clone()),
        timeline_detail_labeled_line(i18n, "timeline-label-role", record.role.clone()),
        timeline_detail_labeled_line(i18n, "timeline-label-state", record.state.clone()),
        timeline_detail_labeled_line(i18n, "timeline-label-revision", record.revision.to_string()),
    ];
    if let Some(run_id) = record.run_id {
        detail_lines.push(timeline_detail_labeled_line(
            i18n,
            "timeline-label-run",
            run_id.to_string(),
        ));
    }
    if let Some(parent_part_id) = record.parent_part_id {
        detail_lines.push(timeline_detail_labeled_line(
            i18n,
            "timeline-label-parent-part",
            parent_part_id.to_string(),
        ));
    }
    detail_lines.push(app_detail_plain_line(String::new()));
    let body = match serde_json::to_string_pretty(&record.content) {
        Ok(body) => body,
        Err(error) => {
            tracing::error!(
                part_id = record.part_id,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize timeline record content",
                    &error,
                ),
                "timeline detail content could not be rendered"
            );
            "<timeline content serialization failed>".to_owned()
        }
    };
    detail_lines.push(app_detail_plain_line(body.clone()));
    let detail_document =
        build_detail_document(detail_lines.as_slice(), &DetailTextSpec::label_width(16));
    TimelineItem {
        summary,
        detail_body: detail_document.text,
        search_text: format!(
            "{} {} {}",
            body.to_ascii_lowercase(),
            detail_document.plain.to_ascii_lowercase(),
            record.kind.to_ascii_lowercase()
        ),
    }
}

fn timeline_detail_labeled_line(
    i18n: &I18n,
    label_key: &str,
    value: String,
) -> DetailTextLine<'static> {
    app_detail_labeled_line(ui_text::t(i18n, label_key), value)
}

/// Row text for a timeline part the server stored without a summary. The kind
/// names a machine value (`run_started`, `message_part_checkpointed`, …), so a
/// catalog may carry a human label under `timeline-summary-kind-{kind}`; an
/// unnamed kind keeps its raw value instead of a placeholder.
fn timeline_kind_label(i18n: &I18n, kind: &str) -> String {
    let trimmed = kind.trim();
    if trimmed.is_empty() {
        return ui_text::t(i18n, "timeline-label-kind");
    }
    let key = format!("timeline-summary-kind-{}", trimmed.replace('_', "-"));
    i18n.try_text(key.as_str())
        .unwrap_or_else(|| trimmed.to_owned())
}

use crate::{
    DateTime, DetailTextLine, DetailTextSpec, I18n, Local, TimelineItem, Utc,
    build_detail_document, ui_text,
};
