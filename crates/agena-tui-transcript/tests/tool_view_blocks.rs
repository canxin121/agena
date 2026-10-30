use agena_api::resource::{RunRole, RunStatus};
use agena_domain::{
    ContentCursor, ContentDocument, ContentEpoch, ContentFormat, ContentId, ContentKind,
    ContentRef, ContentResource, ContentState, PartDocument, ToolInvocation, ToolResultState,
    ViewBlock,
};
use agena_runtime_contracts::part::OperationPart;
use agena_tui::i18n::I18n;
use agena_tui_media::{MathRenderContext, with_math_render_context};
use agena_tui_transcript::{
    PartExecutionStatusResource, RenderedMessageBlock, ToolCallView, TranscriptActivityContent,
    TranscriptContentId, TranscriptEntry, TranscriptEntryId, TranscriptEntryPart,
    TranscriptNodeKey, TranscriptPartContent, content::ContentFrame, render_entry_detailed,
};
use serde_json::json;
use std::sync::Arc;
use unicode_width::UnicodeWidthStr;

const IMAGE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABQAAAAoCAYAAAD+MdrbAAAAKUlEQVR4nO3MoQEAAAwCIP9/envCJoFKLrmmCIVCoXA2LH9CoVAo3A0fsaQ6G+7F7JIAAAAASUVORK5CYII=";

fn tool(blocks: Vec<ViewBlock>) -> ToolCallView {
    let mut operation = OperationPart::pending(
        1,
        ToolInvocation::new("audit.tool", Default::default()),
        Default::default(),
    );
    operation.state = ToolResultState::Completed;
    ToolCallView::from_operation(
        operation,
        Some(PartDocument {
            title: "Tool result".into(),
            summary: String::new(),
            blocks,
        }),
    )
}

fn entry(tool: ToolCallView) -> TranscriptEntry<'static> {
    TranscriptEntry {
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
    }
}

fn render(tool: ToolCallView, width: u16) -> RenderedMessageBlock {
    let entry = entry(tool);
    let expansions = [(
        TranscriptNodeKey::Activity {
            entry_id: entry.id,
            content_id: entry.parts[0].id,
        },
        true,
    )]
    .into();
    render_entry_detailed(
        &entry,
        width,
        &I18n::english(),
        &Default::default(),
        &expansions,
    )
}

