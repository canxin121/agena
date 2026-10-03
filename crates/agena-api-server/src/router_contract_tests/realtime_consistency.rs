use super::*;
use agena_api::live::SessionChangeResource;
use agena_client::SubscriptionEvent;
use agena_storage::store::{NewPart, PartDelta, PartRole, PartState, PartVisibility};
use std::time::Duration;

#[tokio::test]
async fn bounded_reconciliation_filters_memberships_visibility_and_validates_ids_over_http() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(&server.url).unwrap();
    let source = client
        .create_session(server.workspace_id, "source", None)
        .await
        .unwrap();
    let unrelated = client
        .create_session(server.workspace_id, "unrelated", None)
        .await
        .unwrap();
    let store = AppState::from_application(application_for_test(&server.runtime))
        .session_store()
        .unwrap();
    let input = |text: &str| NewPart {
        state: PartState::Completed,
        ..NewPart::pending("text", PartRole::User, serde_json::json!({"text": text}))
    };
    let first = store
        .submit_user_run(source.id, vec![input("first")], None)
        .await
        .unwrap();
    let second = store
        .submit_user_run(
            source.id,
            vec![
                input("second"),
                NewPart {
                    visibility: PartVisibility::Ai,
                    ..input("private")
                },
            ],
            None,
        )
        .await
        .unwrap();
    let hidden = second
        .parts
        .iter()
        .find(|part| part.visibility == PartVisibility::Ai)
        .unwrap();
    let ids = [
        second.run_id,
        first.run_id,
        first.run_id,
        hidden.part_id,
        i64::MAX,
    ];
    let page = client.session_parts_by_ids(source.id, &ids).await.unwrap();
    assert_eq!(page.parts.len(), 2);
    assert_eq!(page.parts[0].user_message_ordinal, Some(1));
    assert_eq!(page.parts[1].user_message_ordinal, Some(2));
    assert!(!page.page.has_more);
    assert!(
        client
            .session_parts_by_ids(unrelated.id, &ids)
            .await
            .unwrap()
            .parts
            .is_empty()
    );
    for query in [
        "ids=0".to_owned(),
        "ids=-1".into(),
        "ids=abc".into(),
        "ids=1&limit=2".into(),
        format!("ids={}", vec!["1"; 257].join(",")),
    ] {
        let response = reqwest::get(format!(
            "{}/api/v1/sessions/{}/parts?{query}",
            server.url, source.id
        ))
        .await
        .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
    store.delete(source.id).await.unwrap();
    for endpoint in ["parts?ids=1", "parts", "transcript", "state"] {
        let response = reqwest::get(format!(
            "{}/api/v1/sessions/{}/{endpoint}",
            server.url, source.id
        ))
        .await
        .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND,
            "{endpoint}"
        );
    }
}

#[tokio::test]
async fn real_stream_delivers_buffered_text_and_empty_session_deletion_in_workspace_scope() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(&server.url).unwrap();
    let source = client
        .create_session(server.workspace_id, "source", None)
        .await
        .unwrap();
    let empty = client
        .create_session(server.workspace_id, "empty", None)
        .await
        .unwrap();
    let store = AppState::from_application(application_for_test(&server.runtime))
        .session_store()
        .unwrap();
    let run = store
        .submit_user_run(
            source.id,
            vec![NewPart {
                state: PartState::InProgress,
                ..NewPart::pending("text", PartRole::Assistant, serde_json::json!({"text": ""}))
            }],
            None,
        )
        .await
        .unwrap();
    let part = run.parts.iter().find(|part| part.kind == "text").unwrap();
    let before = store
        .load_part_ids(source.id, &[])
        .await
        .unwrap()
        .meta
        .version;
    let mut stream = client
        .stream_changes(agena_api::Scope::Workspace {
            workspace_id: server.workspace_id,
        })
        .await
        .unwrap();
    for _ in 0..10 {
        store
            .update_part(
                source.id,
                part.part_id,
                PartDelta {
                    content_text_delta: Some("x".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    let observed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Ok(SubscriptionEvent::SessionChanged(SessionChangeResource::PartUpdated {
                part: updated,
                ..
            }))) = stream.recv().await
                && updated.part_id == part.part_id
                && updated.content["text"] == "xxxxxxxxxx"
            {
                break updated;
            }
        }
    })
    .await
    .expect("buffered text must arrive before completion");
    assert_eq!(observed.revision, part.revision);
    assert!(observed.updated_at_ms > part.updated_at_ms);
    assert_eq!(
        store
            .load_part_ids(source.id, &[])
            .await
            .unwrap()
            .meta
            .version,
        before,
        "no per-token durable writes"
    );
    store.delete(empty.id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Ok(SubscriptionEvent::SessionChanged(
                SessionChangeResource::SessionDeleted {
                    session_id,
                    workspace_id,
                },
            ))) = stream.recv().await
                && session_id == empty.id
            {
                assert_eq!(workspace_id, server.workspace_id);
                break;
            }
        }
    })
    .await
    .expect("empty deletion must survive workspace filtering");
}
