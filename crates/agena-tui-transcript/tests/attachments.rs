use agena_api::part::AttachmentPartResource;
use agena_api::resource::{PartAttachment, RunRole, RunStatus};
use agena_domain::{AttachmentItem, PartDocument, RawOutput, ToolInvocation, ViewBlock};
use agena_runtime_contracts::part::OperationPart;
use agena_tui::i18n::I18n;
use agena_tui_media::{MathRenderContext, with_math_render_context, with_text_math_rendering};
use agena_tui_transcript::{
    PartExecutionStatusResource, ToolCallView, TranscriptActivityContent, TranscriptContentId,
    TranscriptEntry, TranscriptEntryId, TranscriptEntryPart, TranscriptNodeKey,
    TranscriptPartContent, render_entry_detailed,
};
use serde_json::json;

// 20 x 40 RGBA pixels: red upper half and blue lower half.
const IMAGE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABQAAAAoCAYAAAD+MdrbAAAAKUlEQVR4nO3MoQEAAAwCIP9/envCJoFKLrmmCIVCoXA2LH9CoVAo3A0fsaQ6G+7F7JIAAAAASUVORK5CYII=";

fn attachment() -> AttachmentItem {
    serde_json::from_value(json!({
        "kind": "image",
        "mime": "image/png",
        "filename": "generated.png",
        "sha256": "generated-image",
        "source": {"source": "data_url", "url": IMAGE}
    }))
    .unwrap()
}

fn media() -> ViewBlock {
    serde_json::from_value(json!({
        "type": "media",
        "id": "generated-image",
        "artifact": {
            "uri": IMAGE,
            "mime": "image/png",
            "name": "generated.png",
            "sha256": "generated-image"
        }
    }))
    .unwrap()
}

fn entry(content: TranscriptPartContent<'static>) -> TranscriptEntry<'static> {
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
            content,
        }],
    }
}

fn operation(attachments: Vec<AttachmentItem>, blocks: Vec<ViewBlock>) -> TranscriptEntry<'static> {
    let mut operation = OperationPart::pending(
        1,
        ToolInvocation::new("image.generate", Default::default()),
        Default::default(),
    );
    operation.output = Some(RawOutput {
        attachments,
        ..Default::default()
    });
    entry(TranscriptPartContent::Activity(
        TranscriptActivityContent::Operation(Box::new(ToolCallView::from_operation(
            operation,
            Some(PartDocument {
                title: "Generated image".into(),
                summary: String::new(),
                blocks,
            }),
        ))),
    ))
}

#[test]
fn generated_images_open_by_default_and_render_once_without_a_native_protocol() {
    let entry = operation(vec![attachment()], vec![media()]);
    let context = MathRenderContext::new(None, &std::env::temp_dir());
    let rendered = with_math_render_context(&context, || {
        render_entry_detailed(
            &entry,
            60,
            &I18n::english(),
            &Default::default(),
            &Default::default(),
        )
    });
    let activity = rendered
        .nodes
        .iter()
        .find(|node| matches!(node.key, TranscriptNodeKey::Activity { .. }))
        .unwrap();
    assert!(activity.expanded);
    assert!(rendered.lines.iter().any(|line| line.text.contains('▀')));
    assert!(rendered.lines.iter().all(|line| line.math.is_empty()));
    assert_eq!(
        rendered
            .lines
            .iter()
            .filter(|line| line.text.contains('🖼'))
            .count(),
        1
    );
    assert!(
        !rendered
            .lines
            .iter()
            .any(|line| line.text.contains("base64,"))
    );

    let expansions = [(activity.key.clone(), false)].into();
    let folded = with_math_render_context(&context, || {
        render_entry_detailed(
            &entry,
            60,
            &I18n::english(),
            &Default::default(),
            &expansions,
        )
    });
    assert!(!folded.lines.iter().any(|line| line.text.contains('▀')));
    assert!(
        !folded
            .nodes
            .iter()
            .find(|node| node.key == activity.key)
            .unwrap()
            .expanded
    );
}

