use super::{app_detail_labeled_line, app_detail_plain_line};

pub(in crate::app) fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    DateTime::<Local>::from(timestamp)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// Build the terminal item from Runtime's presentation projection. The UI
/// localizes stable labels but never deserializes a generic event payload back
/// into Runtime's concrete event envelope.
pub(in crate::app) fn build_timeline_item(
    i18n: &I18n,
    record: &agena_runtime::RuntimeTimelineEvent,
) -> TimelineItem {
    let event_type = ui_text::t(i18n, record.type_key.as_str());
    let summary = if record.summary.trim().is_empty() {
        format!("#{}  {}", record.meta.seq_global, event_type)
    } else {
        format!(
            "#{}  {}  {}",
            record.meta.seq_global, event_type, record.summary
        )
    };
    let mut detail_lines = vec![
        timeline_detail_labeled_line(
            i18n,
            "timeline-label-seq",
            record.meta.seq_global.to_string(),
        ),
        timeline_detail_labeled_line(
            i18n,
            "timeline-label-created",
            format_timestamp(record.meta.created_at),
        ),
        timeline_detail_labeled_line(i18n, "timeline-label-type", event_type),
        timeline_detail_labeled_line(i18n, "timeline-label-event-id", record.meta.id.to_string()),
    ];
    if let Some(causation_id) = record.meta.causation_id {
        detail_lines.push(timeline_detail_labeled_line(
            i18n,
            "timeline-label-causation-id",
            causation_id.to_string(),
        ));
    }
    if let Some(correlation_id) = record.meta.correlation_id {
        detail_lines.push(timeline_detail_labeled_line(
            i18n,
            "timeline-label-correlation-id",
            correlation_id.to_string(),
        ));
    }
    detail_lines.push(app_detail_plain_line(String::new()));
    for line in &record.detail_lines {
        detail_lines.push(timeline_detail_labeled_line(
            i18n,
            line.label.as_str(),
            line.value.clone(),
        ));
    }
    let detail_document =
        build_detail_document(detail_lines.as_slice(), &DetailTextSpec::label_width(16));
    TimelineItem {
        summary,
        detail_body: detail_document.text,
        search_text: format!(
            "{} {} {}",
            record.search_text,
            detail_document.plain.to_ascii_lowercase(),
            record.kind.to_ascii_lowercase(),
        ),
        linked_message_id: record.linked_message_id,
    }
}

fn timeline_detail_labeled_line(
    i18n: &I18n,
    label_key: &str,
    value: String,
) -> DetailTextLine<'static> {
    app_detail_labeled_line(ui_text::t(i18n, label_key), value)
}

use crate::app::{
    DateTime, DetailTextLine, DetailTextSpec, I18n, Local, TimelineItem, Utc,
    build_detail_document, ui_text,
};