fn visible_text(rendered: &RenderedMessageBlock) -> String {
    rendered
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn activity_copy(rendered: &RenderedMessageBlock) -> &str {
    &rendered
        .nodes
        .iter()
        .find(|node| matches!(node.key, TranscriptNodeKey::Activity { .. }))
        .unwrap()
        .copy_text
}

fn frame(kind: ContentKind, state: ContentState) -> ContentFrame {
    let cursor = ContentCursor {
        epoch: ContentEpoch::new().into(),
        sequence: 1,
    };
    ContentFrame {
        source_text: Arc::from("content-body"),
        inline_ui: None,
        resource: ContentResource {
            resource_id: ContentId::new(),
            owner_session_id: 1,
            part_id: 2,
            kind,
            state,
            cursor,
            committed_cursor: cursor,
            total_bytes: None,
            dropped_bytes: 0,
            dropped_packets: 0,
            committed_dropped_bytes: 0,
            committed_dropped_packets: 0,
            retained_ranges: Vec::new(),
            capture_error: None,
        },
        lines: if matches!(kind, ContentKind::Log | ContentKind::Terminal) {
            vec![ratatui::text::Line::raw("content-body")]
        } else {
            Vec::new()
        },
        wrapped: Vec::new(),
        terminal_size: (kind == ContentKind::Terminal).then_some((24, 80)),
        gap: false,
        windowed: false,
        text: Arc::from(if kind == ContentKind::Text {
            "content-body"
        } else {
            ""
        }),
        document: matches!(kind, ContentKind::Structured | ContentKind::Document).then(|| {
            Arc::new(ContentDocument {
                blocks: vec![ViewBlock::Text {
                    id: Some("document-body".into()),
                    text: "content-body".into(),
                }],
            })
        }),
    }
}

fn with_content(frame: ContentFrame) -> ToolCallView {
    let resource = frame.resource.reference();
    let mut tool = tool(vec![ViewBlock::Content {
        id: "content".into(),
        resource: resource.clone(),
        format: ContentFormat::Plain,
    }]);
    tool.contents.insert(resource.resource_id, Arc::new(frame));
    tool
}

#[test]
fn every_view_block_kind_has_visible_human_output_and_copyable_facts() {
    let content = frame(ContentKind::Text, ContentState::Complete);
    let cases = [
        (
            "text",
            json!({"type":"text","text":"text-body"}),
            vec!["text-body"],
        ),
        (
            "markdown",
            json!({"type":"markdown","text":"**markdown-body**"}),
            vec!["markdown-body"],
        ),
        (
            "json",
            json!({"type":"json","value":{"result":"json-body"}}),
            vec!["json-body"],
        ),
        (
            "table",
            json!({"type":"table","columns":["Name","Value"],"rows":[["table-body",7]]}),
            vec!["table-body", "Value"],
        ),
        (
            "log",
            json!({"type":"log","stream":"stderr","text":"log-body"}),
            vec!["log-body", "stderr"],
        ),
        (
            "command",
            json!({"type":"command","command":"printf command-body","stdout":"command-out","stderr":"command-err","exit_code":2}),
            vec!["command-body", "command-out", "command-err"],
        ),
        (
            "file_changes",
            json!({"type":"file_changes","changes":[{"path":"changed-file.txt","kind":"updated"}]}),
            vec!["changed-file.txt"],
        ),
        (
            "diff",
            json!({"type":"diff","diff":"--- a/example.txt\n+++ b/example.txt\n@@ -1 +1 @@\n-old-body\n+diff-body\n"}),
            vec!["diff-body", "old-body"],
        ),
        (
            "search_results",
            json!({"type":"search_results","items":[{"title":"search-title","url":"https://example.test/result","snippet":"search-body"}],"total":1}),
            vec!["search-title", "example.test/result", "search-body"],
        ),
        (
            "media",
            json!({"type":"media","artifact":{"uri":IMAGE,"mime":"image/png","name":"media-body.png"}}),
            vec!["media-body.png"],
        ),
        (
            "content",
            json!({"type":"content","id":"content","resource":content.resource.reference()}),
            vec!["content-body"],
        ),
        (
            "progress",
            json!({"type":"progress","id":"progress","phase":"progress-body","completed":3,"total":4,"unit":"items"}),
            vec!["progress-body", "3/4", "items"],
        ),
        (
            "custom",
            json!({"type":"custom","kind":"plugin-summary","presentation":{"summary":"custom-body"}}),
            vec!["custom-body"],
        ),
    ];
    let context = MathRenderContext::new(None, &std::env::temp_dir());
    for (kind, value, expected) in cases {
        let block: ViewBlock = serde_json::from_value(value).unwrap();
        let mut tool = tool(vec![block]);
        tool.contents
            .insert(content.resource.resource_id, Arc::new(content.clone()));
        for width in [60, 100] {
            let rendered = with_math_render_context(&context, || render(tool.clone(), width));
            let visible = visible_text(&rendered);
            let copied = activity_copy(&rendered);
            for expected in &expected {
                assert!(
                    visible.contains(*expected),
                    "{kind} {width}: missing visible {expected}"
                );
                assert!(
                    copied.contains(*expected),
                    "{kind} {width}: missing copied {expected}"
                );
            }
            assert!(!copied.contains("base64,"), "{kind}");
            if kind == "media" {
                assert!(visible.contains('▀'));
                assert!(!copied.contains('▀'));
            }
        }
    }
}

#[test]
fn every_content_resource_kind_retains_output_and_completion_state() {
    for kind in [
        ContentKind::Text,
        ContentKind::Log,
        ContentKind::Terminal,
        ContentKind::Structured,
        ContentKind::Document,
    ] {
        for (state, key) in [
            (ContentState::Active, "content-active"),
            (ContentState::Complete, "content-complete"),
            (ContentState::Interrupted, "content-interrupted"),
        ] {
            let rendered = render(with_content(frame(kind, state)), 100);
            let visible = visible_text(&rendered);
            assert!(visible.contains("content-body"), "{kind:?} {state:?}");
            assert!(
                visible.contains(&I18n::english().text(key)),
                "{kind:?} {state:?}"
            );
            assert!(
                activity_copy(&rendered).contains("content-body"),
                "{kind:?} {state:?}"
            );
        }
    }
}

#[test]
fn content_connection_and_capture_failures_are_visible_wrapped_and_copyable() {
    let message = "The content stream disconnected before the final result was received.";
    let dirty_message = format!("{message}\x1b[31m\x07\x00\x7f\u{202e}");
    let safe_message = message.to_owned();
    let resource = ContentRef {
        resource_id: ContentId::new(),
        kind: ContentKind::Log,
    };
    let mut unloaded = tool(vec![ViewBlock::Content {
        id: "unloaded".into(),
        resource: resource.clone(),
        format: ContentFormat::Plain,
    }]);
    unloaded
        .content_errors
        .insert(resource.resource_id, dirty_message.clone());
    let mut captured = frame(ContentKind::Log, ContentState::Interrupted);
    captured.resource.capture_error = Some(dirty_message.clone());
    let captured = with_content(captured);
    let mut disconnected = with_content(frame(ContentKind::Log, ContentState::Active));
    let id = *disconnected.contents.keys().next().unwrap();
    disconnected.content_errors.insert(id, dirty_message);

    for tool in [unloaded, captured, disconnected] {
        for width in [24, 48, 100] {
            let rendered = render(tool.clone(), width);
            assert!(activity_copy(&rendered).contains(safe_message.as_str()));
            assert!(!activity_copy(&rendered).contains('\x1b'));
            assert!(!activity_copy(&rendered).contains('\x07'));
            assert!(!activity_copy(&rendered).contains('\x00'));
            assert!(!activity_copy(&rendered).contains('\x7f'));
            assert!(!activity_copy(&rendered).contains('\u{202e}'));
            let displayed_rows = rendered
                .lines
                .iter()
                .filter(|line| line.text.starts_with("  │ "))
                .map(|line| line.copy_text.as_str())
                .collect::<Vec<_>>()
                .join("");
            let displayed = displayed_rows
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>();
            let expected = safe_message
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect::<String>();
            assert!(
                displayed.contains(expected.as_str()),
                "missing content error at {width}"
            );
            assert!(
                rendered.lines.iter().all(|line| {
                    UnicodeWidthStr::width(line.text.as_str()) <= usize::from(width)
                }),
                "overflow at {width}"
            );
        }
    }
}

#[test]
fn text_resource_copy_cannot_reintroduce_controls_hidden_from_the_screen() {
    let mut text = frame(ContentKind::Text, ContentState::Complete);
    text.text = Arc::from("content-body\x1b[31m\x07\x00\x7f\u{202e}\x1b]52;c;clipboard-canary\x07");
    text.source_text = text.text.clone();
    let rendered = render(with_content(text), 60);
    let visible = visible_text(&rendered);
    let copied = activity_copy(&rendered);
    assert!(visible.contains("content-body"));
    assert!(copied.contains("content-body"));
    for value in [visible.as_str(), copied] {
        assert!(!value.contains("clipboard-canary"));
        assert!(
            value
                .chars()
                .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        );
        assert!(!value.contains('\u{202e}'));
    }
}

#[test]
fn pending_content_shows_loading_until_the_first_frame() {
    let resource = ContentRef {
        resource_id: ContentId::new(),
        kind: ContentKind::Text,
    };
    let pending = tool(vec![ViewBlock::Content {
        id: "pending".into(),
        resource,
        format: ContentFormat::Plain,
    }]);
    let rendered = render(pending, 60);
    assert!(visible_text(&rendered).contains(&I18n::english().text("content-loading")));
    assert!(!activity_copy(&rendered).contains("content-body"));
}

#[test]
fn custom_schema_fallback_preserves_fields_without_executing_unknown_ui() {
    let schema = json!({"title":"custom-schema-body", "values":[3,5,8]});
    for presentation in [None, Some(json!({}))] {
        let mut value = json!({"type":"custom","kind":"unknown-chart","schema":schema});
        if let Some(presentation) = presentation {
            value["presentation"] = presentation;
        }
        let rendered = render(tool(vec![serde_json::from_value(value).unwrap()]), 60);
        assert!(visible_text(&rendered).contains("custom-schema-body"));
        assert!(visible_text(&rendered).contains("unknown-chart"));
        let copied = activity_copy(&rendered);
        let (_, body) = copied.split_once("unknown-chart\n").unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            schema
        );
    }
    let named_only = render(
        tool(vec![
            serde_json::from_value(json!({
                "type":"custom", "kind":"unknown-chart"
            }))
            .unwrap(),
        ]),
        60,
    );
    assert!(visible_text(&named_only).contains("unknown-chart"));
    assert!(activity_copy(&named_only).contains("unknown-chart"));

    let presented = render(
        tool(vec![
            serde_json::from_value(json!({
                "type":"custom", "kind":"unknown-chart", "schema":schema,
                "presentation":{"summary":"reviewed-human-presentation"}
            }))
            .unwrap(),
        ]),
        60,
    );
    assert!(visible_text(&presented).contains("reviewed-human-presentation"));
    assert!(activity_copy(&presented).contains("reviewed-human-presentation"));
    assert!(!activity_copy(&presented).contains("custom-schema-body"));
}

