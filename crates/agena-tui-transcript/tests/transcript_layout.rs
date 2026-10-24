use agena_api::resource::{RunRole, RunStatus};
use agena_domain::{
    PartDocument, TextSegmentActivity, ToolInvocation, UserInputRequest, ViewBlock,
};
use agena_runtime_contracts::part::OperationPart;
use agena_tui::i18n::I18n;
use agena_tui_transcript::{
    InteractionLineKind, PartExecutionStatusResource, PendingInteractionView, ToolCallView,
    TranscriptActivityContent, TranscriptContentId, TranscriptDetailDefaults, TranscriptEntry,
    TranscriptEntryId, TranscriptEntryPart, TranscriptPartContent, TranscriptUserDocument,
    TranscriptUserDocumentNode, render_entry_detailed, render_entry_detailed_with_interactions,
};
use serde_json::json;
use unicode_width::UnicodeWidthStr;

fn entry(role: RunRole, content: TranscriptPartContent<'static>) -> TranscriptEntry<'static> {
    TranscriptEntry {
        id: TranscriptEntryId::StoredMessage(1),
        reply_id: None,
        role: Some(role),
        state: RunStatus::Completed,
        created_at: chrono::Utc::now(),
        parts: vec![TranscriptEntryPart {
            id: TranscriptContentId::StoredPart(1),
            status: PartExecutionStatusResource::InProgress,
            headline: Default::default(),
            content,
        }],
    }
}

#[test]
fn user_and_answer_share_one_body_inset_without_removing_source_indentation() {
    let user = entry(
        RunRole::User,
        TranscriptPartContent::UserDocument(TranscriptUserDocument {
            nodes: vec![TranscriptUserDocumentNode::Text {
                id: None,
                text: "用户正文\n    原文缩进".into(),
            }],
        }),
    );
    let assistant = entry(
        RunRole::Assistant,
        TranscriptPartContent::Activity(TranscriptActivityContent::Answer(Box::new(
            TextSegmentActivity {
                text: "回答正文\n\n- 外层列表\n  - 内层列表".into(),
            },
        ))),
    );
    for width in [20, 48, 80] {
        let user = render_entry_detailed(
            &user,
            width,
            &I18n::english(),
            &Default::default(),
            &Default::default(),
        );
        let assistant = render_entry_detailed(
            &assistant,
            width,
            &I18n::english(),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(user.lines[0].text, "user");
        assert_eq!(assistant.lines[0].text, "assistant");
        assert!(assistant.lines[1].text.starts_with("▾ "));
        for (lines, source) in [(&user.lines, "用户正文"), (&assistant.lines, "回答正文")] {
            let body = lines.iter().find(|line| line.copy_text == source).unwrap();
            assert_eq!(body.text, format!("  {source}"));
            assert_eq!(body.copy_column, 2);
            assert!(
                lines
                    .iter()
                    .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= usize::from(width))
            );
        }
        let indented = user
            .lines
            .iter()
            .find(|line| line.copy_text.contains("原文缩进"))
            .unwrap();
        assert_eq!(indented.copy_text, "    原文缩进");
        assert_eq!(indented.text, "      原文缩进");
        let outer = assistant
            .lines
            .iter()
            .find(|line| line.text.contains("外层列表"))
            .unwrap();
        let inner = assistant
            .lines
            .iter()
            .find(|line| line.text.contains("内层列表"))
            .unwrap();
        assert!(inner.copy_column > outer.copy_column);
    }
}

#[test]
fn question_buttons_and_summary_copy_follow_the_compact_columns() {
    let request: UserInputRequest = serde_json::from_value(json!({
        "request_id": "layout-question", "session_id": 1, "kind": "ask_user", "source": "plugin",
        "created_at": "2026-10-25T00:00:00Z", "questions": [{
            "header": "布局", "question": "请选择显示方式，正文应当保留足够的可用空间。",
            "options": [{"label": "紧凑显示", "description": "保留内容和操作"}],
            "multiple": false, "allow_custom": true
        }]
    }))
    .unwrap();
    let mut operation = OperationPart::pending(
        1,
        ToolInvocation::new("interaction.ask", Default::default()),
        Default::default(),
    );
    operation.user_input.push_pending(request);
    let tool = ToolCallView::from_operation(
        operation,
        Some(PartDocument {
            title: "Question".into(),
            summary: String::new(),
            blocks: Vec::<ViewBlock>::new(),
        }),
    );
    let entry = entry(
        RunRole::Assistant,
        TranscriptPartContent::Activity(TranscriptActivityContent::Operation(Box::new(tool))),
    );
    for width in [20, 40, 80] {
        for page in [0, 1] {
            let views = [(
                "layout-question".into(),
                PendingInteractionView {
                    question_page: page,
                    ..Default::default()
                },
            )]
            .into();
            let rendered = render_entry_detailed_with_interactions(
                &entry,
                width,
                &I18n::english(),
                &TranscriptDetailDefaults::default(),
                &Default::default(),
                &views,
            );
            let mut button_rows = 0;
            let mut summary_rows = 0;
            for line in &rendered.lines {
                assert!(
                    UnicodeWidthStr::width(line.text.as_str()) <= usize::from(width),
                    "{width}: {}",
                    line.text
                );
                for control in &line.controls {
                    let agena_tui_transcript::RenderedTranscriptControl {
                        action:
                            agena_tui_transcript::TranscriptControlAction::UserInput { kind, .. },
                        ..
                    } = control
                    else {
                        continue;
                    };
                    if matches!(
                        kind,
                        InteractionLineKind::AskPrevious
                            | InteractionLineKind::AskNext
                            | InteractionLineKind::AskSubmit
                            | InteractionLineKind::AskCancel
                    ) {
                        assert_eq!(line.controls[0].columns.start, 2);
                        assert!(control.columns.end <= UnicodeWidthStr::width(line.text.as_str()));
                        assert_eq!(line.copy_column, 2);
                        assert_eq!(line.text.strip_prefix("  ").unwrap(), line.copy_text);
                        button_rows += 1;
                    }
                    if matches!(kind, InteractionLineKind::AskSummary { .. }) {
                        assert_eq!(line.copy_column, 2);
                        assert_eq!(line.text.strip_prefix("  ").unwrap(), line.copy_text);
                        summary_rows += 1;
                    }
                }
            }
            assert!(button_rows > 0);
            assert_eq!(summary_rows > 0, page == 1);
        }
    }
}
