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
    // Exercise the real live store through the exact authorization/read path
    // used by HTTP/SSE, with a private test root rather than the user's data.
    let directory = tempfile::tempdir().unwrap();
    let store = InlineUiStore::new(directory.path());
    store
        .create(session.get(), &reference, document.clone(), tool.id().get())
        .unwrap();
    let first = resolve_inline_snapshot(&runtime.ports.reader, session, &reference, store.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(first.phase, agena_domain::InlinePhase::Working);
    let changed = InlineDocument::from_value(json!({"version":1,"blocks":[{"type":"callout","id":"fact","text":"Updated after actual work"}]})).unwrap();
    let update = agena_domain::InlineUpdate {
        reference: reference.clone(),
        expected_revision: 1,
        document: Some(changed.clone()),
        patch: None,
        phase: Some(agena_domain::InlinePhase::Complete),
    };
    let mut events = store.subscribe(session.get(), &reference).unwrap();
    store
        .update(session.get(), &update, tool.id().get() + 1000)
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_millis(500), events.changed())
        .await
        .unwrap()
        .unwrap();
    let latest = resolve_inline_snapshot(&runtime.ports.reader, session, &reference, store.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.revision, 2);
    assert_eq!(latest.document, changed);
    assert_eq!(latest.phase, agena_domain::InlinePhase::Complete);
    let original = runtime.ports.reader.tool(session, tool.id()).await.unwrap();
    assert_eq!(
        original.revision, before,
        "live updates must not rewrite the original tool result"
    );
    assert_eq!(
        original
            .result
            .as_ref()
            .unwrap()
            .pointer("/raw_output/payload/inline_ui_prepared/document")
            .unwrap(),
        &serde_json::to_value(&document).unwrap()
    );
    // A record existing in another scope still does not grant transcript membership.
    store
        .create(
            unrelated.get(),
            &reference,
            document.clone(),
            tool.id().get(),
        )
        .unwrap();
    assert!(
        resolve_inline_snapshot(&runtime.ports.reader, unrelated, &reference, store.clone())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        resolve_inline_snapshot(&runtime.ports.reader, session, &forged, store)
            .await
            .unwrap()
            .is_none()
    );
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
