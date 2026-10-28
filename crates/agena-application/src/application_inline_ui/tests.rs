use super::*;
use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use serde_json::json;

#[tokio::test]
async fn inline_ui_resolution_is_immutable_and_session_scoped() {
    let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
        .await
        .unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/inline-ui-tests")
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
    let unrelated = runtime
        .ports
        .sessions
        .create_configured(RequestId::new(), workspace, "B".into(), Default::default())
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
                call_id: "prepare1".into(),
                name: "inline_ui.prepare".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let reference = format!("ui_{}_0123456789abcdef0123456789abcdef", tool.id().get());
    let document = InlineDocument::from_value(
        json!({"version":1,"blocks":[{"type":"callout","id":"fact","text":"Original fact"}]}),
    )
    .unwrap();
    let value = json!({"invocation":{"name":"inline_ui.prepare"},"raw_output":{"payload":{"inline_ui_prepared":{"reference":reference,"document":document}}}});
    assert!(
        resolve_inline_reference(&runtime.ports.reader, session, &reference)
            .await
            .unwrap()
            .is_none()
    );
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value,
        },
    )
    .await
    .unwrap();
    let before = runtime
        .ports
        .reader
        .tool(session, tool.id())
        .await
        .unwrap()
        .revision;
    for _ in 0..2 {
        assert_eq!(
            resolve_inline_reference(&runtime.ports.reader, session, &reference)
                .await
                .unwrap(),
            Some(document.clone())
        );
    }
    assert_eq!(
        runtime
            .ports
            .reader
            .tool(session, tool.id())
            .await
            .unwrap()
            .revision,
        before,
        "reading must not mutate the prepared result"
    );
    assert!(
        resolve_inline_reference(&runtime.ports.reader, unrelated, &reference)
            .await
            .unwrap()
            .is_none()
    );
    let forged = reference.replace(
        "0123456789abcdef0123456789abcdef",
        "ffffffffffffffffffffffffffffffff",
    );
    assert!(
        resolve_inline_reference(&runtime.ports.reader, session, &forged)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        resolve_inline_reference(&runtime.ports.reader, session, "../escape")
            .await
            .unwrap()
            .is_none()
    );
    let body = format!("Before\n\n```inline-ui\n{reference}\n```\n\nAfter");
    let refs = resolve_inline_text(&runtime.ports.reader, session, &body)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refs.len(), 1);
    assert!(agena_domain::expand_inline_references(&body, &refs, false).contains("Original fact"));
    // Even a tool returning the same-shaped payload cannot impersonate prepare.
    let other = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "other".into(),
                name: "untrusted.tool".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let other_ref = format!("ui_{}_0123456789abcdef0123456789abcdef", other.id().get());
    other.complete(RequestId::new(),ToolResult {phase:ToolPhase::Succeeded,value:json!({"invocation":{"name":"untrusted.tool"},"raw_output":{"payload":{"inline_ui_prepared":{"reference":other_ref,"document":document}}}})}).await.unwrap();
    assert!(
        resolve_inline_reference(&runtime.ports.reader, session, &other_ref)
            .await
            .unwrap()
            .is_none()
    );
}
