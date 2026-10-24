use super::super::{
    I18n, Modifier, RenderedLine, Style, first_non_empty_preview_line, push_label_value,
    push_section_heading, transcript_part_content,
};
use crate::ui_text;
use crate::{TranscriptActivityContent, TranscriptEntryPart, TranscriptPartContent};

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
        &format!("  {}", ui_text::t(i18n, "message-file-changes")),
        Style::default()
            .fg(agena_tui_components::theme::special_color())
            .add_modifier(Modifier::BOLD),
        width,
    );
    for entry in changes {
        push_label_value(
            out,
            "    - ",
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
        TranscriptPartContent::Activity(TranscriptActivityContent::Fold { .. }) => None,
        TranscriptPartContent::Activity(_) => {
            let headline = crate::part_headline::part_headline(part, i18n);
            Some(
                if headline.summary.is_empty() || headline.summary == headline.title {
                    headline.title
                } else {
                    format!("{} · {}", headline.title, headline.summary)
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TranscriptContentId;
    use agena_api::part::{PartExecutionStatusResource, ReasoningPartResource};
    use agena_domain::TextSegmentActivity;
    use agena_runtime_contracts::part_content::NoticeContent;

    #[test]
    fn history_previews_include_the_part_type_and_meaningful_content() {
        for (content, expected) in [
            (
                TranscriptActivityContent::Reasoning(ReasoningPartResource {
                    summary: vec!["\n\nInspect output\nMore reasoning".into()],
                    raw_content: vec![],
                    encrypted_content: None,
                }),
                "Thinking · Inspect output",
            ),
            (
                TranscriptActivityContent::Answer(Box::new(TextSegmentActivity {
                    text: "# Answer topic\nAnswer body".into(),
                })),
                "Answer · Answer topic",
            ),
            (
                TranscriptActivityContent::Notice(Box::new(NoticeContent {
                    summary: " ".into(),
                    detail: Some("Notice detail\nRemaining detail".into()),
                    ..Default::default()
                })),
                "Notice · Notice detail",
            ),
        ] {
            let part = TranscriptEntryPart {
                id: TranscriptContentId::StoredPart(1),
                status: PartExecutionStatusResource::Completed,
                headline: Default::default(),
                content: TranscriptPartContent::Activity(content),
            };
            assert_eq!(
                preview_for_part(&part, &I18n::english()).as_deref(),
                Some(expected)
            );
        }
    }
}