#[test]
fn image_audio_video_pdf_and_file_results_open_by_default_with_a_named_fallback() {
    let context = MathRenderContext::new(None, &std::env::temp_dir());
    for (mime, filename) in [
        ("image/png", "picture.png"),
        ("audio/wav", "recording.wav"),
        ("video/mp4", "movie.mp4"),
        ("application/pdf", "report.pdf"),
        ("text/plain", "result.txt"),
    ] {
        let source = if mime == "image/png" {
            IMAGE
        } else {
            "file:///server/artifact"
        };
        let block = serde_json::from_value(json!({
            "type":"media", "artifact":{"uri":source,"mime":mime,"name":filename}
        }))
        .unwrap();
        let entry = entry(tool(vec![block]));
        let rendered = with_math_render_context(&context, || {
            render_entry_detailed(
                &entry,
                60,
                &I18n::english(),
                &Default::default(),
                &Default::default(),
            )
        });
        assert!(
            rendered
                .nodes
                .iter()
                .any(
                    |node| matches!(node.key, TranscriptNodeKey::Activity { .. }) && node.expanded
                )
        );
        assert!(visible_text(&rendered).contains(filename));
        assert!(activity_copy(&rendered).contains(filename));
        assert_eq!(visible_text(&rendered).contains('▀'), mime == "image/png");
    }
}