#[test]
fn media_only_results_and_standalone_attachments_render_image_previews() {
    let item: PartAttachment =
        serde_json::from_value(serde_json::to_value(attachment()).unwrap()).unwrap();
    let entries = [
        operation(Vec::new(), vec![media()]),
        entry(TranscriptPartContent::Activity(
            TranscriptActivityContent::Attachment(AttachmentPartResource {
                attachments: vec![item],
            }),
        )),
    ];
    let context = MathRenderContext::new(None, &std::env::temp_dir());
    for entry in entries {
        let rendered = with_math_render_context(&context, || {
            render_entry_detailed(
                &entry,
                60,
                &I18n::english(),
                &Default::default(),
                &Default::default(),
            )
        });
        assert!(rendered.nodes.iter().any(|node| {
            matches!(node.key, TranscriptNodeKey::Activity { .. }) && node.expanded
        }));
        assert!(rendered.lines.iter().any(|line| line.text.contains('▀')));
    }
}

#[test]
fn ordinary_file_outputs_are_visible_and_text_exports_do_not_contain_image_pixels() {
    let file = serde_json::from_value(json!({
        "kind": "file", "mime": "text/plain", "filename": "report.txt",
        "source": {"source": "local_path", "path": "/server/managed/report.txt"}
    }))
    .unwrap();
    let entry = operation(vec![attachment(), file], Vec::new());
    let context = MathRenderContext::new(None, &std::env::temp_dir());
    let rendered = with_math_render_context(&context, || {
        with_text_math_rendering(|| {
            render_entry_detailed(
                &entry,
                60,
                &I18n::english(),
                &Default::default(),
                &Default::default(),
            )
        })
    });
    assert!(
        rendered
            .lines
            .iter()
            .any(|line| line.text.contains("generated.png"))
    );
    assert!(
        rendered
            .lines
            .iter()
            .any(|line| line.text.contains("report.txt"))
    );
    assert!(rendered.lines.iter().all(|line| {
        !line.text.contains('▀') && line.math.is_empty() && line.remote_image_sources.is_empty()
    }));
    assert!(
        !rendered
            .lines
            .iter()
            .any(|line| line.text.contains("base64,"))
    );
}

#[test]
fn open_technical_sections_keep_preview_pixels_and_summarize_inline_image_bytes() {
    use agena_tui_transcript::TranscriptActivitySection;
    let entry = operation(vec![attachment()], vec![media()]);
    let context = MathRenderContext::new(None, &std::env::temp_dir());
    let expansions = [
        TranscriptActivitySection::TechnicalDetails,
        TranscriptActivitySection::Output,
        TranscriptActivitySection::RenderData,
    ]
    .into_iter()
    .map(|section| {
        (
            TranscriptNodeKey::ActivitySection {
                entry_id: entry.id,
                content_id: TranscriptContentId::StoredPart(2),
                section,
            },
            true,
        )
    })
    .collect();
    let rendered = with_math_render_context(&context, || {
        render_entry_detailed(
            &entry,
            100,
            &I18n::english(),
            &Default::default(),
            &expansions,
        )
    });
    assert!(rendered.lines.iter().any(|line| line.text.contains('▀')));
    assert_eq!(
        rendered
            .lines
            .iter()
            .filter(|line| line.text.contains('🖼'))
            .count(),
        1
    );
    assert!(
        rendered
            .nodes
            .iter()
            .any(|node| node.copy_text.contains("encoded bytes omitted"))
    );
    assert!(
        rendered
            .nodes
            .iter()
            .any(|node| node.copy_text.contains("generated.png"))
    );
    assert!(
        rendered
            .nodes
            .iter()
            .all(|node| !node.copy_text.contains("iVBORw0KGgo"))
    );
    assert!(
        rendered
            .lines
            .iter()
            .all(|line| !line.text.contains("iVBORw0KGgo"))
    );
}
