use super::*;
use agena_domain::{ComposerDocument, ComposerNode, InlineDocument, InlineUiReferences};
use agena_storage::kernel::*;
use serde_json::json;

#[tokio::test]
async fn inline_ui_authorized_sidecars_survive_four_byte_source_pages_without_changing_text() {
    let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
        .await
        .unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/inline-content-tests")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create_configured(RequestId::new(), workspace, "A".into(), Default::default())
        .await
        .unwrap()
        .value
        .id;
    let execution = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Explain".into(),
            }]),
        )
        .await
        .unwrap()
        .value
        .id;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, execution)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "prepare".into(),
                name: "inline_ui.prepare".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let reference = format!("ui_{}_0123456789abcdef0123456789abcdef", tool.id().get());
    let document = InlineDocument::from_value(
        json!({"version":1,"blocks":[{"type":"paragraph","id":"fact","text":"内嵌内容"}]}),
    )
    .unwrap();
    tool.complete(RequestId::new(),ToolResult {phase:ToolPhase::Succeeded,value:json!({"invocation":{"name":"inline_ui.prepare"},"raw_output":{"payload":{"inline_ui_prepared":{"reference":reference,"document":document}}}})}).await.unwrap();
    let source = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    let original = format!("开始🌱\n\n[[inline-ui:{reference}]]\n\n结束");
    source.append(&original).await.unwrap();
    source.finish(RequestId::new()).await.unwrap();
    let changes = runtime
        .ports
        .reader
        .watch_content(session, source.id())
        .await
        .unwrap();
    let head = changes.head().unwrap();
    let watch = super::ContentWatch {
        inline_reply: true,
        resource: resource(session.get(), &head),
        session,
        reader: runtime.ports.reader.clone(),
        changes,
    };
    let mut cursor = None;
    let mut raw = String::new();
    let mut refs = InlineUiReferences::new();
    loop {
        let page = watch.read(cursor, 4).await.unwrap();
        for chunk in page.chunks {
            let ContentPayload::Text { text } = chunk.payload else {
                panic!("text source changed kind")
            };
            assert!(text.len() <= 4);
            raw.push_str(&text);
        }
        assert_eq!(page.next_cursor.sequence, raw.len() as u64);
        assert!(
            page.inline_ui.is_some(),
            "partial assistant reply pages retain their inert embed context"
        );
        if let Some(sidecar) = page.inline_ui {
            refs.extend(sidecar);
        }
        if !page.has_more {
            break;
        }
        assert!(cursor.is_none_or(|previous: ContentCursor| page.next_cursor.sequence>previous.sequence));
        cursor = Some(page.next_cursor);
    }
    assert_eq!(
        raw, original,
        "reference resolution must not rewrite stored/delivered bytes"
    );
    assert_eq!(refs.get(&reference), Some(&document));
    assert_eq!(
        agena_domain::expand_inline_references(&raw, &refs, false),
        format!("开始🌱\n\n{}\n\n结束", document.to_markdown())
    );
}
