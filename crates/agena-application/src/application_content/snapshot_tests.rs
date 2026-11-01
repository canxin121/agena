use super::*;
use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::*;

#[tokio::test]
async fn frozen_open_content_finishes_delivery_without_changing_capture_facts() {
    let runtime = agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
        .await
        .unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/frozen-delivery")
        .await
        .unwrap();
    let parent = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Parent".into())
        .await
        .unwrap()
        .value
        .id;
    let execution = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            parent,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Stream an answer".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), parent, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let source = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    source.append("before🌱").await.unwrap();
    let branch = runtime
        .ports
        .sessions
        .fork(RequestId::new(), parent, source.entry_id())
        .await
        .unwrap()
        .value
        .id;
    source.append("-after-the-fork").await.unwrap();
    let nested = runtime
        .ports
        .sessions
        .fork(RequestId::new(), branch, source.entry_id())
        .await
        .unwrap()
        .value
        .id;

    for session in [branch, nested] {
        let changes = runtime
            .ports
            .reader
            .watch_content(session, source.id())
            .await
            .unwrap();
        let head = changes.head().unwrap();
        assert_eq!(
            head.state,
            CaptureState::Open,
            "the historical capture fact stays open"
        );
        let watch = super::ContentWatch {
            inline_reply: true,
            resource: resource(session.get(), &head, changes.is_snapshot()),
            session,
            reader: runtime.ports.reader.clone(),
            changes,
        };
        let mut delivery = watch.delivery(None, 4);
        let mut text = String::new();
        let final_resource = loop {
            let page = delivery.next().await.unwrap().expect("a frozen page");
            for chunk in page.chunks {
                if let ContentPayload::Text { text: part } = chunk.payload {
                    text.push_str(&part);
                }
            }
            if !page.has_more {
                break page.resource;
            }
        };
        assert_eq!(text, "before🌱");
        let end = tokio::time::timeout(std::time::Duration::from_secs(1), delivery.next())
            .await
            .expect("a finite snapshot must not wait for another source update");
        assert!(
            matches!(end, Ok(None)),
            "finite delivery must end without an error: {end:?}"
        );
        assert_eq!(final_resource.state, ContentState::Complete);
        assert_eq!(
            runtime
                .ports
                .reader
                .text_range(session, source.id(), None, 64)
                .await
                .unwrap()
                .stream
                .state,
            CaptureState::Open
        );
    }

    let changes = runtime
        .ports
        .reader
        .watch_content(parent, source.id())
        .await
        .unwrap();
    let head = changes.head().unwrap();
    let watch = super::ContentWatch {
        inline_reply: true,
        resource: resource(parent.get(), &head, changes.is_snapshot()),
        session: parent,
        reader: runtime.ports.reader.clone(),
        changes,
    };
    let mut delivery = watch.delivery(None, 64);
    let page = delivery.next().await.unwrap().unwrap();
    assert_eq!(
        page.resource.state,
        ContentState::Active,
        "the live parent's source must remain active"
    );
    runtime
        .ports
        .sessions
        .cancel(RequestId::new(), parent, execution.id)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}
