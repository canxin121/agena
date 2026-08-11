use super::{app_detail_labeled_line, app_detail_plain_line};

pub(crate) fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    DateTime::<Local>::from(timestamp)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// Build the terminal item from one ordered v2 session part.
pub(crate) fn build_timeline_item(
    i18n: &I18n,
    record: &crate::app_backend::SessionTimelineEntry,
) -> TimelineItem {
    let label = record
        .summary
        .as_deref()
        .filter(|summary| !summary.trim().is_empty())
        .unwrap_or(record.kind.as_str());
    let summary = format!(
        "#{}  {}/{}  {}  {}",
        record.part_id, record.role, record.kind, record.state, label
    );
    let created_at = DateTime::<Utc>::from_timestamp_millis(record.created_at_ms)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    let mut detail_lines = vec![
        app_detail_labeled_line("Part ID", record.part_id.to_string()),
        timeline_detail_labeled_line(i18n, "timeline-label-created", format_timestamp(created_at)),
        app_detail_labeled_line("Kind", record.kind.clone()),
        app_detail_labeled_line("Role", record.role.clone()),
        app_detail_labeled_line("State", record.state.clone()),
        app_detail_labeled_line("Revision", record.revision.to_string()),
    ];
    if let Some(run_id) = record.run_id {
        detail_lines.push(app_detail_labeled_line("Run", run_id.to_string()));
    }
    if let Some(parent_part_id) = record.parent_part_id {
        detail_lines.push(app_detail_labeled_line(
            "Parent part",
            parent_part_id.to_string(),
        ));
    }
    detail_lines.push(app_detail_plain_line(String::new()));
    let body = record
        .rendered_markdown
        .clone()
        .unwrap_or_else(|| serde_json::to_string_pretty(&record.content).unwrap_or_default());
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

use crate::{
    DateTime, DetailTextLine, DetailTextSpec, I18n, Local, TimelineItem, Utc,
    build_detail_document, ui_text,
};
