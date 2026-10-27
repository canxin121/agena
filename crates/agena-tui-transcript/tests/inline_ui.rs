use agena_api::resource::{RunRole, RunStatus};
use agena_domain::{InlineDocument, PartDocument, ToolInvocation, ViewBlock};
use agena_runtime_contracts::part::OperationPart;
use agena_tui::i18n::I18n;
use agena_tui_transcript::{
    PartExecutionStatusResource, ToolCallView, TranscriptActivityContent, TranscriptContentId,
    TranscriptEntry, TranscriptEntryId, TranscriptEntryPart, TranscriptNodeKey,
    TranscriptPartContent, render_entry_detailed,
};
use unicode_width::UnicodeWidthStr;

#[test]
fn inline_ui_is_readable_without_expanding_tools_at_narrow_and_wide_widths() {
    let document = InlineDocument::from_value(
        serde_json::from_str(include_str!("../../../examples/inline-ui/reply.json")).unwrap(),
    )
    .unwrap();
    let operation = OperationPart::pending(
        1,
        ToolInvocation::new("inline_ui.present", Default::default()),
        Default::default(),
    );
    let tool = ToolCallView::from_operation(
        operation,
        Some(PartDocument {
            title: document.title.clone(),
            summary: String::new(),
            blocks: vec![ViewBlock::InlineUi {
                id: Some("inline-reply".into()),
                document: document.clone(),
            }],
        }),
    );
    let entry = TranscriptEntry {
        id: TranscriptEntryId::StoredMessage(1),
        reply_id: None,
        role: Some(RunRole::Assistant),
        state: RunStatus::Completed,
        created_at: chrono::Utc::now(),
        parts: vec![TranscriptEntryPart {
            id: TranscriptContentId::StoredPart(2),
            status: PartExecutionStatusResource::Completed,
            headline: Default::default(),
            content: TranscriptPartContent::Activity(TranscriptActivityContent::Operation(
                Box::new(tool),
            )),
        }],
    };
    let i18n = I18n::english();
    for width in [28, 48, 80, 120] {
        for expanded in [false, true] {
            let expansion = [(
                TranscriptNodeKey::Activity {
                    entry_id: entry.id,
                    content_id: entry.parts[0].id,
                },
                expanded,
            )]
            .into();
            let rendered =
                render_entry_detailed(&entry, width, &i18n, &Default::default(), &expansion);
            let text = rendered
                .lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                text.contains("实施顺序"),
                "collapsed={expanded}, width={width}: {text}"
            );
            assert!(text.contains("为什么不直接"), "details disappeared: {text}");
            assert!(
                !text.contains("\"blocks\""),
                "raw JSON must not be the normal display"
            );
            for line in &rendered.lines {
                assert!(
                    line.text.width() <= usize::from(width),
                    "overflow {width}: {:?}",
                    line.text
                );
            }
        }
    }
}
